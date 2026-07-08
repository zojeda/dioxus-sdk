package dev.dioxus.push

import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage

/**
 * FCM entry point. Declared in this library's `AndroidManifest.xml` (with the
 * `com.google.firebase.MESSAGING_EVENT` intent-filter), which the manifest
 * merger propagates into the consuming app — no app-side declaration needed.
 *
 * Behavior per message class:
 * - **Notification messages, app in background** — the FCM SDK posts the tray
 *   notification itself and this callback is *not* invoked; the tap relaunches
 *   the launcher activity and is picked up by [DioxusPush.onActivityCreated].
 * - **Notification messages, app in foreground** — forwarded to Rust as a
 *   received message; nothing is posted to the tray (the app is visible).
 * - **Data-only messages** — always forwarded to Rust; a tray notification is
 *   posted only when the payload carries displayable content (`title` / `body`
 *   keys) and no activity of the app is in the foreground.
 *
 * May run in a cold-started process without the Rust library loaded; all
 * native calls are guarded and buffered by [DioxusPush].
 */
class DioxusFirebaseMessagingService : FirebaseMessagingService() {
    override fun onNewToken(token: String) {
        DioxusPush.configureFromContext(this)
        DioxusPush.emitToken(token)
    }

    override fun onMessageReceived(message: RemoteMessage) {
        DioxusPush.configureFromContext(this)
        DioxusPush.trackApplication(application)

        val payload = PushPayload(
            title = message.notification?.title ?: message.data["title"],
            body = message.notification?.body ?: message.data["body"],
            data = message.data,
            messageId = message.messageId,
        )
        DioxusPush.handleRemoteMessage(this, payload)
    }
}
