package com.shiver.secrets

import android.app.Activity
import android.content.Context
import android.content.SharedPreferences
import android.webkit.CookieManager
import android.webkit.WebStorage
import android.webkit.WebView
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.util.concurrent.Executors
import org.json.JSONArray

@InvokeArg
class KeyArgs {
    lateinit var key: String
}

@InvokeArg
class SetArgs {
    lateinit var key: String
    lateinit var value: String
}

@InvokeArg
class OriginArgs {
    lateinit var origin: String
}

/**
 * Encrypted storage for Shiver's sessions, passwords and unread floors:
 * `EncryptedSharedPreferences` (AES-256-SIV keys, AES-256-GCM values) under an Android Keystore
 * master key. (`security-crypto` is deprecated without a successor; replacing it only touches this
 * file.)
 *
 * Commands arrive on the UI thread, and the first Tink/Keystore touch can take seconds, so all work
 * runs on one serial daemon thread (preserving write-then-read order) and every command answers,
 * even on failure.
 */
@TauriPlugin
class SecretsPlugin(private val activity: Activity) : Plugin(activity) {
    private val worker = Executors.newSingleThreadExecutor { runnable ->
        Thread(runnable, "shiver-secrets").apply { isDaemon = true }
    }

    private val store: SharedPreferences by lazy {
        val master = MasterKey.Builder(activity).setKeyScheme(MasterKey.KeyScheme.AES256_GCM).build()

        EncryptedSharedPreferences.create(
            activity,
            "shiver-secrets",
            master,
            EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
            EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM
        )
    }

    /** Opens the store early, off the main thread; a Keystore failure surfaces later as rejections. */
    override fun load(webView: WebView) {
        worker.execute {
            try {
                store
            } catch (ex: Exception) {
                android.util.Log.w("shiver", "could not open the secret store: ${ex.message}")
            }
        }
    }

    private fun offMainThread(invoke: Invoke, work: () -> JSObject?) {
        worker.execute {
            try {
                val result = work()

                if (result == null) invoke.resolve() else invoke.resolve(result)
            } catch (ex: Exception) {
                invoke.reject(ex.message ?: "The secret store failed")
            }
        }
    }

    /** Writes synchronously (a lost write would leave a credential on disk the app reports gone). */
    private fun commit(change: SharedPreferences.Editor.() -> Unit) {
        val editor = store.edit()

        editor.change()

        if (!editor.commit()) throw IllegalStateException("Could not write to the secret store")
    }

    @Command
    fun setSecret(invoke: Invoke) = offMainThread(invoke) {
        val args = invoke.parseArgs(SetArgs::class.java)

        commit { putString(args.key, args.value) }

        null
    }

    /** Empty for absent, which the Rust side reads as None. */
    @Command
    fun getSecret(invoke: Invoke) = offMainThread(invoke) {
        val args = invoke.parseArgs(KeyArgs::class.java)

        JSObject().apply { put("value", store.getString(args.key, "") ?: "") }
    }

    @Command
    fun removeSecret(invoke: Invoke) = offMainThread(invoke) {
        val args = invoke.parseArgs(KeyArgs::class.java)

        commit { remove(args.key) }

        null
    }

    @Command
    fun secretKeys(invoke: Invoke) = offMainThread(invoke) {
        JSObject().apply { put("keys", JSONArray(store.all.keys.toList())) }
    }

    /**
     * Drops an origin's Web Storage and IndexedDB (removing keys from a page leaves old records in
     * LevelDB until compaction), expires the cookies that origin can see, and forgets the camera and
     * microphone answer `MainActivity` keeps for it. Best effort: the HTTP cache and cookies scoped
     * to other paths or a parent domain are not covered. Must run on the UI thread; fire-and-forget,
     * since the page is already being replaced.
     */
    @Command
    fun wipeOrigin(invoke: Invoke) {
        try {
            val origin = invoke.parseArgs(OriginArgs::class.java).origin

            activity.runOnUiThread {
                WebStorage.getInstance().deleteOrigin(origin)

                val cookies = CookieManager.getInstance()

                cookies.getCookie(origin)?.split(';')?.forEach { pair ->
                    val name = pair.substringBefore('=').trim()

                    if (name.isNotEmpty()) cookies.setCookie(origin, "$name=; Max-Age=0; Path=/")
                }

                cookies.flush()
                activity.getSharedPreferences("shiver-media", Context.MODE_PRIVATE).edit().remove(origin).apply()
            }

            invoke.resolve()
        } catch (ex: Exception) {
            invoke.reject(ex.message ?: "Could not clear the page's storage")
        }
    }
}
