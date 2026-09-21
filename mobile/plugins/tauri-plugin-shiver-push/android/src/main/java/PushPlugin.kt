package com.shiver.push

import android.app.Activity
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Channel
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

/**
 * UnifiedPush, so a sleeping phone can be woken without Google.
 *
 * Shiver's notifications come from its own sockets, which only run while its process does. The usual
 * fix is FCM, which would make a self-hosted client depend on Google Play Services. UnifiedPush is
 * the alternative the self-hosting world settled on: a *distributor* app the user already runs
 * (ntfy, for one) holds a single connection on behalf of every app on the phone, and hands each one
 * an endpoint URL that its server can post to. The battery cost is paid once, by the distributor,
 * instead of once per app.
 *
 * The protocol is a handful of broadcasts and is implemented here directly rather than through the
 * connector library — it is small, it is stable, and Shiver would rather own a hundred lines than a
 * dependency it cannot audit. The action names below are the spec; if they ever stop matching, that
 * is where to look.
 *
 * **One registration per server.** The `token` is Shiver's rail entry id, so each server gets its own
 * endpoint and the endpoint that receives a push is itself the answer to "which server?". Nothing
 * has to travel in the payload, which is why the payload is empty.
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

/**
 * Whether Shiver's core is up in this process.
 *
 * The manifest receiver and the in-process one both hear every push. This is how they avoid saying
 * the same thing twice: a live process has the flag set and the manifest receiver stays quiet,
 * leaving the core to produce a precise notification. A push that arrives after Android has killed
 * Shiver starts a *fresh* process, where the flag is false because nothing has set it — which is
 * exactly the case the manifest receiver exists for. No IPC and no guessing.
 */
internal object PushState {
    @Volatile
    var running: Boolean = false
}

@InvokeArg
class RegisterArgs {
    lateinit var token: String
    /** the server's display name, kept so the cold-start receiver can say which server woke you */
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
    /** Where events go once Rust has asked for them. Null until then, and pushes are dropped. */
    private var events: Channel? = null

    /** Whether the receiver is currently registered, so teardown does not unregister twice. */
    private var registered = false

    private val prefs by lazy { activity.getSharedPreferences(PREFS, Context.MODE_PRIVATE) }

    /**
     * The warm half: Shiver is running, so a push can be turned into something precise.
     *
     * Registered in code rather than in the manifest, because it only makes sense while there is a
     * core to deliver to. It hands the event to Rust, which connects and works out what actually
     * arrived — the same path the unified inbox already uses.
     *
     * `PushReceiver` at the bottom of this file is the other half, for when Shiver is not running at
     * all. Both are registered, and Android delivers to both; the manifest one checks whether the
     * app is up before saying anything, so the user never gets two notifications for one message.
     */
    private val receiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            val token = intent.getStringExtra(EXTRA_TOKEN) ?: return

            // This receiver is exported, because a distributor is another app and Android will not
            // deliver to an unexported one. So the token is the guard: Shiver acts only on tokens it
            // issued itself, which are the rail entries' v4 uuids, and anything else is somebody
            // else's broadcast. Without this an app could hand Shiver an endpoint of its own choosing
            // and have a server post to it.
            if (!knows(token)) return

            // And the sender has to be the distributor the user actually chose. The token was the
            // whole barrier, and a uuid is a real one — but any app that learns a token, including
            // one the user tried as a distributor and switched away from, could otherwise hand
            // Shiver an endpoint of its own and have the server post to it on every message.
            if (!fromChosenDistributor(this, intent)) return

            when (intent.action) {
                ACTION_NEW_ENDPOINT -> {
                    val endpoint = intent.getStringExtra(EXTRA_ENDPOINT) ?: return
                    emit("endpoint", token, endpoint)
                }
                ACTION_REGISTRATION_FAILED -> emit("failed", token, null)
                ACTION_UNREGISTERED -> emit("unregistered", token, null)
                ACTION_MESSAGE -> {
                    // The payload is deliberately not read. Shiver's server half sends an empty body,
                    // and anything that did arrive would have travelled through a relay outside the
                    // server's origin — so it is not something to trust or to show.
                    emit("message", token, null)
                    acknowledge(intent, token)
                }
            }
        }
    }

    override fun load(webView: android.webkit.WebView) {
        super.load(webView)

        PushState.running = true

        val filter = IntentFilter().apply {
            addAction(ACTION_NEW_ENDPOINT)
            addAction(ACTION_REGISTRATION_FAILED)
            addAction(ACTION_UNREGISTERED)
            addAction(ACTION_MESSAGE)
        }

        // Exported, for the same reason the manifest receiver is: the distributor is a different
        // app, and from Android 13 a receiver registered as NOT_EXPORTED is not delivered another
        // app's broadcast at all — which is what left Shiver waiting for an endpoint that had already
        // been sent. The token check in `onReceive` is what makes that safe.
        if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.TIRAMISU) {
            activity.registerReceiver(receiver, filter, Context.RECEIVER_EXPORTED)
        } else {
            @Suppress("UnspecifiedRegisterReceiverFlag")
            activity.registerReceiver(receiver, filter)
        }

        registered = true
    }

    /**
     * Lets go of the receiver, and of the claim that a core is running.
     *
     * Neither used to happen. The receiver was registered against the Activity and never
     * unregistered, which Android reports as a leak; and `PushState.running` was set on load and
     * never cleared, so an Activity destroyed while the process stayed warm left it `true` with
     * nothing behind it. `PushReceiver` then stayed silent because the flag said Shiver was up,
     * and nothing else was listening — the user got no notification at all, which is the exact
     * failure the two halves exist to avoid.
     *
     * **Cleared on destroy, not on backgrounding.** A backgrounded Shiver still has its core and
     * its sockets, and is still the half that should be producing the precise notification; a flag
     * cleared when the user switched apps would give them two notifications for every message.
     */
    override fun onDestroy() {
        if (registered) {
            try {
                activity.unregisterReceiver(receiver)
            } catch (ex: IllegalArgumentException) {
                // already gone, which is not worth failing a teardown over
            }

            registered = false
        }

        PushState.running = false

        super.onDestroy()
    }

    /**
     * Whether this is a token Shiver handed out.
     *
     * Registration writes the server's name beside its token, so the presence of that entry is the
     * record of having asked. It is also what the cold-start receiver checks, for the same reason.
     */
    private fun knows(token: String) = prefs.contains("name:$token")

    /**
     * Whether this broadcast came from the distributor Shiver registered with.
     *
     * `sentFromPackage` is the direct answer and exists from API 34. Below that there is no way to
     * ask, so the token stays the only guard there — stated rather than silently assumed.
     */
    private fun fromChosenDistributor(receiver: BroadcastReceiver, intent: Intent): Boolean {
        val chosen = prefs.getString(KEY_DISTRIBUTOR, null) ?: return false

        if (android.os.Build.VERSION.SDK_INT < android.os.Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            return true
        }

        val sender = try {
            // a Java getter, so Kotlin sees a property — `sentFromPackage()` asks to invoke the
            // String it returns, which is what stopped this module compiling at all
            receiver.sentFromPackage
        } catch (ex: Exception) {
            // only valid while the broadcast is being delivered; if it is not available, fall back
            // to the token check alone rather than dropping a push the user is waiting for
            null
        }

        if (sender != null && sender != chosen) {
            android.util.Log.w("shiver", "ignoring a push broadcast from $sender")

            return false
        }

        return true
    }

    private fun emit(kind: String, token: String, endpoint: String?) {
        val payload = JSObject()

        payload.put("kind", kind)
        payload.put("token", token)
        if (endpoint != null) payload.put("endpoint", endpoint)

        events?.send(payload)
    }

    /** Tells the distributor the message arrived, so it stops holding on to it. */
    private fun acknowledge(intent: Intent, token: String) {
        val messageId = intent.getStringExtra(EXTRA_MESSAGE_ID) ?: return
        val distributor = prefs.getString(KEY_DISTRIBUTOR, null) ?: return

        val ack = Intent(ACTION_MESSAGE_ACK).apply {
            `package` = distributor
            putExtra(EXTRA_TOKEN, token)
            putExtra(EXTRA_MESSAGE_ID, messageId)
        }

        activity.sendBroadcast(ack)
    }

    /** Where the core says "send events here". Until this is called, pushes go nowhere. */
    @Command
    fun setEventHandler(invoke: Invoke) {
        val args = invoke.parseArgs(EventsArgs::class.java)

        events = args.handler

        invoke.resolve()
    }

    /**
     * Every app on the phone that can act as a distributor.
     *
     * Found by asking the package manager who listens for the register broadcast, which is how the
     * spec says to discover them. An empty list means the user has none installed, and Shiver should
     * say so rather than silently never being woken.
     *
     * **This only works because of the `<queries>` element in the manifest.** Since Android 11 an
     * app sees only the packages it has declared an interest in, and without that declaration this
     * returns an empty list on a phone with ntfy installed and running — which is exactly what it
     * did, and it looks identical to having no distributor at all.
     */
    @Command
    fun listDistributors(invoke: Invoke) {
        val intent = Intent(ACTION_REGISTER)
        val found = activity.packageManager
            .queryBroadcastReceivers(intent, 0)
            .mapNotNull { it.activityInfo?.packageName }
            .distinct()

        val result = JSObject()

        result.put("distributors", app.tauri.plugin.JSArray().apply { found.forEach { put(it) } })
        result.put("saved", prefs.getString(KEY_DISTRIBUTOR, null) ?: "")

        invoke.resolve(result)
    }

    /** Remembers which distributor to talk to. Registration is a separate step. */
    @Command
    fun setDistributor(invoke: Invoke) {
        val args = invoke.parseArgs(DistributorArgs::class.java)

        prefs.edit().putString(KEY_DISTRIBUTOR, args.distributor).apply()

        invoke.resolve()
    }

    /**
     * Asks the distributor for an endpoint for one rail entry.
     *
     * The answer does not come back here — the distributor replies with a `NEW_ENDPOINT` broadcast,
     * which arrives at the receiver above and is emitted on the channel. So this resolves as soon as
     * the request is sent, and the endpoint turns up shortly afterwards.
     */
    @Command
    fun register(invoke: Invoke) {
        val args = invoke.parseArgs(RegisterArgs::class.java)
        val distributor = prefs.getString(KEY_DISTRIBUTOR, null)

        if (distributor.isNullOrEmpty()) {
            invoke.reject("No UnifiedPush distributor has been chosen")

            return
        }

        // stored before the request, so a push that arrives before the endpoint round-trip
        // completes still has a name to show
        args.name?.let { prefs.edit().putString("name:${args.token}", it).apply() }

        val intent = Intent(ACTION_REGISTER).apply {
            `package` = distributor
            putExtra(EXTRA_TOKEN, args.token)
            putExtra(EXTRA_APPLICATION, activity.packageName)
        }

        activity.sendBroadcast(intent)

        invoke.resolve()
    }

    @Command
    fun unregister(invoke: Invoke) {
        val args = invoke.parseArgs(RegisterArgs::class.java)
        val distributor = prefs.getString(KEY_DISTRIBUTOR, null)

        if (!distributor.isNullOrEmpty()) {
            val intent = Intent(ACTION_UNREGISTER).apply {
                `package` = distributor
                putExtra(EXTRA_TOKEN, args.token)
                putExtra(EXTRA_APPLICATION, activity.packageName)
            }

            activity.sendBroadcast(intent)
        }

        prefs.edit().remove("name:${args.token}").apply()

        invoke.resolve()
    }
}

/**
 * The cold-start half: what happens when Shiver is not running at all.
 *
 * Declared in the manifest, so Android delivers to it even from a dead process — which is the entire
 * reason any of this exists. What it deliberately does **not** do is start Shiver: background activity
 * starts are blocked from Android 10, and a background foreground-service start from Android 12, so
 * anything built on waking the app would work on the developer's phone and fail on the user's.
 *
 * Instead it posts the notification itself, from what is already on this device. Shiver writes each
 * server's display name beside its token when it registers, so the receiver can say *which* server
 * wants attention without a network call, without the Rust core, and without the push carrying
 * anything. Opening it starts Shiver the ordinary way, and Shiver then says who actually said what.
 *
 * Less precise than the notifications Shiver posts while it is running, and that is the trade: this
 * one exists to get the user to look.
 */
class PushReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != ACTION_MESSAGE) return

        // Shiver is up in this process and will say something better than "new messages"
        if (PushState.running) return

        val token = intent.getStringExtra(EXTRA_TOKEN) ?: return
        val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

        // written by the core when it registered this entry; absent means Shiver no longer knows this
        // token, in which case there is nothing honest to say and nothing is shown
        val server = prefs.getString("name:$token", null) ?: return

        notify(context, token, server)
        acknowledge(context, intent, token, prefs.getString(KEY_DISTRIBUTOR, null))
    }

    private fun notify(context: Context, token: String, server: String) {
        val manager = context.getSystemService(Context.NOTIFICATION_SERVICE)
            as? android.app.NotificationManager ?: return

        // created once rather than on every push: after the first it is the user's to configure,
        // and re-creating it each time is work for an answer that cannot change
        if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.O &&
            manager.getNotificationChannel(CHANNEL_ID) == null
        ) {
            manager.createNotificationChannel(
                android.app.NotificationChannel(
                    CHANNEL_ID,
                    "Messages while Shiver is closed",
                    android.app.NotificationManager.IMPORTANCE_DEFAULT
                )
            )
        }

        val open = context.packageManager.getLaunchIntentForPackage(context.packageName)?.apply {
            addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        }

        val pending = open?.let {
            android.app.PendingIntent.getActivity(
                context,
                notificationId(token),
                it,
                android.app.PendingIntent.FLAG_UPDATE_CURRENT or
                    android.app.PendingIntent.FLAG_IMMUTABLE
            )
        }

        val builder = androidx.core.app.NotificationCompat.Builder(context, CHANNEL_ID)
            .setSmallIcon(android.R.drawable.stat_notify_chat)
            .setContentTitle(server)
            .setContentText("New messages")
            .setAutoCancel(true)
            .setContentIntent(pending)

        try {
            androidx.core.app.NotificationManagerCompat.from(context)
                .notify(notificationId(token), builder.build())
        } catch (error: SecurityException) {
            // notifications not granted; nothing to be done from here
        }
    }

    private fun acknowledge(context: Context, intent: Intent, token: String, distributor: String?) {
        val messageId = intent.getStringExtra(EXTRA_MESSAGE_ID) ?: return

        if (distributor.isNullOrEmpty()) return

        context.sendBroadcast(
            Intent(ACTION_MESSAGE_ACK).apply {
                `package` = distributor
                putExtra(EXTRA_TOKEN, token)
                putExtra(EXTRA_MESSAGE_ID, messageId)
            }
        )
    }

    private companion object {
        const val CHANNEL_ID = "shiver-push"
    }
}

/**
 * The notification id for one server, shared by both halves of push.
 *
 * **This number has to match the one Shiver's core computes**, because the two halves post and
 * clear the same server's notification independently: this receiver posts from a dead process, and
 * the running core takes it back when the user opens that server. It was `token.hashCode()` here
 * and a Rust `DefaultHasher` there — the same string through two different hash functions — so the
 * numbers disagreed, and a notification posted from a cold start could never be cleared. It sat on
 * the shade until the user swiped it, and the next warm notification stacked beside it instead of
 * replacing it.
 *
 * `String.hashCode` is the specified one of the two, so it is the one both sides implement: the
 * core calls `shiver_core::hash::java_string`, which is checked against a JVM's own answers. This
 * stays a named function rather than an inline `.hashCode()` so that the next person to touch
 * either side can see there is a second implementation to keep in step.
 *
 * The token is the rail entry's uuid, which is what makes it the right thing to key on.
 */
internal fun notificationId(token: String): Int = token.hashCode()
