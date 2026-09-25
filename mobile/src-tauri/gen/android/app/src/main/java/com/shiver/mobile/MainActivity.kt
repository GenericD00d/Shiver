package com.shiver.mobile

import android.app.Activity
import android.app.AlertDialog
import android.content.Context
import android.graphics.Bitmap
import android.graphics.Color
import android.graphics.Rect
import android.net.Uri
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.Base64
import android.view.MotionEvent
import android.view.PixelCopy
import android.view.View
import android.webkit.ConsoleMessage
import android.webkit.GeolocationPermissions
import android.webkit.JsPromptResult
import android.webkit.JsResult
import android.webkit.PermissionRequest
import android.webkit.ValueCallback
import android.webkit.WebChromeClient
import android.webkit.WebView
import androidx.activity.OnBackPressedCallback
import androidx.activity.SystemBarStyle
import androidx.activity.enableEdgeToEdge
import androidx.annotation.Keep
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import java.io.ByteArrayOutputStream
import kotlin.math.abs

class MainActivity : TauriActivity() {
  /** Shiver answers back presses itself (below) rather than walking the webview's history. */
  override val handleBackNavigation = false

  private var webView: WebView? = null

  /** A still of the server page last left, and that server's origin, until Shiver's page takes it. */
  private var still: Pair<String, Bitmap>? = null
  private var touchStart: Pair<Float, Float>? = null

  override fun onCreate(savedInstanceState: Bundle?) {
    // light status and navigation icons, over Shiver's dark background
    enableEdgeToEdge(
      statusBarStyle = SystemBarStyle.dark(Color.TRANSPARENT),
      navigationBarStyle = SystemBarStyle.dark(Color.TRANSPARENT)
    )
    super.onCreate(savedInstanceState)

    // Edge to edge is mandatory from Android 15, and Sharkord's fixed panels cannot be moved from
    // CSS, so the content view is padded by the system bars (and the keyboard, at the bottom) and
    // the insets consumed, leaving `env(safe-area-inset-*)` at zero inside the page.
    val content = findViewById<View>(android.R.id.content)

    ViewCompat.setOnApplyWindowInsetsListener(content) { view, insets ->
      val bars = insets.getInsets(WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout())
      val ime = insets.getInsets(WindowInsetsCompat.Type.ime())

      view.setPadding(bars.left, bars.top, bars.right, maxOf(bars.bottom, ime.bottom))

      WindowInsetsCompat.CONSUMED
    }
  }

  override fun onWebViewCreate(webView: WebView) {
    this.webView = webView
    gateMedia(webView)

    // Sharkord's default background until Rust applies the user's (Android paints white between documents)
    webView.setBackgroundColor(getColor(R.color.shiver_background))

    onBackPressedDispatcher.addCallback(
      this,
      object : OnBackPressedCallback(true) {
        /** when a server page last swallowed a press */
        private var swallowedAt = 0L

        override fun handleOnBackPressed() {
          val view = this@MainActivity.webView ?: return leave()
          val serverPage = !isShiverPage(view.url)

          if (serverPage) keepStill()

          // A server page answers the press (closing a menu, or leaving for Shiver's rail), but a
          // page could answer "true" forever, so a second press soon after a swallowed one always goes back.
          if (serverPage && SystemClock.uptimeMillis() - swallowedAt < ESCAPE_MS) {
            swallowedAt = 0
            return goBack(view)
          }

          view.evaluateJavascript("window.__SHIVER_BACK__ ? window.__SHIVER_BACK__() : false") { answer ->
            if (answer != "true") return@evaluateJavascript goBack(view)
            if (serverPage) swallowedAt = SystemClock.uptimeMillis()
          }
        }

        /**
         * Back through the webview's history, but never between Shiver's pages and a server's: a
         * history step passes no navigation guard, so a server would arrive without its session
         * (the core sends such a page home), and into Shiver's boot page it would only reopen the
         * server. Then whatever the system would do.
         */
        private fun goBack(view: WebView) {
          val history = view.copyBackForwardList()
          val previous = if (history.currentIndex > 0) history.getItemAtIndex(history.currentIndex - 1)?.url else null

          if (previous != null && isShiverPage(view.url) == isShiverPage(previous)) view.goBack() else leave()
        }

        /** Whatever the system would have done with the press. */
        private fun leave() {
          isEnabled = false
          onBackPressedDispatcher.onBackPressed()
          isEnabled = true
        }
      }
    )
  }

  /** A swipe right may be the one leaving a server page for the rail (the page decides), so it is photographed as it ends. */
  override fun dispatchTouchEvent(event: MotionEvent): Boolean {
    when (event.actionMasked) {
      MotionEvent.ACTION_DOWN -> touchStart = event.x to event.y
      MotionEvent.ACTION_UP -> touchStart?.let { (x, y) ->
        val dx = event.x - x

        if (dx >= SWIPE_DP * resources.displayMetrics.density && dx > abs(event.y - y)) keepStill()
      }
    }

    return super.dispatchTouchEvent(event)
  }

  /**
   * Copies what a server page shows, before the page can change, at a third of its size. It is
   * kept in memory only, for Shiver's page to show behind the rail (`takeStill`).
   */
  private fun keepStill() {
    val view = webView ?: return
    val origin = view.url?.takeUnless(::isShiverPage)?.let { originOf(Uri.parse(it)) } ?: return

    if (view.width < SCALE || view.height < SCALE) return

    val at = IntArray(2).also(view::getLocationInWindow)
    val area = Rect(at[0], at[1], at[0] + view.width, at[1] + view.height)
    val bitmap = Bitmap.createBitmap(view.width / SCALE, view.height / SCALE, Bitmap.Config.ARGB_8888)

    PixelCopy.request(window, area, bitmap, { result ->
      if (result == PixelCopy.SUCCESS) still = origin to bitmap
    }, Handler(Looper.getMainLooper()))
  }

  /** The still of `origin`'s page as a `data:` uri, given once; a still of any other page is dropped. Called from Rust. */
  @Keep
  fun takeStill(origin: String): String? {
    val (taken, bitmap) = still ?: return null

    still = null

    if (taken != origin) return null

    val jpeg = ByteArrayOutputStream().also { bitmap.compress(Bitmap.CompressFormat.JPEG, JPEG_QUALITY, it) }

    return "data:image/jpeg;base64," + Base64.encodeToString(jpeg.toByteArray(), Base64.NO_WRAP)
  }

  /** wry sets its chrome client after this hook, in the same turn of the UI thread; it is wrapped on the next. */
  private fun gateMedia(webView: WebView) {
    webView.post {
      val inner = webView.webChromeClient

      if (inner == null) gateMedia(webView) else webView.webChromeClient = MediaGate(this, webView, inner)
    }
  }

  private companion object {
    const val ESCAPE_MS = 1000L
    /** the bridge's own threshold for a swipe home, in dp (CSS px) */
    const val SWIPE_DP = 50
    const val SCALE = 3
    const val JPEG_QUALITY = 70
  }
}

/** Shiver's bundled origin, or (debug builds only) the vite dev server. */
private fun isShiverPage(url: String?): Boolean {
  val uri = url?.let(Uri::parse) ?: return false
  val host = uri.host ?: return false

  if (host == "tauri.localhost" && (uri.scheme == "http" || uri.scheme == "https")) return true

  return BuildConfig.DEBUG && uri.scheme == "http" && (host == "localhost" || host == "10.0.2.2")
}

/** `scheme://host[:port]`, the form Shiver stores origins in. */
private fun originOf(uri: Uri?): String? {
  val host = uri?.host?.lowercase() ?: return null

  return "${uri.scheme}://$host" + if (uri.port != -1) ":${uri.port}" else ""
}

/** Each server's camera and microphone answer, by origin; `SecretsPlugin.wipeOrigin` clears it. */
private const val MEDIA_PREFS = "shiver-media"

/**
 * wry's chrome client, except for permissions: wry grants a page whatever the app itself holds, so
 * once voice worked on one server every other could open the microphone unasked. Here the camera
 * and microphone go only to the server page on screen (not a frame in it), once the user has said
 * yes for that server, and nothing else is granted. The rest of what wry overrides is handed on.
 */
private class MediaGate(
  private val activity: Activity,
  private val webView: WebView,
  private val inner: WebChromeClient
) : WebChromeClient() {
  private val consent = activity.getSharedPreferences(MEDIA_PREFS, Context.MODE_PRIVATE)
  private var asking: Pair<PermissionRequest, AlertDialog>? = null

  override fun onPermissionRequest(request: PermissionRequest) {
    val wanted = request.resources
      .filter { it == PermissionRequest.RESOURCE_AUDIO_CAPTURE || it == PermissionRequest.RESOURCE_VIDEO_CAPTURE }
      .toTypedArray()
    val origin = originOf(request.origin)

    if (wanted.isEmpty() || origin == null || origin != originOf(webView.url?.let(Uri::parse)) || isShiverPage(webView.url)) {
      return request.deny()
    }

    val narrowed = Narrowed(request, wanted)

    if (consent.getBoolean(origin, false)) return inner.onPermissionRequest(narrowed)

    val what = when {
      wanted.size == 2 -> "your microphone and camera"
      wanted[0] == PermissionRequest.RESOURCE_AUDIO_CAPTURE -> "your microphone"
      else -> "your camera"
    }

    // one question at a time: an unanswered one is a no
    asking?.let { (earlier, dialog) ->
      dialog.dismiss()
      earlier.deny()
    }

    val answer = { allowed: Boolean ->
      asking = null

      if (allowed) {
        consent.edit().putBoolean(origin, true).apply()
        inner.onPermissionRequest(narrowed)
      } else {
        request.deny()
      }
    }

    val dialog = AlertDialog.Builder(activity)
      .setTitle(Uri.parse(origin).host)
      .setMessage("Let this server use $what? Shiver remembers a yes until you log out of the server or remove it.")
      .setPositiveButton("Allow") { _, _ -> answer(true) }
      .setNegativeButton("Don't allow") { _, _ -> answer(false) }
      .setOnCancelListener { answer(false) }
      .create()

    asking = request to dialog
    dialog.show()
  }

  override fun onPermissionRequestCanceled(request: PermissionRequest) {
    if (asking?.first != request) return

    asking?.second?.dismiss()
    asking = null
  }

  // no location permission is declared, so no page gets a location
  override fun onGeolocationPermissionsShowPrompt(origin: String?, callback: GeolocationPermissions.Callback?) {
    callback?.invoke(origin, false, false)
  }

  override fun onShowCustomView(view: View?, callback: CustomViewCallback?) = inner.onShowCustomView(view, callback)

  override fun onHideCustomView() = inner.onHideCustomView()

  override fun onJsAlert(view: WebView?, url: String?, message: String?, result: JsResult?) =
    inner.onJsAlert(view, url, message, result)

  override fun onJsConfirm(view: WebView?, url: String?, message: String?, result: JsResult?) =
    inner.onJsConfirm(view, url, message, result)

  override fun onJsPrompt(view: WebView?, url: String?, message: String?, defaultValue: String?, result: JsPromptResult?) =
    inner.onJsPrompt(view, url, message, defaultValue, result)

  override fun onShowFileChooser(
    webView: WebView?,
    filePathCallback: ValueCallback<Array<Uri>>?,
    fileChooserParams: FileChooserParams?
  ) = inner.onShowFileChooser(webView, filePathCallback, fileChooserParams)

  override fun onConsoleMessage(consoleMessage: ConsoleMessage?) = inner.onConsoleMessage(consoleMessage)

  override fun onReceivedTitle(view: WebView?, title: String?) = inner.onReceivedTitle(view, title)
}

/** A request cut down to the resources Shiver lets through (wry grants whatever it lists). */
private class Narrowed(private val request: PermissionRequest, private val allowed: Array<String>) : PermissionRequest() {
  override fun getOrigin(): Uri = request.origin

  override fun getResources(): Array<String> = allowed

  override fun grant(resources: Array<String>?) = request.grant(resources.orEmpty().filter(allowed::contains).toTypedArray())

  override fun deny() = request.deny()
}
