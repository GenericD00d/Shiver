package com.shiver.mobile

import android.graphics.Color
import android.os.Bundle
import android.view.View
import android.webkit.WebView
import androidx.activity.OnBackPressedCallback
import androidx.activity.SystemBarStyle
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  /**
   * Wry's own back handling is not installed.
   *
   * Left on, it walks the webview's history and then finishes the activity — which on a client that
   * navigates within itself means back lands somewhere arbitrary rather than anywhere the user
   * asked for. Shiver answers the press itself, below.
   */
  override val handleBackNavigation = false

  private var webView: WebView? = null

  override fun onCreate(savedInstanceState: Bundle?) {
    // Both bars are told they sit on something dark, which is what decides the colour of the
    // clock and the battery. Without it the system picks from the day/night setting and draws
    // them dark — invisible against the background behind them.
    enableEdgeToEdge(
      statusBarStyle = SystemBarStyle.dark(Color.TRANSPARENT),
      navigationBarStyle = SystemBarStyle.dark(Color.TRANSPARENT)
    )
    super.onCreate(savedInstanceState)

    // Keep the webview out from under the status and navigation bars.
    //
    // Edge to edge is not optional from Android 15 on, so the window genuinely does extend behind
    // both bars, and Sharkord's own header ended up sharing the strip with the clock — its server
    // name written over the time. That cannot be fixed from css: the panels involved are
    // `position: fixed`, which anchors them to the viewport rather than to anything a stylesheet of
    // Shiver's can push down, and it is not Shiver's place to re-lay-out the client anyway.
    //
    // Padding the content view moves the viewport itself instead, so every one of those panels
    // lands below the bar without the page being told anything. The insets are consumed rather than
    // passed on, so `env(safe-area-inset-*)` inside the page correctly reports nothing left to
    // avoid — the rail's own safe-area padding then adds zero rather than a second inset.
    //
    // The bottom takes whichever is larger of the navigation bar and the keyboard, so the composer
    // clears the gesture pill and is not buried when the keyboard opens.
    val content = findViewById<View>(android.R.id.content)

    ViewCompat.setOnApplyWindowInsetsListener(content) { view, insets ->
      val bars =
        insets.getInsets(
          WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout()
        )
      val ime = insets.getInsets(WindowInsetsCompat.Type.ime())

      view.setPadding(bars.left, bars.top, bars.right, maxOf(bars.bottom, ime.bottom))

      WindowInsetsCompat.CONSUMED
    }
  }

  /**
   * Points the back button at the server rail.
   *
   * Registered here rather than in `onCreate` because this runs once the webview exists, which is
   * also the only moment there is anything to ask. The rail answers innermost thing first — a menu,
   * then the direct-message list, then the rail itself — and says whether it used the press.
   *
   * On a page Shiver is not drawing on, its own screens included, nothing answers and the press falls
   * through to what the system would have done.
   */
  override fun onWebViewCreate(webView: WebView) {
    this.webView = webView

    // Kills the white flash between servers, from the first frame.
    //
    // Android's WebView paints white when no page is painting, and on mobile every server switch is
    // two navigations — the rail inside a server's page has no way to call Shiver, so it goes to
    // Shiver's own page with a fragment and Shiver navigates on from there. Each of those tears the old
    // document down before the new one paints, and the webview's own background is what shows in
    // between.
    //
    // Sharkord's own colour, which is Shiver's default, rather than the user's chosen one: this runs
    // before Rust has read the settings. The Rust side sets the user's actual background straight
    // after (`webview::background_color`) and again whenever they change it, so this is the floor
    // rather than the answer — but it is the floor that covers app start.
    webView.setBackgroundColor(getColor(R.color.shiver_background))

    onBackPressedDispatcher.addCallback(
      this,
      object : OnBackPressedCallback(true) {
        override fun handleOnBackPressed() {
          val view = this@MainActivity.webView

          if (view == null) {
            leave()

            return
          }

          // asynchronous, and deliberately so: the reply arrives on this thread rather than being
          // waited for on it, which is the difference between a back press and a frozen app
          view.evaluateJavascript(
            "window.__SHIVER_BACK__ ? window.__SHIVER_BACK__() : false"
          ) { answer ->
            if (answer == "true") return@evaluateJavascript

            if (view.canGoBack()) view.goBack() else leave()
          }
        }

        /** What the system would have done with the press, had Shiver not taken it. */
        private fun leave() {
          isEnabled = false
          onBackPressedDispatcher.onBackPressed()
          isEnabled = true
        }
      }
    )
  }
}
