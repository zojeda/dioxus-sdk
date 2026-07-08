# dev.dioxus.push — KMP bridge for `dioxus-sdk-push`

Kotlin Multiplatform library backing the Android side of the
[`dioxus-sdk-push`](../README.md) crate: Firebase Cloud Messaging integration,
tray-notification display, notification-tap delivery, and the runtime
`POST_NOTIFICATIONS` permission flow.

## How it is consumed

**Dioxus apps (the normal case).** Nothing to add: the Rust crate embeds plugin
metadata via `#[manganis::ffi]`, and `dx` (>= 0.7.9) bundles this folder into the
generated Android project as the Gradle submodule `:plugins:dioxuspushkotlin`
during every Android build. The library's `AndroidManifest.xml` merges the FCM
`<service>` and the `POST_NOTIFICATIONS` permission into the app manifest
automatically. See the crate README for the two things the app must still do
(custom `MainActivity` forwarding + Firebase configuration).

**Standalone KMP.** The module also works as a plain KMP library
(`commonMain` + `androidMain` + `iosMain`):

```kotlin
// settings.gradle.kts of your project
include(":push")
project(":push").projectDir = file("path/to/dioxus-sdk/packages/push/kotlin")
```

- Android target: always configured (AGP 8.7.0 / Kotlin 2.0.20, JVM 17,
  compileSdk 34, minSdk 24).
- iOS targets (`iosArm64`, `iosX64`, `iosSimulatorArm64`): opt-in via the Gradle
  property `dev.dioxus.push.ios=true`, so the `dx`-embedded Android build never
  touches Kotlin/Native.

The shared surface is `PushPayload` plus:

```kotlin
expect object DioxusPushPlatform {
    fun requestAuthorization(onResult: (Boolean) -> Unit)
    fun fetchToken(onSuccess: (String) -> Unit, onError: (String) -> Unit)
}
```

On iOS the `actual` exists for pure-Kotlin consumers only — Dioxus iOS apps get
APNs handling natively from the Rust crate (`src/platform/apple.rs`). The APNs
token must be forwarded from the AppDelegate to
`DioxusPushPlatform.onDeviceToken(tokenHex)`.

## JNI contract

Keep in sync with `packages/push/src/platform/android.rs`. All classes live in
the `dev.dioxus.push` namespace.

### Rust -> Kotlin (static calls on `dev/dioxus/push/DioxusPush`, UI thread)

| Method | JNI signature | Purpose |
| --- | --- | --- |
| `register` | `(Landroid/content/Context;)V` | Fetch the FCM token; also flushes the buffered-event queue |
| `requestNotificationPermission` | `(Landroid/content/Context;)V` | Request the Android 13+ runtime permission |

### Kotlin -> Rust (`external fun`s on `DioxusPush`, may fire from any thread)

| Rust export | JNI signature | Event emitted |
| --- | --- | --- |
| `Java_dev_dioxus_push_DioxusPush_nativeOnToken` | `(Ljava/lang/String;)V` | `PushEvent::TokenRegistered` |
| `Java_dev_dioxus_push_DioxusPush_nativeOnTokenError` | `(Ljava/lang/String;)V` | `PushEvent::TokenError` |
| `Java_dev_dioxus_push_DioxusPush_nativeOnMessage` | `(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)V` | `PushEvent::MessageReceived` (title?, body?, dataJson, messageId?) |
| `Java_dev_dioxus_push_DioxusPush_nativeOnTap` | `(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)V` | `PushEvent::NotificationTapped` (title?, body?, dataJson, messageId?) |
| `Java_dev_dioxus_push_DioxusPush_nativeOnPermission` | `(Z)V` | `PushEvent::PermissionChanged` |

### Rust instantiation shim

`dev.dioxus.push.DioxusPushKotlin` — empty class with an
`(Landroid/app/Activity;)V` constructor, instantiated by the wrapper that
`#[manganis::ffi]` generates. It carries the plugin metadata; it does no work.

## App-forwarded hooks (`DioxusPush`)

| Hook | Forward from | Why |
| --- | --- | --- |
| `onActivityCreated(activity, intent)` | `Activity.onCreate` | Cold-start tap detection, foreground tracking |
| `onNewIntent(intent)` | `Activity.onNewIntent` | Taps while the app is running (also call `setIntent(intent)`) |
| `onRequestPermissionsResult(code, grantResults)` | `Activity.onRequestPermissionsResult` | Permission prompt result (request code `0x5058`) |

## Behavior notes

- **Cold-start safety** — FCM may start the process before the Rust library is
  loaded. Every `native*` call is guarded (`System.loadLibrary` attempt +
  `UnsatisfiedLinkError` catch) and buffered in a synchronized queue, flushed
  when Rust calls `register`. The library name defaults to `"main"`; override
  with `DioxusPush.setNativeLibraryName(...)` or a
  `<meta-data android:name="dev.dioxus.push.lib" android:value="..."/>` entry.
- **Tray display** — foreground notification-messages and data-only messages are
  never shown by the FCM SDK. `DioxusPush` posts a notification on the
  `dioxus_push` channel when the message has displayable content (`title`/`body`,
  from the notification block or data keys) *and* no activity is in the
  foreground. The tap `PendingIntent` (`FLAG_IMMUTABLE`) relaunches the launcher
  activity with `dev.dioxus.push.tap` extras.
- **Tap dedup** — taps are deduplicated by message id (or a generated tap id), so
  an intent that is both delivered to `onNewIntent` and later replayed through a
  recreated activity's `onCreate` reports a single `NotificationTapped`.
- **System-tray taps** — background notification-messages are posted by the FCM
  SDK itself; the tap relaunch carries `google.message_id` / `google.sent_time`
  plus the data payload as extras. Bookkeeping keys (`google.*`, `gcm.*`,
  `collapse_key`, `from`) are stripped before the payload reaches Rust; the tray
  title/body are not recoverable in this path.
