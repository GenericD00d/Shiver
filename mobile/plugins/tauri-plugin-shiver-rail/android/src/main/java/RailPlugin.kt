package com.shiver.rail

import android.annotation.SuppressLint
import android.app.Activity
import android.graphics.Color
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.view.View
import android.view.ViewGroup
import android.view.animation.DecelerateInterpolator
import android.webkit.RenderProcessGoneDetail
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebView
import android.widget.FrameLayout
import androidx.activity.ComponentActivity
import androidx.activity.OnBackPressedCallback
import androidx.appcompat.app.AppCompatActivity
import androidx.webkit.JavaScriptReplyProxy
import androidx.webkit.WebMessageCompat
import androidx.webkit.WebViewAssetLoader
import androidx.webkit.WebViewClientCompat
import androidx.webkit.WebViewCompat
import androidx.webkit.WebViewFeature
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Channel
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import org.json.JSONException
import org.json.JSONObject
import java.io.ByteArrayInputStream

/*
 * Host for the quick rail: the shared rail's page (mobile/rail, in this plugin's assets) in a WebView
 * of its own over the server page, never inside it. It loads only that page and talks only to this
 * plugin, over one channel offered to its origin. Here: the overlay, scrim, slide-in, back press and
 * the channel; the core sends what to draw (`show`, `refresh`) and hears what was chosen.
 */

/** the rail's width, as on Shiver's own page (`--shiver-rail-width`) */
private const val RAIL_DP = 72
private const val SLIDE_MS = 180L
/** what shows of the page behind: dimmed */
private const val SCRIM = 0x73000000
/** the overlay while the page draws: too faint to see, but drawn (at 0 Android skips it) */
private const val PREPARING_ALPHA = 0.01f
/** how long a show waits for the page to draw before it gives up (the core then uses Shiver's page) */
private const val DRAW_TIMEOUT_MS = 2000L

private const val ORIGIN = "https://appassets.androidplatform.net"
private const val PAGE_PATH = "/assets/rail/"
private const val PAGE = "$ORIGIN${PAGE_PATH}index.html"
/** the page's end of the channel: `window.shiverRail` */
private const val CHANNEL = "shiverRail"
/** what the user can choose, closing the rail; anything else the page says is dropped */
private val CHOICES = setOf("dms", "add", "settings", "closed")
/** an entry or folder id: a uuid, bounded */
private val ID = Regex("^[A-Za-z0-9-]{1,64}$")

@InvokeArg
class EventsArgs {
    lateinit var handler: Channel
}

@TauriPlugin
class RailPlugin(private val activity: Activity) : Plugin(activity) {
    private var events: Channel? = null
    private val main = Handler(Looper.getMainLooper())

    private var overlay: FrameLayout? = null
    private var rail: WebView? = null
    private var back: OnBackPressedCallback? = null

    /** the page's end of the channel, once it has said it is ready */
    private var page: JavaScriptReplyProxy? = null
    /** the latest view, held until the page is ready for it */
    private var queued: String? = null
    private var sequence = 0
    /** a show waiting for its view to be drawn: its sequence number, the call, and the logos lacked */
    private var revealing: Triple<Int, Invoke, JSArray>? = null
    /** the logo key the page has been given for each entry */
    private val given = HashMap<String, String>()

    /** Made at start-up rather than on the first swipe, so the page is loaded before it is wanted. */
    override fun load(webView: WebView) {
        super.load(webView)

        activity.runOnUiThread { overlay() }
    }

    override fun onDestroy(activity: AppCompatActivity) {
        rail?.destroy()
        rail = null
        overlay = null
        page = null
        given.clear()
    }

    @Command
    fun setEventHandler(invoke: Invoke) {
        events = invoke.parseArgs(EventsArgs::class.java).handler
        invoke.resolve()
    }

    @Command
    fun show(invoke: Invoke) = draw(invoke, reveal = true)

    @Command
    fun refresh(invoke: Invoke) = draw(invoke, reveal = false)

    @Command
    fun hide(invoke: Invoke) {
        activity.runOnUiThread {
            dismiss()
            invoke.resolve()
        }
    }

    /**
     * Hands `invoke`'s view to the page. A rail already up is redrawn at once; a show waits until the
     * page has drawn it, then slides in, so it never shows a blank or stale rail.
     */
    private fun draw(invoke: Invoke, reveal: Boolean) {
        val view = invoke.getArgs()

        activity.runOnUiThread {
            val up = overlay?.visibility == View.VISIBLE

            if (!reveal && !up) return@runOnUiThread invoke.resolve(drawn(JSArray()))

            val root = overlay() ?: return@runOnUiThread invoke.reject("The rail could not be drawn")
            val missing = JSArray()
            val number = ++sequence

            takeLogos(view, missing)
            send(JSONObject().put("seq", number).put("view", view).toString())

            if (!reveal || (up && revealing == null)) return@runOnUiThread invoke.resolve(drawn(missing))

            // a show still waiting is answered as drawn: rejecting it would send the core to Shiver's page
            revealing?.let { it.second.resolve(drawn(it.third)) }
            revealing = Triple(number, invoke, missing)
            prepare(root)

            main.postDelayed({
                val waiting = revealing

                if (waiting != null && waiting.first == number) {
                    revealing = null
                    dismiss()
                    waiting.second.reject("The rail did not draw in time")
                }
            }, DRAW_TIMEOUT_MS)
        }
    }

    private fun drawn(missing: JSArray) = JSObject().apply { put("missing", missing) }

    /** Notes the logos this view gives the page, and the ones it names that the page has never been given. */
    private fun takeLogos(view: JSONObject, missing: JSArray) {
        val servers = view.optJSONArray("servers") ?: return

        for (index in 0 until servers.length()) {
            val server = servers.optJSONObject(index) ?: continue
            val id = server.optString("id")
            val key = server.optString("iconKey").takeIf { it.isNotEmpty() && !server.isNull("iconKey") }
            val icon = !server.isNull("icon") && server.optString("icon").isNotEmpty()

            when {
                key == null -> given.remove(id)
                icon -> given[id] = key
                given[id] != key -> missing.put(id)
            }
        }
    }

    private fun send(message: String) {
        val proxy = page

        if (proxy == null) queued = message else proxy.postMessage(message)
    }

    private fun dp(value: Int): Int = (value * activity.resources.displayMetrics.density).toInt()

    /** The overlay and its WebView, made once and kept (again, if the content view was replaced). */
    private fun overlay(): FrameLayout? {
        val content = activity.findViewById<ViewGroup>(android.R.id.content) ?: return null
        val existing = overlay

        if (existing != null) {
            if (existing.parent == null) content.addView(existing)
            if (rail == null) existing.addView(railView() ?: return null)

            return existing
        }

        val web = railView() ?: return null
        val root = FrameLayout(activity).apply {
            layoutParams = FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT)
            setBackgroundColor(SCRIM)
            visibility = View.GONE
            // a tap beside the rail goes back to the page, and no touch reaches the page meanwhile
            isClickable = true
            setOnClickListener { choose("closed", null) }
        }

        root.addView(web)
        content.addView(root)
        overlay = root

        return root
    }

    /** The rail's WebView: its own page only, from the assets, with the network shut off. */
    @SuppressLint("SetJavaScriptEnabled")
    private fun railView(): WebView? {
        // the channel is the only way to the page; a WebView too old for it cannot host the rail
        if (!WebViewFeature.isFeatureSupported(WebViewFeature.WEB_MESSAGE_LISTENER)) return null

        val assets = WebViewAssetLoader.AssetsPathHandler(activity)
        val loader = WebViewAssetLoader.Builder()
            .addPathHandler(PAGE_PATH) { path -> assets.handle("rail/$path") }
            .build()

        val web = WebView(activity).apply {
            layoutParams = FrameLayout.LayoutParams(dp(RAIL_DP), ViewGroup.LayoutParams.MATCH_PARENT)
            setBackgroundColor(Color.TRANSPARENT)
            isVerticalScrollBarEnabled = false
            overScrollMode = View.OVER_SCROLL_NEVER

            settings.apply {
                javaScriptEnabled = true
                blockNetworkLoads = true
                allowFileAccess = false
                allowContentAccess = false
                domStorageEnabled = false
                javaScriptCanOpenWindowsAutomatically = false
                setSupportMultipleWindows(false)
                setGeolocationEnabled(false)
                setSupportZoom(false)
            }

            webViewClient = object : WebViewClientCompat() {
                override fun shouldInterceptRequest(view: WebView, request: WebResourceRequest): WebResourceResponse =
                    loader.shouldInterceptRequest(request.url)
                        ?: WebResourceResponse("text/plain", "utf-8", 404, "Not Found", emptyMap(), ByteArrayInputStream(ByteArray(0)))

                // the rail's page, and nowhere else
                override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest) =
                    request.url.toString() != PAGE

                override fun onRenderProcessGone(view: WebView, detail: RenderProcessGoneDetail): Boolean {
                    forget(view)

                    return true
                }
            }
        }

        WebViewCompat.addWebMessageListener(web, CHANNEL, setOf(ORIGIN)) { _, message, origin, isMainFrame, reply ->
            if (isMainFrame && origin == Uri.parse(ORIGIN)) heard(message, reply)
        }

        web.loadUrl(PAGE)
        rail = web

        return web
    }

    /** The WebView's renderer is gone: a new one, and a fresh page, next time the rail is wanted. */
    private fun forget(web: WebView) {
        dismiss()
        overlay?.removeView(web)
        web.destroy()
        rail = null
        page = null
        given.clear()
    }

    /** What the page said: that it is ready, that a view is drawn, or what the user chose. */
    private fun heard(message: WebMessageCompat, reply: JavaScriptReplyProxy) {
        val said = try {
            JSONObject(message.data ?: return)
        } catch (ex: JSONException) {
            return
        }

        when (val kind = said.optString("kind")) {
            "ready" -> {
                page = reply
                queued?.let { reply.postMessage(it) }
                queued = null
            }
            "drawn" -> {
                val waiting = revealing ?: return

                // a later view (a refresh meanwhile) being drawn means this one has been too
                if (said.optInt("seq") < waiting.first) return

                revealing = null
                slideIn()
                waiting.second.resolve(drawn(waiting.third))
            }
            else -> if (showing()) chosen(kind, said)
        }
    }

    /** The rail is up and seen: only then can the user have chosen anything on it. */
    private fun showing() = overlay?.visibility == View.VISIBLE && revealing == null

    private fun chosen(kind: String, said: JSONObject) {
        when (kind) {
            "open" -> said.optString("entryId").takeIf { ID.matches(it) }?.let { choose("open", it) }
            "folder" -> {
                val folderId = said.optString("folderId").takeIf { ID.matches(it) } ?: return
                val expanded = said.opt("expanded") as? Boolean ?: return

                // the rail stays up; the core stores it and redraws
                events?.send(JSObject().apply {
                    put("kind", "folder")
                    put("folderId", folderId)
                    put("expanded", expanded)
                })
            }
            in CHOICES -> choose(kind, null)
        }
    }

    /**
     * Up but not yet seen, so the page draws before it slides in: a WebView that is hidden, or whose
     * overlay is fully transparent, is not drawn at all, and neither is its page.
     */
    private fun prepare(root: FrameLayout) {
        root.animate().cancel()
        rail?.animate()?.cancel()
        root.alpha = PREPARING_ALPHA
        root.isClickable = false
        root.visibility = View.VISIBLE
        rail?.translationX = 0f
    }

    private fun slideIn() {
        val root = overlay ?: return
        val web = rail ?: return

        root.isClickable = true
        web.translationX = -dp(RAIL_DP).toFloat()
        root.animate().alpha(1f).setDuration(SLIDE_MS).start()
        web.animate().translationX(0f).setDuration(SLIDE_MS).setInterpolator(DecelerateInterpolator()).start()

        // added after the activity's own, so it is asked first while the rail is up
        if (back == null) {
            (activity as? ComponentActivity)?.let { owner ->
                val callback = object : OnBackPressedCallback(false) {
                    override fun handleOnBackPressed() = choose("closed", null)
                }

                owner.onBackPressedDispatcher.addCallback(callback)
                back = callback
            }
        }

        back?.isEnabled = true
    }

    private fun dismiss() {
        overlay?.animate()?.cancel()
        rail?.animate()?.cancel()
        overlay?.visibility = View.GONE
        back?.isEnabled = false
    }

    /**
     * Closes the rail and tells the core what was chosen: `closed`, `open` with an entry id, or one of
     * Shiver's own screens (`dms`, `add`, `settings`).
     */
    private fun choose(kind: String, entryId: String?) {
        dismiss()

        events?.send(JSObject().apply {
            put("kind", kind)
            if (entryId != null) put("entryId", entryId)
        })
    }
}
