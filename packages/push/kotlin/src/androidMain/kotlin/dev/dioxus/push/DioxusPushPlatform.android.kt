package dev.dioxus.push

import com.google.firebase.messaging.FirebaseMessaging

/**
 * Android `actual`: delegates to Firebase Messaging and the permission plumbing
 * in [DioxusPush].
 *
 * Dioxus apps do not need this object — the Rust crate drives [DioxusPush]
 * directly over JNI. It exists for pure-Kotlin/KMP consumers of the module.
 */
actual object DioxusPushPlatform {
    /**
     * Request the Android 13+ `POST_NOTIFICATIONS` runtime permission.
     *
     * Requires a current activity, i.e. [DioxusPush.onActivityCreated] must have
     * been forwarded first; otherwise [onResult] receives `false`. The prompt
     * result additionally requires the activity to forward
     * [DioxusPush.onRequestPermissionsResult]. On Android < 13 this reports
     * `true` immediately.
     */
    actual fun requestAuthorization(onResult: (Boolean) -> Unit) {
        DioxusPush.requestAuthorizationWithCallback(onResult)
    }

    /** Fetch the FCM registration token. */
    actual fun fetchToken(onSuccess: (String) -> Unit, onError: (String) -> Unit) {
        FirebaseMessaging.getInstance().token.addOnCompleteListener { task ->
            if (task.isSuccessful) {
                onSuccess(task.result)
            } else {
                onError(task.exception?.message ?: "getToken failed")
            }
        }
    }
}
