package com.shiver.mobile

import android.graphics.Color
import android.net.Uri
import android.os.Bundle
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
        override fun handleOnBackPressed() {
          val view = this@MainActivity.webView ?: return leave()

          // Only Shiver's own pages are asked: a server's page could define `__SHIVER_BACK__` and
          // swallow every press.
          if (!isShiverPage(view.url)) {
            if (view.canGoBack()) view.goBack() else leave()

            return
          }

          // the rail closes its innermost open thing and answers whether it used the press
          view.evaluateJavascript("window.__SHIVER_BACK__ ? window.__SHIVER_BACK__() : false") { answer ->
            if (answer == "true") return@evaluateJavascript

            if (view.canGoBack()) view.goBack() else leave()
          }
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
}
