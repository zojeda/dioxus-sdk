package dev.dioxus.push

import android.app.Activity
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import com.google.firebase.messaging.FirebaseMessaging
import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage
import org.json.JSONObject

/**
 * Bridge between Firebase Cloud Messaging and the Rust `dioxus-sdk-push` crate.
 *
 * Copy this file into your app's Kotlin source set (keeping the
 * `dev.dioxus.push` package) and declare [DioxusFirebaseMessagingService] in your
 * `AndroidManifest.xml`:
 *
 * ```xml
 * <service
 *     android:name="dev.dioxus.push.DioxusFirebaseMessagingService"
 *     android:exported="false">
 *     <intent-filter>
 *         <action android:name="com.google.firebase.MESSAGING_EVENT" />
 *     </intent-filter>
 * </service>
 * <uses-permission android:name="android.permission.POST_NOTIFICATIONS" />
 * ```
 */
object DioxusPush {
    private const val PERMISSION_REQUEST_CODE = 0x44494f58 // "DIOX"

    // Implemented in Rust (JNI exports in platform/android.rs).
    external fun nativeOnToken(token: String)
    external fun nativeOnTokenError(error: String)
    external fun nativeOnMessage(title: String?, body: String?, dataJson: String, messageId: String?)
    external fun nativeOnPermission(granted: Boolean)

    /** Fetch the current FCM registration token. Called from Rust via JNI. */
    @JvmStatic
    fun register(context: Context) {
        FirebaseMessaging.getInstance().token
            .addOnCompleteListener { task ->
                if (task.isSuccessful) {
                    nativeOnToken(task.result)
                } else {
                    nativeOnTokenError(task.exception?.message ?: "getToken failed")
                }
            }
    }

    /** Request the Android 13+ POST_NOTIFICATIONS runtime permission. Called from Rust. */
    @JvmStatic
    fun requestNotificationPermission(context: Context) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
            nativeOnPermission(true)
            return
        }
        val granted = ContextCompat.checkSelfPermission(
            context,
            android.Manifest.permission.POST_NOTIFICATIONS
        ) == PackageManager.PERMISSION_GRANTED

        if (granted) {
            nativeOnPermission(true)
        } else if (context is Activity) {
            ActivityCompat.requestPermissions(
                context,
                arrayOf(android.Manifest.permission.POST_NOTIFICATIONS),
                PERMISSION_REQUEST_CODE
            )
            // The actual result should be forwarded from the Activity's
            // onRequestPermissionsResult by calling nativeOnPermission(...).
        } else {
            nativeOnPermission(false)
        }
    }
}

class DioxusFirebaseMessagingService : FirebaseMessagingService() {
    override fun onNewToken(token: String) {
        DioxusPush.nativeOnToken(token)
    }

    override fun onMessageReceived(message: RemoteMessage) {
        val dataJson = JSONObject(message.data as Map<*, *>).toString()
        DioxusPush.nativeOnMessage(
            message.notification?.title,
            message.notification?.body,
            dataJson,
            message.messageId
        )
    }
}
