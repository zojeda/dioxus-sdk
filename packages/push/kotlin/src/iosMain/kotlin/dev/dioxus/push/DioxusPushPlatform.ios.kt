package dev.dioxus.push

import platform.UIKit.UIApplication
import platform.UserNotifications.UNAuthorizationOptionAlert
import platform.UserNotifications.UNAuthorizationOptionBadge
import platform.UserNotifications.UNAuthorizationOptionSound
import platform.UserNotifications.UNUserNotificationCenter
import platform.darwin.dispatch_async
import platform.darwin.dispatch_get_main_queue

/**
 * iOS `actual` for pure-Kotlin/KMP consumers.
 *
 * On Dioxus iOS this file is *not* used: the Rust crate (`src/platform/apple.rs`)
 * talks to `UserNotifications` / `UIApplication` natively over objc2 and captures
 * the APNs device token by swizzling the app delegate. This source set exists so
 * the module remains a usable KMP library outside Dioxus; it is only compiled
 * when the `dev.dioxus.push.ios` Gradle property is set (see build.gradle.kts).
 */
actual object DioxusPushPlatform {
    private var deviceTokenHex: String? = null
    private var pendingTokenCallback: ((String) -> Unit)? = null

    /**
     * Request notification authorization (badge | sound | alert) from
     * `UNUserNotificationCenter`. [onResult] fires on a UserNotifications
     * callback thread.
     */
    actual fun requestAuthorization(onResult: (Boolean) -> Unit) {
        val options =
            UNAuthorizationOptionBadge or UNAuthorizationOptionSound or UNAuthorizationOptionAlert
        UNUserNotificationCenter.currentNotificationCenter()
            .requestAuthorizationWithOptions(options) { granted, _ ->
                onResult(granted)
            }
    }

    /**
     * Begin APNs registration. `registerForRemoteNotifications` is dispatched on
     * the main thread, but UIKit has no completion callback for it: the token
     * arrives through the AppDelegate. Unless a token was already forwarded via
     * [onDeviceToken] (in which case [onSuccess] fires immediately), this reports
     * the situation through [onError] and invokes [onSuccess] later, once the
     * AppDelegate forwards the token:
     *
     * ```swift
     * func application(_ application: UIApplication,
     *                  didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data) {
     *     let hex = deviceToken.map { String(format: "%02x", $0) }.joined()
     *     DioxusPushPlatform.shared.onDeviceToken(tokenHex: hex)
     * }
     * ```
     */
    actual fun fetchToken(onSuccess: (String) -> Unit, onError: (String) -> Unit) {
        val cached = deviceTokenHex
        if (cached != null) {
            onSuccess(cached)
            return
        }
        pendingTokenCallback = onSuccess
        dispatch_async(dispatch_get_main_queue()) {
            UIApplication.sharedApplication.registerForRemoteNotifications()
        }
        onError(
            "APNs delivers the device token asynchronously through the AppDelegate: " +
                "forward application(_:didRegisterForRemoteNotificationsWithDeviceToken:) " +
                "to DioxusPushPlatform.onDeviceToken(tokenHex); onSuccess fires then."
        )
    }

    /**
     * Forward the APNs device token (hex-encoded) from the AppDelegate. Stores it
     * for subsequent [fetchToken] calls and completes a pending [fetchToken].
     */
    fun onDeviceToken(tokenHex: String) {
        deviceTokenHex = tokenHex
        pendingTokenCallback?.invoke(tokenHex)
        pendingTokenCallback = null
    }
}
