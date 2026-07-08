# dioxus-sdk-push

Remote (server-initiated) push notifications for Dioxus, plus server-side sending
helpers — an Expo-style abstraction across every target.

| Platform        | Backend                                   | Token (`DevicePushToken`)        |
| --------------- | ----------------------------------------- | -------------------------------- |
| Android         | Firebase Cloud Messaging (FCM)            | FCM registration token           |
| iOS / macOS     | Apple Push Notification service (APNs)    | APNs device token (hex)          |
| Web             | W3C Web Push (VAPID + service worker)     | serialized `PushSubscription`    |
| Windows / Linux | App-managed live connection + local toast | client-generated device id       |

## Client

```rust
use dioxus::prelude::*;
use dioxus_sdk_push::{init_push_manager, use_push_notifications, PushConfig};

fn app() -> Element {
    init_push_manager(PushConfig {
        web_vapid_public_key: Some("<vapid-public-key>".into()),
        fallback_endpoint: Some("wss://example.com/push".into()),
        ..Default::default()
    });

    let state = use_push_notifications();
    rsx! { "token: {state().token:?}" }
}
```

Send `state().token` to your backend; it sends pushes with the server helpers below.

## Server (`server` feature)

```rust
use dioxus_sdk_push::server::{FcmClient, Message};

let fcm = FcmClient::from_service_account_file("service-account.json").await?;
fcm.send(&Message::to_token(token).notification("Title", "Body")).await?;
```

A configurable fan-out `PushHub` notifies all / a topic / a single device across
backend instances, with in-memory or Redis (`redis` feature) registry + backplane:

```rust
use dioxus_sdk_push::server::{PushHub, PushEventMsg};

let hub = PushHub::builder().fcm(fcm).build();
hub.notify_topic("news", PushEventMsg::notification("Breaking", "…")).await?;
```

## Native prerequisites

- **Android** — the Kotlin bridge module (`kotlin/`, package `dev.dioxus.push`) is
  bundled automatically by `dx` >= 0.7.9 via the crate's plugin metadata: every
  Android build installs it as the Gradle submodule `:plugins:dioxuspushkotlin`,
  and its library manifest merges the FCM `<service>` and the
  `POST_NOTIFICATIONS` permission into the app. The app still provides a
  forwarding `MainActivity` and Firebase configuration — see
  [Android setup](#android-setup).
- **iOS / macOS** — enable the Push Notifications capability + `aps-environment`
  entitlement, sign with an App ID configured for APNs. The device-token capture
  swizzles the `wry` app delegate and may need revisiting across `wry`/`dioxus`
  versions.
- **Web** — generate a VAPID key pair, host `assets/dioxus-push-sw.js` at the URL in
  `PushConfig.web_service_worker_url` (default `/dioxus-push-sw.js`), serve over HTTPS.
- **Windows / Linux** — run a backend WebSocket endpoint and pass its URL in
  `PushConfig.fallback_endpoint`. This delivers only while the app is running and
  connected (it is not a wake-from-closed OS push service).

## Android setup

The Kotlin module ships inside this crate (`kotlin/`, see its
[README](./kotlin/README.md) for the JNI contract) and is auto-installed into the
`dx`-generated Android project — no gradle edits, no manifest edits, no copied
sources. Two things remain app-side:

### 1. Custom `MainActivity` forwarding lifecycle hooks

Notification taps and permission results arrive through `Activity` callbacks, so
the app must use a custom `MainActivity` that forwards them to
`dev.dioxus.push.DioxusPush`. Point `Dioxus.toml` at it:

```toml
[application]
android_main_activity = "android/MainActivity.kt"
```

and create `android/MainActivity.kt` (the file replaces the generated one
verbatim — keep the `dev.dioxus.main` package, and replace `com.example.myapp`
with your app's Android application id):

```kotlin
package dev.dioxus.main

import android.content.Intent
import android.os.Bundle
import dev.dioxus.push.DioxusPush

typealias BuildConfig = com.example.myapp.BuildConfig

class MainActivity : WryActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        DioxusPush.onActivityCreated(this, intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        DioxusPush.onNewIntent(intent)
    }

    override fun onRequestPermissionsResult(
        requestCode: Int,
        permissions: Array<out String>,
        grantResults: IntArray,
    ) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        DioxusPush.onRequestPermissionsResult(requestCode, grantResults)
    }
}
```

### 2. Firebase configuration

Firebase must know your project before `FirebaseMessaging` can mint tokens.
Either option works; no `google-services` gradle plugin is involved (the
`dx`-generated project doesn't apply one, and none is needed — that plugin only
generates the same string resources at build time):

- **Programmatic (recommended with `dx`)** — initialize from your custom
  `MainActivity.onCreate`, before the first `register()`, with the values from
  your `google-services.json`:

  ```kotlin
  import com.google.firebase.FirebaseApp
  import com.google.firebase.FirebaseOptions

  FirebaseApp.initializeApp(
      this,
      FirebaseOptions.Builder()
          .setApplicationId("1:1234567890:android:abc123")   // google-services.json: mobilesdk_app_id
          .setApiKey("AIza...")                               // api_key.current_key
          .setGcmSenderId("1234567890")                       // project_number
          .setProjectId("my-project-id")                      // project_id
          .build(),
  )
  ```

- **Android string resources** — Firebase's built-in `FirebaseInitProvider`
  auto-initializes when these resources exist in the app module (useful if your
  workflow adds res values to the generated Gradle project, e.g. via a small
  library module of your own):

  ```xml
  <resources>
      <string name="google_app_id" translatable="false">1:1234567890:android:abc123</string>
      <string name="google_api_key" translatable="false">AIza...</string>
      <string name="gcm_defaultSenderId" translatable="false">1234567890</string>
      <string name="project_id" translatable="false">my-project-id</string>
  </resources>
  ```

  Unlike activity-based init, `FirebaseInitProvider` runs at every process
  start, so Firebase is also initialized in processes FCM cold-starts in the
  background.

Notes:

- Data-only / foreground messages are displayed by the Kotlin module on the
  `dioxus_push` notification channel when they carry a `title`/`body` and no
  activity is in the foreground; taps surface as `PushEvent::NotificationTapped`.
- FCM can cold-start the process before the Rust library is loaded; events are
  buffered Kotlin-side and flushed when `register()` runs. If your native library
  is not named `main`, declare
  `<meta-data android:name="dev.dioxus.push.lib" android:value="<libname>"/>` or
  call `DioxusPush.setNativeLibraryName(...)`.
