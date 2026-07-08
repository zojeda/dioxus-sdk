package dev.dioxus.push

/**
 * A platform-neutral view of a received push message.
 *
 * @property title display title, when the message carried one.
 * @property body display body, when the message carried one.
 * @property data arbitrary key/value payload attached by the sender.
 * @property messageId provider message id, when available.
 */
data class PushPayload(
    val title: String?,
    val body: String?,
    val data: Map<String, String>,
    val messageId: String?,
)

/**
 * Minimal shared push surface for Kotlin Multiplatform consumers.
 *
 * The surface is intentionally small and honest: it covers only what every
 * platform can actually do from shared code — asking for notification
 * authorization and kicking off token retrieval. Everything event-driven
 * (message delivery, notification taps) is inherently platform-entry-point
 * bound and lives in the platform source sets ([DioxusPush] on Android).
 *
 * Dioxus apps do not call this object: the Rust crate (`dioxus-sdk-push`)
 * drives [DioxusPush] directly over JNI on Android and talks to APNs natively
 * on iOS/macOS. This exists for pure-Kotlin/KMP consumers of the module.
 */
expect object DioxusPushPlatform {
    /**
     * Request notification authorization from the OS.
     *
     * [onResult] receives `true` when authorization is (already) granted.
     * It may be invoked synchronously or from a platform callback thread.
     */
    fun requestAuthorization(onResult: (Boolean) -> Unit)

    /**
     * Fetch the platform push token (FCM registration token on Android,
     * hex-encoded APNs device token on iOS).
     *
     * Exactly which callback fires — and when — is platform-dependent; see the
     * `actual` declarations for the caveats (on iOS the token is delivered
     * through the AppDelegate and must be forwarded manually).
     */
    fun fetchToken(onSuccess: (String) -> Unit, onError: (String) -> Unit)
}
