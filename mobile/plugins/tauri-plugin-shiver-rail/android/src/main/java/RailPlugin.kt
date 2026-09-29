package com.shiver.rail

import android.app.Activity
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.util.Base64
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.view.animation.DecelerateInterpolator
import android.widget.FrameLayout
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import androidx.activity.ComponentActivity
import androidx.activity.OnBackPressedCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Channel
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import org.json.JSONArray
import org.json.JSONObject

/*
 * Shiver's quick rail, drawn with Android's own views over the server page on screen, which keeps
 * running (and connected) behind it. Views, not a page: the server's page shares a script world
 * with anything drawn inside it, and must never see what else is on the rail.
 *
 * The core hands over what to draw (`show`, `refresh`) and hears what the user chose through the
 * event channel: another server, Shiver's own page, or back to the page behind. Commands arrive on
 * the core's thread and are drawn on the UI thread before they answer.
 */

/** the rail's width, as on Shiver's own page (`--shiver-rail-width`) */
private const val RAIL_DP = 68
private const val TILE_DP = 48
private const val FOLDER_TILE_DP = 40
private const val GAP_DP = 8
private const val BADGE_DP = 18
/** a logo is decoded no larger than a tile needs at the densest screens */
private const val LOGO_PX = 160
private const val SLIDE_MS = 180L
/** what shows of the page behind: dimmed, as Shiver's page dimmed its picture of it */
private const val SCRIM = 0x73000000

@InvokeArg
class EventsArgs {
    lateinit var handler: Channel
}

@TauriPlugin
class RailPlugin(private val activity: Activity) : Plugin(activity) {
    private var events: Channel? = null

    /** each entry's logo, decoded once, under the key the core named it by */
    private val logos = HashMap<String, Pair<String, Bitmap?>>()

    private var overlay: FrameLayout? = null
    private var panel: ScrollView? = null
    private var back: OnBackPressedCallback? = null
    /** the server whose page is behind the rail */
    private var current: String? = null

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

    /** Draws `invoke`'s view (sliding the rail in when `reveal`), answering with the logos it lacked. */
    private fun draw(invoke: Invoke, reveal: Boolean) {
        val view = invoke.getArgs()

        activity.runOnUiThread {
            try {
                val shown = overlay?.visibility == View.VISIBLE
                val missing = JSArray()

                if (reveal || shown) {
                    render(view, missing)

                    if (!shown) slideIn()
                }

                invoke.resolve(JSObject().apply { put("missing", missing) })
            } catch (ex: Exception) {
                invoke.reject(ex.message ?: "The rail could not be drawn")
            }
        }
    }

    private fun dp(value: Int): Int =
        TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_DIP, value.toFloat(), activity.resources.displayMetrics).toInt()

    /** The overlay, attached over the webview (again, if the content view was replaced). */
    private fun overlay(): FrameLayout {
        val content = activity.findViewById<ViewGroup>(android.R.id.content)
        val existing = overlay

        if (existing != null) {
            if (existing.parent == null) content.addView(existing)

            return existing
        }

        val root = FrameLayout(activity).apply {
            layoutParams = FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT)
            setBackgroundColor(SCRIM)
            visibility = View.GONE
            // a tap beside the rail goes back to the page, and no touch reaches the page meanwhile
            isClickable = true
            setOnClickListener { choose("closed", null) }
        }

        val scroll = ScrollView(activity).apply {
            layoutParams = FrameLayout.LayoutParams(dp(RAIL_DP), ViewGroup.LayoutParams.MATCH_PARENT)
            isVerticalScrollBarEnabled = false
            // a tap on the rail between tiles is not a tap beside it
            isClickable = true
        }

        root.addView(scroll)
        content.addView(root)

        // added after the activity's own, so it is asked first while the rail is up
        (activity as? ComponentActivity)?.let { owner ->
            val callback = object : OnBackPressedCallback(false) {
                override fun handleOnBackPressed() = choose("closed", null)
            }

            owner.onBackPressedDispatcher.addCallback(callback)
            back = callback
        }

        overlay = root
        panel = scroll

        return root
    }

    private fun render(view: JSONObject, missing: JSArray) {
        overlay()

        val scroll = panel ?: return
        val palette = Palette(view)

        current = view.optString("current").takeIf { it.isNotEmpty() && !view.isNull("current") }
        scroll.setBackgroundColor(palette.rail)

        val column = LinearLayout(activity).apply {
            orientation = LinearLayout.VERTICAL
            gravity = Gravity.CENTER_HORIZONTAL
            setPadding(0, dp(10) - dp(GAP_DP), 0, dp(10))
        }

        column.addView(homeTile(palette))
        column.addView(View(activity).apply {
            setBackgroundColor(palette.divider)
            layoutParams = LinearLayout.LayoutParams(dp(32), 1).apply { topMargin = dp(GAP_DP) }
        })

        val rows = view.optJSONArray("rows") ?: JSONArray()

        for (index in 0 until rows.length()) {
            val row = rows.optJSONObject(index) ?: continue

            when (row.optString("kind")) {
                "server" -> row.optJSONObject("server")?.let { column.addView(tile(it, TILE_DP, palette, missing)) }
                "folder" -> column.addView(folder(row, palette, missing))
            }
        }

        scroll.removeAllViews()
        scroll.addView(column, ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT))
    }

    /** Shiver's own page: messages, settings and servers to add. */
    private fun homeTile(palette: Palette): View {
        val frame = FrameLayout(activity).apply {
            layoutParams = LinearLayout.LayoutParams(dp(TILE_DP), dp(TILE_DP)).apply { topMargin = dp(GAP_DP) }
            contentDescription = "Shiver: messages, settings and servers"
            isClickable = true
            isFocusable = true
            setOnClickListener { choose("home", null) }
        }

        val face = ImageView(activity).apply {
            setImageDrawable(activity.packageManager.getApplicationIcon(activity.applicationInfo))
            scaleType = ImageView.ScaleType.CENTER_CROP
            background = rounded(palette.surface, dp(16).toFloat())
            clipToOutline = true
        }

        frame.addView(face, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))

        return frame
    }

    /** A folder: its servers, grouped on the dimmer surface, always open (this rail is for going). */
    private fun folder(row: JSONObject, palette: Palette, missing: JSArray): View {
        val group = LinearLayout(activity).apply {
            orientation = LinearLayout.VERTICAL
            gravity = Gravity.CENTER_HORIZONTAL
            background = rounded(palette.surfaceDim, dp(16).toFloat())
            setPadding(dp(4), dp(4), dp(4), dp(4))
            layoutParams = LinearLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply {
                topMargin = dp(GAP_DP)
            }
            contentDescription = row.optString("name")
        }

        val servers = row.optJSONArray("servers") ?: JSONArray()

        for (index in 0 until servers.length()) {
            servers.optJSONObject(index)?.let { group.addView(tile(it, FOLDER_TILE_DP, palette, missing, first = index == 0)) }
        }

        return group
    }

    private fun tile(server: JSONObject, size: Int, palette: Palette, missing: JSArray, first: Boolean = false): View {
        val id = server.optString("id")
        val name = server.optString("name")
        val unread = server.optInt("unread", 0)
        val radius = dp(if (size == TILE_DP) 16 else 13).toFloat()

        val frame = FrameLayout(activity).apply {
            layoutParams = LinearLayout.LayoutParams(dp(size), dp(size)).apply { topMargin = if (first) 0 else dp(GAP_DP) }
            contentDescription = if (unread > 0) "$name, $unread unread" else name
            isClickable = true
            isFocusable = true
            setOnClickListener { choose(if (id == current) "closed" else "open", id) }
        }

        val logo = logo(id, server, missing)
        val face: View = if (logo != null) {
            ImageView(activity).apply {
                setImageBitmap(logo)
                scaleType = ImageView.ScaleType.CENTER_CROP
            }
        } else {
            TextView(activity).apply {
                text = server.optString("initials", "?")
                setTextColor(palette.text)
                setTextSize(TypedValue.COMPLEX_UNIT_SP, if (size == TILE_DP) 16f else 14f)
                typeface = Typeface.DEFAULT_BOLD
                gravity = Gravity.CENTER
            }
        }

        face.background = rounded(palette.surface, radius)
        face.clipToOutline = true
        frame.addView(face, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))

        // the server whose page is behind: ringed in the accent, as on Shiver's own rail
        if (id == current) {
            frame.addView(
                View(activity).apply {
                    background = GradientDrawable().apply {
                        cornerRadius = radius
                        setColor(Color.TRANSPARENT)
                        setStroke(dp(2), palette.accent)
                    }
                },
                FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT)
            )
        }

        if (unread > 0) {
            frame.addView(
                TextView(activity).apply {
                    text = if (unread > 99) "99+" else unread.toString()
                    setTextColor(palette.rail)
                    setTextSize(TypedValue.COMPLEX_UNIT_SP, 9f)
                    typeface = Typeface.DEFAULT_BOLD
                    gravity = Gravity.CENTER
                    background = GradientDrawable().apply {
                        shape = GradientDrawable.OVAL
                        setColor(palette.text)
                        setStroke(1, palette.rail)
                    }
                    importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
                },
                FrameLayout.LayoutParams(dp(BADGE_DP), dp(BADGE_DP), Gravity.TOP or Gravity.END)
            )
        }

        return frame
    }

    /** The entry's logo: decoded from what came with this view, or kept from an earlier one; a key never seen is reported. */
    private fun logo(id: String, server: JSONObject, missing: JSArray): Bitmap? {
        val key = server.optString("iconKey").takeIf { it.isNotEmpty() && !server.isNull("iconKey") } ?: return null
        val data = server.optString("icon").takeIf { it.isNotEmpty() && !server.isNull("icon") }

        logos[id]?.let { (known, bitmap) -> if (known == key && data == null) return bitmap }

        if (data == null) {
            missing.put(id)

            return null
        }

        val bitmap = decode(data)

        logos[id] = key to bitmap

        return bitmap
    }

    /** A `data:` uri's image, scaled down on decoding; null for what Android cannot draw (SVG). */
    private fun decode(uri: String): Bitmap? = try {
        val bytes = Base64.decode(uri.substringAfter(','), Base64.DEFAULT)
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }

        BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)

        var sample = 1

        while (bounds.outWidth / (sample * 2) >= LOGO_PX && bounds.outHeight / (sample * 2) >= LOGO_PX) sample *= 2

        BitmapFactory.decodeByteArray(bytes, 0, bytes.size, BitmapFactory.Options().apply { inSampleSize = sample })
    } catch (ex: IllegalArgumentException) {
        null
    }

    private fun rounded(color: Int, radius: Float) = GradientDrawable().apply {
        cornerRadius = radius
        setColor(color)
    }

    private fun slideIn() {
        val root = overlay ?: return
        val scroll = panel ?: return

        root.visibility = View.VISIBLE
        root.alpha = 0f
        root.animate().alpha(1f).setDuration(SLIDE_MS).start()
        scroll.translationX = -dp(RAIL_DP).toFloat()
        scroll.animate().translationX(0f).setDuration(SLIDE_MS).setInterpolator(DecelerateInterpolator()).start()
        back?.isEnabled = true
    }

    private fun dismiss() {
        overlay?.animate()?.cancel()
        overlay?.visibility = View.GONE
        back?.isEnabled = false
    }

    /** Closes the rail and tells the core what was chosen: `closed`, `home`, or `open` with an entry id. */
    private fun choose(kind: String, entryId: String?) {
        dismiss()

        events?.send(JSObject().apply {
            put("kind", kind)
            if (entryId != null) put("entryId", entryId)
        })
    }

    /** The core's `#rrggbb` colours; anything unreadable falls back to Shiver's defaults. */
    private class Palette(view: JSONObject) {
        val rail = color(view, "rail", 0xFF171717.toInt())
        val surface = color(view, "surface", 0xFF313131.toInt())
        val surfaceDim = color(view, "surfaceDim", 0xFF1f1f1f.toInt())
        val text = color(view, "text", 0xFFFAFAFA.toInt())
        val accent = color(view, "accent", 0xFFE5E5E5.toInt())
        /** `--shiver-border`: the text at a tenth */
        val divider = (text and 0x00FFFFFF) or 0x1A000000

        private companion object {
            fun color(view: JSONObject, name: String, fallback: Int): Int = try {
                Color.parseColor(view.optString(name))
            } catch (ex: RuntimeException) {
                // unparseable, or empty (which throws out of bounds rather than illegal argument)
                fallback
            }
        }
    }
}
