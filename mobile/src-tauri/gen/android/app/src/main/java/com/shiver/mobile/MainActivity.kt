package com.shiver.mobile

import android.graphics.Color
import android.net.Uri
import android.os.Bundle
import android.os.SystemClock
import android.view.View
import android.webkit.WebView
import androidx.activity.OnBackPressedCallback
import androidx.activity.SystemBarStyle
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  /** Shiver answers back presses itself (below) rather than walking the webview's history. */
  override val handleBackNavigation = false

  private var webView: WebView? = null

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

        /** Shiver's bundled origin, or (debug builds only) the vite dev server. */
        private fun isShiverPage(url: String?): Boolean {
          val uri = url?.let(Uri::parse) ?: return false
          val host = uri.host ?: return false

          if (host == "tauri.localhost" && (uri.scheme == "http" || uri.scheme == "https")) return true

          return BuildConfig.DEBUG && uri.scheme == "http" && (host == "localhost" || host == "10.0.2.2")
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

  private companion object {
    const val ESCAPE_MS = 1000L
  }
}
