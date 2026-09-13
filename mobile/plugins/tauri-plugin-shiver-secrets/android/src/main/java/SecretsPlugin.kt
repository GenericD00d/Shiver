package com.shiver.secrets

import android.app.Activity
import android.content.SharedPreferences
import androidx.security.crypto.EncryptedSharedPreferences
import android.webkit.WebStorage
import android.webkit.WebView
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
 * Encrypted storage for Shiver session tokens.
 *
 * The master key is held by the Android Keystore, so it is hardware-backed on a device with a
 * secure element and never sits in the app own files. Keys are encrypted as well as values, so the
 * file does not even reveal which servers a session exists for.
 *
 * Every method answers, including on failure: a device whose Keystore refuses to produce a key
 * would otherwise leave the core waiting on a call that never returns.
 *
 * **Nothing here runs on the main thread.** Tauri delivers a plugin command on Android's UI thread
 * (`run_on_android_context`), and `EncryptedSharedPreferences` is Tink underneath — the first touch
 * asks the Keystore for a master key and parses a keyset out of protobuf. On a warm app that is
 * milliseconds; on the first launch after an install, with those classes still unverified and
 * interpreted, it was seconds, and an ANR trace caught the main thread in the middle of it. So each
 * command hands its work to one background thread and answers from there. The response path is a
 * map lookup on the Rust side and does not care which thread it arrives on.
 */
@TauriPlugin
class SecretsPlugin(private val activity: Activity) : Plugin(activity) {
    /**
     * One thread for all of it, rather than a pool.
     *
     * Serialising the work is the point: a write followed by a read has to happen in that order,
     * and there is never enough of this work to be worth doing two at a time. Daemon, so it can
     * never hold the process open.
     */
    private val worker = Executors.newSingleThreadExecutor { runnable ->
        Thread(runnable, "shiver-secrets").apply { isDaemon = true }
    }

    private val store: SharedPreferences by lazy {
        val master = MasterKey.Builder(activity)
            .setKeyScheme(MasterKey.KeyScheme.AES256_GCM)
            .build()

        EncryptedSharedPreferences.create(
            activity,
            "shiver-secrets",
            master,
            EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
            EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM
        )
    }

    /**
     * Opens the store before anything asks for it.
     *
     * The expensive part of this plugin is the first touch, and Shiver's first touch happens while
     * the app is starting and restoring sessions. Doing it here means it is already done — and if
     * a command does arrive first, `by lazy` is synchronised, so that command waits on the worker
     * thread rather than on anybody's frame.
     */
    override fun load(webView: WebView) {
        worker.execute {
            try {
                store
            } catch (ex: Exception) {
                // A Keystore that will not produce a key is a real failure, but not one to take the
                // app down over: every command answers, so Shiver will hear about it as a rejection
                // and carry on without stored sessions.
                android.util.Log.w("shiver", "could not open the secret store: ${ex.message}")
            }
        }
    }

    /** Runs one command's work off the main thread, and answers however it goes. */
    private fun offMainThread(invoke: Invoke, work: () -> JSObject?) {
        worker.execute {
            try {
                val result = work()

                if (result == null) invoke.resolve() else invoke.resolve(result)
            } catch (ex: Exception) {
                invoke.reject(ex.message)
            }
        }
    }

    @Command
    fun setSecret(invoke: Invoke) = offMainThread(invoke) {
        val args = invoke.parseArgs(SetArgs::class.java)

        store.edit().putString(args.key, args.value).apply()

        null
    }

    @Command
    fun getSecret(invoke: Invoke) = offMainThread(invoke) {
        val args = invoke.parseArgs(KeyArgs::class.java)
        val result = JSObject()

        // empty for absent, which the Rust side reads as None
        result.put("value", store.getString(args.key, "") ?: "")

        result
    }

    @Command
    fun removeSecret(invoke: Invoke) = offMainThread(invoke) {
        val args = invoke.parseArgs(KeyArgs::class.java)

        store.edit().remove(args.key).apply()

        null
    }

    /**
     * Deletes everything a server's page kept in web storage.
     *
     * The point is the bytes, not the values. Removing a key from `localStorage` writes a deletion
     * into LevelDB and leaves the old record in the log until it happens to be compacted, so a
     * session cleared that way can still be read out of the app's files. This drops the origin's
     * storage outright.
     *
     * The one command that stays on the main thread, because WebStorage is not safe to call from
     * anywhere else. It touches no encrypted storage, so there is nothing slow in it, and it is
     * fire-and-forget: it is called once the page for that origin is already being replaced.
     */
    @Command
    fun wipeOrigin(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(OriginArgs::class.java)

            activity.runOnUiThread {
                WebStorage.getInstance().deleteOrigin(args.origin)
            }

            invoke.resolve()
        } catch (ex: Exception) {
            invoke.reject(ex.message)
        }
    }

    @Command
    fun secretKeys(invoke: Invoke) = offMainThread(invoke) {
        val result = JSObject()

        result.put("keys", JSONArray(store.all.keys.toList()))

        result
    }
}
