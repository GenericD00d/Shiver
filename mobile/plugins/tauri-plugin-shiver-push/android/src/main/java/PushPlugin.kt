package com.shiver.push

import android.app.Activity
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.SharedPreferences
import android.os.Build
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Channel
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

/*
 * UnifiedPush, implemented directly (it is a handful of broadcasts): a distributor app the user
 * already runs holds one connection and hands Shiver an endpoint per registration.
 *
 * One registration per rail entry, keyed by that entry's random push token (never the entry id),
 * so the endpoint that receives a push says which server it is about and the payload stays empty.
 * Both receivers are exported (a distributor is another app), so every broadcast is checked
 * against the tokens Shiver issued and, when the sender shares its identity (API 34+), against the
 * chosen distributor's package. The random token is the check that always applies.
 */

private const val ACTION_REGISTER = "org.unifiedpush.android.distributor.REGISTER"
private const val ACTION_UNREGISTER = "org.unifiedpush.android.distributor.UNREGISTER"
private const val ACTION_MESSAGE_ACK = "org.unifiedpush.android.distributor.MESSAGE_ACK"

private const val ACTION_NEW_ENDPOINT = "org.unifiedpush.android.connector.NEW_ENDPOINT"
private const val ACTION_REGISTRATION_FAILED = "org.unifiedpush.android.connector.REGISTRATION_FAILED"
private const val ACTION_UNREGISTERED = "org.unifiedpush.android.connector.UNREGISTERED"
private const val ACTION_MESSAGE = "org.unifiedpush.android.connector.MESSAGE"

private const val EXTRA_TOKEN = "token"
private const val EXTRA_ENDPOINT = "endpoint"
private const val EXTRA_APPLICATION = "application"
private const val EXTRA_MESSAGE_ID = "messageId"

private const val PREFS = "shiver-push"
private const val KEY_DISTRIBUTOR = "distributor"
private const val CHANNEL_ID = "shiver-push"

/** Whether the Rust core is receiving events in this process; the cold-start receiver stays quiet while it is. */
internal object PushState {
    @Volatile
    var running: Boolean = false
}

private fun pushPrefs(context: Context): SharedPreferences = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

private fun nameKey(token: String) = "name:$token"

/** The server name stored when `token` was registered; null for a token Shiver did not issue. */
private fun serverFor(prefs: SharedPreferences, token: String): String? = prefs.getString(nameKey(token), null)

/** False only when the sender is known (API 34+, and only if it shares its identity) and is not the chosen distributor. */
private fun fromChosenDistributor(receiver: BroadcastReceiver, prefs: SharedPreferences): Boolean {
    val chosen = prefs.getString(KEY_DISTRIBUTOR, null) ?: return false

    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.UPSIDE_DOWN_CAKE) return true

    val sender = try {
        receiver.sentFromPackage
    } catch (ex: Exception) {
        null
    }

    if (sender != null && sender != chosen) {
        android.util.Log.w("shiver", "ignoring a push broadcast from $sender")

        return false
    }

    return true
}

/** Tells the distributor a message was delivered. */
private fun acknowledge(context: Context, intent: Intent, token: String, prefs: SharedPreferences) {
    val messageId = intent.getStringExtra(EXTRA_MESSAGE_ID) ?: return
    val distributor = prefs.getString(KEY_DISTRIBUTOR, null)

    if (distributor.isNullOrEmpty()) return

    context.sendBroadcast(
        Intent(ACTION_MESSAGE_ACK).apply {
            `package` = distributor
            putExtra(EXTRA_TOKEN, token)
            putExtra(EXTRA_MESSAGE_ID, messageId)
        }
    )
}

/** Must equal the core's `shiver_core::hash::java_string(push_token)`, so either side can replace or clear the other's notification. */
internal fun notificationId(token: String): Int = token.hashCode()

@InvokeArg
class RegisterArgs {
    lateinit var token: String
    /** kept so the cold-start receiver can name the server */
    var name: String? = null
}

@InvokeArg
class DistributorArgs {
    lateinit var distributor: String
}

@InvokeArg
class EventsArgs {
    lateinit var handler: Channel
}

@TauriPlugin
class PushPlugin(private val activity: Activity) : Plugin(activity) {
    /** Rust's event channel; broadcasts are dropped until it is set. */
    private var events: Channel? = null
    private var registered = false
    private val prefs: SharedPreferences by lazy { pushPrefs(activity) }

    /** The warm receiver: hands events to the core, which fetches what arrived over its own connection. */
    private val receiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            val token = intent.getStringExtra(EXTRA_TOKEN) ?: return

            if (serverFor(prefs, token) == null || !fromChosenDistributor(this, prefs)) return

            when (intent.action) {
                ACTION_NEW_ENDPOINT -> emit("endpoint", token, intent.getStringExtra(EXTRA_ENDPOINT) ?: return)
                ACTION_REGISTRATION_FAILED -> emit("failed", token, null)
                ACTION_UNREGISTERED -> emit("unregistered", token, null)
                ACTION_MESSAGE -> {
                    // the payload is never read: it travelled through a relay outside the server's origin
                    emit("message", token, null)
                    acknowledge(activity, intent, token, prefs)
                }
            }
        }
    }

    override fun load(webView: android.webkit.WebView) {
        super.load(webView)

        val filter = IntentFilter().apply {
            addAction(ACTION_NEW_ENDPOINT)
            addAction(ACTION_REGISTRATION_FAILED)
            addAction(ACTION_UNREGISTERED)
            addAction(ACTION_MESSAGE)
        }

        // exported: from Android 13 a NOT_EXPORTED receiver never hears another app's broadcast
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            activity.registerReceiver(receiver, filter, Context.RECEIVER_EXPORTED)
        } else {
            @Suppress("UnspecifiedRegisterReceiverFlag")
            activity.registerReceiver(receiver, filter)
        }

        registered = true
    }

    /** Cleared on destroy, not on backgrounding: a backgrounded Shiver still has its core and sockets. */
    override fun onDestroy() {
        if (registered) {
            try {
                activity.unregisterReceiver(receiver)
            } catch (ex: IllegalArgumentException) {
                // already unregistered
            }

            registered = false
        }

        events = null
        PushState.running = false

        super.onDestroy()
    }

    private fun emit(kind: String, token: String, endpoint: String?) {
        val payload = JSObject()

        payload.put("kind", kind)
        payload.put("token", token)
        if (endpoint != null) payload.put("endpoint", endpoint)

        events?.send(payload)
    }

    @Command
    fun setEventHandler(invoke: Invoke) {
        events = invoke.parseArgs(EventsArgs::class.java).handler
        PushState.running = true

        invoke.resolve()
    }

    /** Apps answering the register broadcast (visible through the manifest's `<queries>`), and the chosen one. */
    @Command
    fun listDistributors(invoke: Invoke) {
        val found = activity.packageManager
            .queryBroadcastReceivers(Intent(ACTION_REGISTER), 0)
            .mapNotNull { it.activityInfo?.packageName }
            .distinct()

        val result = JSObject()

        result.put("distributors", JSArray().apply { found.forEach { put(it) } })
        result.put("saved", prefs.getString(KEY_DISTRIBUTOR, null) ?: "")

        invoke.resolve(result)
    }

    @Command
    fun setDistributor(invoke: Invoke) {
        val args = invoke.parseArgs(DistributorArgs::class.java)

        prefs.edit().putString(KEY_DISTRIBUTOR, args.distributor).apply()

        invoke.resolve()
    }

    /** Asks for an endpoint; it arrives later as a `NEW_ENDPOINT` broadcast. */
    @Command
    fun register(invoke: Invoke) {
        val args = invoke.parseArgs(RegisterArgs::class.java)
        val distributor = prefs.getString(KEY_DISTRIBUTOR, null)

        if (distributor.isNullOrEmpty()) {
            invoke.reject("No UnifiedPush distributor has been chosen")

            return
        }

        // before the request, so the token is known when the answer arrives
        prefs.edit().putString(nameKey(args.token), args.name ?: "Shiver").apply()

        activity.sendBroadcast(
            Intent(ACTION_REGISTER).apply {
                `package` = distributor
                putExtra(EXTRA_TOKEN, args.token)
                putExtra(EXTRA_APPLICATION, activity.packageName)
            }
        )

        invoke.resolve()
    }

    /** Safe for a token that was never registered. */
    @Command
    fun unregister(invoke: Invoke) {
        val args = invoke.parseArgs(RegisterArgs::class.java)
        val distributor = prefs.getString(KEY_DISTRIBUTOR, null)

        if (!distributor.isNullOrEmpty()) {
            activity.sendBroadcast(
                Intent(ACTION_UNREGISTER).apply {
                    `package` = distributor
                    putExtra(EXTRA_TOKEN, args.token)
                    putExtra(EXTRA_APPLICATION, activity.packageName)
                }
            )
        }

        prefs.edit().remove(nameKey(args.token)).apply()

        invoke.resolve()
    }
}

/**
 * The cold-start receiver (declared in the manifest, so it runs in a dead process). It does not
 * start Shiver (background starts are blocked); it posts "New messages" under the stored server
 * name, and opening it starts Shiver normally.
 */
class PushReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != ACTION_MESSAGE || PushState.running) return

        val token = intent.getStringExtra(EXTRA_TOKEN) ?: return
        val prefs = pushPrefs(context)
        val server = serverFor(prefs, token) ?: return

        if (!fromChosenDistributor(this, prefs)) return

        notify(context, token, server)
        acknowledge(context, intent, token, prefs)
    }

    private fun notify(context: Context, token: String, server: String) {
        val manager = context.getSystemService(Context.NOTIFICATION_SERVICE) as? android.app.NotificationManager ?: return

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O && manager.getNotificationChannel(CHANNEL_ID) == null) {
            manager.createNotificationChannel(
                android.app.NotificationChannel(
                    CHANNEL_ID,
                    "Messages while Shiver is closed",
                    android.app.NotificationManager.IMPORTANCE_DEFAULT
                )
            )
        }

        val pending = context.packageManager.getLaunchIntentForPackage(context.packageName)?.let {
            it.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)

            android.app.PendingIntent.getActivity(
                context,
                notificationId(token),
                it,
                android.app.PendingIntent.FLAG_UPDATE_CURRENT or android.app.PendingIntent.FLAG_IMMUTABLE
            )
        }

        val notification = androidx.core.app.NotificationCompat.Builder(context, CHANNEL_ID)
            .setSmallIcon(android.R.drawable.stat_notify_chat)
            .setContentTitle(server)
            .setContentText("New messages")
            .setAutoCancel(true)
            .setContentIntent(pending)
            .build()

        try {
            androidx.core.app.NotificationManagerCompat.from(context).notify(notificationId(token), notification)
        } catch (error: SecurityException) {
            // notifications not granted
        }
    }
}
