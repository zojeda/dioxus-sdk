package dev.dioxus.push

import android.Manifest
import android.app.Activity
import android.app.Application
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import androidx.core.app.ActivityCompat
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import com.google.firebase.messaging.FirebaseMessaging
import org.json.JSONObject
import java.lang.ref.WeakReference
import java.util.UUID
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger

/**
 * Bridge between Firebase Cloud Messaging and the Rust `dioxus-sdk-push` crate.
 *
 * ### JNI contract (keep in sync with `src/platform/android.rs`)
 *
 * Rust -> Kotlin (called via `env.call_static_method` on the Android UI thread):
 * - `register(Landroid/content/Context;)V`
 * - `requestNotificationPermission(Landroid/content/Context;)V`
 *
 * Kotlin -> Rust (`external` members below, `#[unsafe(no_mangle)]` exports in Rust):
 * - `Java_dev_dioxus_push_DioxusPush_nativeOnToken(String)`
 * - `Java_dev_dioxus_push_DioxusPush_nativeOnTokenError(String)`
 * - `Java_dev_dioxus_push_DioxusPush_nativeOnMessage(String?, String?, String, String?)`
 * - `Java_dev_dioxus_push_DioxusPush_nativeOnTap(String?, String?, String, String?)`
 * - `Java_dev_dioxus_push_DioxusPush_nativeOnPermission(Boolean)`
 *
 * ### App integration
 *
 * The app's `MainActivity` must forward three lifecycle hooks (see the crate README):
 * - `onCreate` -> [onActivityCreated] (tap detection + foreground tracking)
 * - `onNewIntent` -> [onNewIntent] (taps while the app is already running)
 * - `onRequestPermissionsResult` -> [onRequestPermissionsResult] (permission results)
 *
 * ### Cold-start safety
 *
 * FCM can spawn the app process without any activity, so the Rust library may not
 * be loaded when a callback fires. Every `native*` call is therefore guarded: we
 * try `System.loadLibrary` (default `"main"`, overridable via [setNativeLibraryName]
 * or the `dev.dioxus.push.lib` manifest meta-data) and buffer events on
 * `UnsatisfiedLinkError`. The buffer is flushed inside [register], which Rust
 * invokes once the app (and thus the native library) is up.
 */
object DioxusPush {
    /** Runtime-permission request code; must fit in 16 bits for AppCompatActivity. */
    private const val PERMISSION_REQUEST_CODE = 0x5058 // "PX"

    /** Notification channel used for tray notifications posted by this module. */
    const val CHANNEL_ID = "dioxus_push"

    /** Manifest `<meta-data>` key overriding the native library name. */
    private const val META_NATIVE_LIB = "dev.dioxus.push.lib"

    // Extras attached to the launch PendingIntent of locally-posted notifications.
    internal const val EXTRA_TAP = "dev.dioxus.push.tap"
    private const val EXTRA_TAP_ID = "dev.dioxus.push.tap_id"
    private const val EXTRA_TAP_TITLE = "dev.dioxus.push.title"
    private const val EXTRA_TAP_BODY = "dev.dioxus.push.body"
    private const val EXTRA_TAP_DATA = "dev.dioxus.push.data"
    private const val EXTRA_TAP_MESSAGE_ID = "dev.dioxus.push.message_id"

    private const val MAX_PENDING_EVENTS = 256
    private const val MAX_DELIVERED_TAP_IDS = 64

    // ---- Kotlin -> Rust (implemented in platform/android.rs) --------------------

    external fun nativeOnToken(token: String)
    external fun nativeOnTokenError(error: String)
    external fun nativeOnMessage(title: String?, body: String?, dataJson: String, messageId: String?)
    external fun nativeOnTap(title: String?, body: String?, dataJson: String, messageId: String?)
    external fun nativeOnPermission(granted: Boolean)

    // ---- Native-library guard + pending-event queue ------------------------------

    private val lock = Any()
    private val pendingEvents = ArrayDeque<() -> Unit>()
    private var nativeReady = false
    private var nativeLibraryName = "main"
    private var metaDataChecked = false

    /**
     * Override the name of the native library carrying the Rust JNI exports.
     * Defaults to `"main"` (the library `dx` builds). Alternatively declare
     * `<meta-data android:name="dev.dioxus.push.lib" android:value="mylib" />`
     * inside `<application>`.
     */
    @JvmStatic
    fun setNativeLibraryName(name: String) {
        synchronized(lock) { nativeLibraryName = name }
    }

    /** Read the optional native-library-name override from manifest meta-data. */
    internal fun configureFromContext(context: Context) {
        synchronized(lock) {
            if (metaDataChecked) return
            metaDataChecked = true
            try {
                val info = context.packageManager
                    .getApplicationInfo(context.packageName, PackageManager.GET_META_DATA)
                info.metaData?.getString(META_NATIVE_LIB)?.let { nativeLibraryName = it }
            } catch (_: Exception) {
                // Keep the default; worst case events are buffered until register().
            }
        }
    }

    /**
     * Run [call] (an invocation of one of the `external fun`s above) if the native
     * library is available, otherwise buffer it until [register] flushes the queue.
     */
    private fun dispatch(call: () -> Unit) {
        synchronized(lock) {
            if (!nativeReady) {
                nativeReady = tryLoadNativeLibrary()
            }
            if (!nativeReady) {
                buffer(call)
                return
            }
        }
        runNative(call)
    }

    /** Must be called with [lock] held. */
    private fun buffer(call: () -> Unit) {
        pendingEvents.addLast(call)
        while (pendingEvents.size > MAX_PENDING_EVENTS) {
            pendingEvents.removeFirst()
        }
    }

    private fun runNative(call: () -> Unit) {
        try {
            call()
        } catch (_: UnsatisfiedLinkError) {
            // Library loaded but symbols missing (e.g. wrong library name):
            // fall back to buffering until register() proves Rust is up.
            synchronized(lock) {
                nativeReady = false
                buffer(call)
            }
        }
    }

    private fun tryLoadNativeLibrary(): Boolean {
        return try {
            System.loadLibrary(nativeLibraryName)
            true
        } catch (_: UnsatisfiedLinkError) {
            false
        }
    }

    /**
     * Mark the Rust side reachable (we were just called *from* it over JNI) and
     * flush any events buffered while the process ran without the native library.
     */
    private fun markNativeReadyAndFlush() {
        val toFlush: List<() -> Unit>
        synchronized(lock) {
            nativeReady = true
            toFlush = pendingEvents.toList()
            pendingEvents.clear()
        }
        toFlush.forEach { runNative(it) }
    }

    // ---- Guarded emit helpers (safe to call from any thread) --------------------

    internal fun emitToken(token: String) = dispatch { nativeOnToken(token) }

    internal fun emitTokenError(error: String) = dispatch { nativeOnTokenError(error) }

    internal fun emitMessage(payload: PushPayload) = dispatch {
        nativeOnMessage(payload.title, payload.body, toJson(payload.data), payload.messageId)
    }

    internal fun emitTap(title: String?, body: String?, dataJson: String, messageId: String?) =
        dispatch { nativeOnTap(title, body, dataJson, messageId) }

    internal fun emitPermission(granted: Boolean) {
        val callback = synchronized(lock) {
            val cb = authorizationCallback
            authorizationCallback = null
            cb
        }
        callback?.invoke(granted)
        dispatch { nativeOnPermission(granted) }
    }

    // ---- Rust -> Kotlin entry points (called over JNI on the UI thread) ---------

    /**
     * Begin FCM registration; the token is reported through [nativeOnToken] /
     * [nativeOnTokenError]. Called from Rust with the current activity — the
     * `(Landroid/content/Context;)V` signature is part of the JNI contract.
     *
     * Also flushes any events buffered before the native library was loaded.
     */
    @JvmStatic
    fun register(context: Context) {
        configureFromContext(context)
        trackApplication(context.applicationContext as? Application)
        markNativeReadyAndFlush()
        FirebaseMessaging.getInstance().token.addOnCompleteListener { task ->
            if (task.isSuccessful) {
                emitToken(task.result)
            } else {
                emitTokenError(task.exception?.message ?: "getToken failed")
            }
        }
    }

    /**
     * Request the Android 13+ `POST_NOTIFICATIONS` runtime permission. The result
     * arrives via [nativeOnPermission] — either immediately, or after the user
     * responds (requires the activity to forward [onRequestPermissionsResult]).
     */
    @JvmStatic
    fun requestNotificationPermission(context: Context) {
        configureFromContext(context)
        markNativeReadyAndFlush()
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
            emitPermission(true)
            return
        }
        val granted = ContextCompat.checkSelfPermission(
            context,
            Manifest.permission.POST_NOTIFICATIONS,
        ) == PackageManager.PERMISSION_GRANTED
        if (granted) {
            emitPermission(true)
            return
        }
        val activity = context as? Activity ?: currentActivity?.get()
        if (activity != null) {
            ActivityCompat.requestPermissions(
                activity,
                arrayOf(Manifest.permission.POST_NOTIFICATIONS),
                PERMISSION_REQUEST_CODE,
            )
            // Result delivered via onRequestPermissionsResult (forwarded by the app).
        } else {
            emitPermission(false)
        }
    }

    // ---- App-forwarded lifecycle hooks -------------------------------------------

    /**
     * Forward from `Activity.onRequestPermissionsResult`. Requests with a foreign
     * request code are ignored, so it is always safe to call unconditionally.
     */
    @JvmStatic
    fun onRequestPermissionsResult(requestCode: Int, grantResults: IntArray) {
        if (requestCode != PERMISSION_REQUEST_CODE) return
        val granted =
            grantResults.isNotEmpty() && grantResults[0] == PackageManager.PERMISSION_GRANTED
        emitPermission(granted)
    }

    /**
     * Forward from `Activity.onCreate` (after `super.onCreate`) with the activity
     * and its launch intent. Detects notification-tap launches — both taps on
     * FCM tray notifications posted by the system (background notification
     * messages) and taps on notifications posted locally by this module — and
     * installs the foreground tracker.
     */
    @JvmStatic
    fun onActivityCreated(activity: Activity, intent: Intent?) {
        configureFromContext(activity)
        currentActivity = WeakReference(activity)
        trackApplication(activity.application)
        handleTapIntent(intent)
    }

    /**
     * Forward from `Activity.onNewIntent` (call `setIntent(intent)` too, so a later
     * recreation does not re-process the previous intent).
     */
    @JvmStatic
    fun onNewIntent(intent: Intent?) {
        handleTapIntent(intent)
    }

    // ---- Notification-tap handling ------------------------------------------------

    /** Recently delivered tap ids, so one tap is never reported twice. */
    private val deliveredTapIds = ArrayDeque<String>()

    /** Returns `true` the first time [id] is seen, `false` on repeats. */
    private fun markTapDelivered(id: String): Boolean {
        synchronized(deliveredTapIds) {
            if (deliveredTapIds.contains(id)) return false
            deliveredTapIds.addLast(id)
            while (deliveredTapIds.size > MAX_DELIVERED_TAP_IDS) {
                deliveredTapIds.removeFirst()
            }
            return true
        }
    }

    private fun handleTapIntent(intent: Intent?) {
        val extras = intent?.extras ?: return

        // Case 1: tap on a notification posted locally by postNotification().
        if (extras.getBoolean(EXTRA_TAP, false)) {
            val tapId = extras.getString(EXTRA_TAP_ID)
            if (tapId != null && !markTapDelivered(tapId)) return
            emitTap(
                extras.getString(EXTRA_TAP_TITLE),
                extras.getString(EXTRA_TAP_BODY),
                extras.getString(EXTRA_TAP_DATA) ?: "{}",
                extras.getString(EXTRA_TAP_MESSAGE_ID),
            )
            return
        }

        // Case 2: tap on an FCM tray notification (background "notification"
        // messages are displayed by the system; tapping relaunches the launcher
        // activity with google.* bookkeeping extras plus the data payload).
        val messageId = extras.getString("google.message_id")
        val looksLikeFcmLaunch = messageId != null || extras.containsKey("google.sent_time")
        if (!looksLikeFcmLaunch) return
        if (messageId != null && !markTapDelivered(messageId)) return

        val data = mutableMapOf<String, String>()
        for (key in extras.keySet()) {
            if (key.startsWith("google.") || key.startsWith("gcm.")) continue
            if (key == "collapse_key" || key == "from") continue
            val value = extras.getString(key) ?: continue
            data[key] = value
        }
        // The tray title/body are not included in the relaunch extras; only the
        // data payload survives. Rust surfaces this as a data-only tap.
        emitTap(null, null, toJson(data), messageId)
    }

    // ---- Foreground tracking --------------------------------------------------------

    private val trackerInstalled = AtomicBoolean(false)
    private val visibleActivities = AtomicInteger(0)
    private var currentActivity: WeakReference<Activity>? = null

    /** `true` while any activity of the app is started (visible). */
    internal fun isAppInForeground(): Boolean = visibleActivities.get() > 0

    internal fun trackApplication(application: Application?) {
        application ?: return
        if (!trackerInstalled.compareAndSet(false, true)) return
        application.registerActivityLifecycleCallbacks(object :
            Application.ActivityLifecycleCallbacks {
            override fun onActivityStarted(activity: Activity) {
                visibleActivities.incrementAndGet()
                currentActivity = WeakReference(activity)
            }

            override fun onActivityStopped(activity: Activity) {
                visibleActivities.decrementAndGet()
            }

            override fun onActivityCreated(activity: Activity, savedInstanceState: Bundle?) {}
            override fun onActivityResumed(activity: Activity) {}
            override fun onActivityPaused(activity: Activity) {}
            override fun onActivitySaveInstanceState(activity: Activity, outState: Bundle) {}
            override fun onActivityDestroyed(activity: Activity) {}
        })
    }

    // ---- Local notification display ---------------------------------------------------

    /**
     * Emit [payload] to Rust and, when it carries displayable content while no
     * activity of the app is in the foreground, post a tray notification whose
     * tap relaunches the app with the payload attached (delivered back through
     * [onActivityCreated] / [onNewIntent] as a tap event).
     */
    internal fun handleRemoteMessage(context: Context, payload: PushPayload) {
        emitMessage(payload)
        val displayable = payload.title != null || payload.body != null
        if (displayable && !isAppInForeground()) {
            postNotification(context, payload)
        }
    }

    internal fun postNotification(context: Context, payload: PushPayload) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            ContextCompat.checkSelfPermission(
                context,
                Manifest.permission.POST_NOTIFICATIONS,
            ) != PackageManager.PERMISSION_GRANTED
        ) {
            return
        }

        ensureChannel(context)

        val tapId = payload.messageId ?: UUID.randomUUID().toString()
        val launch =
            context.packageManager.getLaunchIntentForPackage(context.packageName) ?: return
        launch.addFlags(
            Intent.FLAG_ACTIVITY_NEW_TASK or
                Intent.FLAG_ACTIVITY_SINGLE_TOP or
                Intent.FLAG_ACTIVITY_CLEAR_TOP,
        )
        launch.putExtra(EXTRA_TAP, true)
        launch.putExtra(EXTRA_TAP_ID, tapId)
        launch.putExtra(EXTRA_TAP_TITLE, payload.title)
        launch.putExtra(EXTRA_TAP_BODY, payload.body)
        launch.putExtra(EXTRA_TAP_DATA, toJson(payload.data))
        launch.putExtra(EXTRA_TAP_MESSAGE_ID, payload.messageId)

        val pendingIntent = PendingIntent.getActivity(
            context,
            tapId.hashCode(),
            launch,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )

        val smallIcon = context.applicationInfo.icon
            .takeIf { it != 0 } ?: android.R.drawable.ic_dialog_info

        val notification = NotificationCompat.Builder(context, CHANNEL_ID)
            .setSmallIcon(smallIcon)
            .setContentTitle(payload.title)
            .setContentText(payload.body)
            .setPriority(NotificationCompat.PRIORITY_DEFAULT)
            .setAutoCancel(true)
            .setContentIntent(pendingIntent)
            .build()

        NotificationManagerCompat.from(context).notify(tapId.hashCode(), notification)
    }

    private fun ensureChannel(context: Context) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        val manager =
            context.getSystemService(Context.NOTIFICATION_SERVICE) as? NotificationManager
                ?: return
        if (manager.getNotificationChannel(CHANNEL_ID) != null) return
        manager.createNotificationChannel(
            NotificationChannel(
                CHANNEL_ID,
                "Push notifications",
                NotificationManager.IMPORTANCE_DEFAULT,
            ),
        )
    }

    // ---- Pure-Kotlin (KMP) authorization callback --------------------------------

    private var authorizationCallback: ((Boolean) -> Unit)? = null

    /** One-shot callback used by [DioxusPushPlatform.requestAuthorization]. */
    internal fun requestAuthorizationWithCallback(onResult: (Boolean) -> Unit) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
            onResult(true)
            return
        }
        val activity = currentActivity?.get()
        if (activity == null) {
            onResult(false)
            return
        }
        synchronized(lock) { authorizationCallback = onResult }
        requestNotificationPermission(activity)
    }

    // ---- Helpers ------------------------------------------------------------------

    private fun toJson(data: Map<String, String>): String =
        JSONObject(data as Map<*, *>).toString()
}
