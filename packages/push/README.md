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

- **Android** — add `google-services.json`, the `google-services` gradle plugin and
  `firebase-messaging` dependency, copy `android_shim/DioxusFirebaseMessagingService.kt`
  into your Kotlin source set, and declare the `<service>` (with the
  `com.google.firebase.MESSAGING_EVENT` intent-filter) plus the `POST_NOTIFICATIONS`
  permission in `AndroidManifest.xml`.
- **iOS / macOS** — enable the Push Notifications capability + `aps-environment`
  entitlement, sign with an App ID configured for APNs. The device-token capture
  swizzles the `wry` app delegate and may need revisiting across `wry`/`dioxus`
  versions.
- **Web** — generate a VAPID key pair, host `assets/dioxus-push-sw.js` at the URL in
  `PushConfig.web_service_worker_url` (default `/dioxus-push-sw.js`), serve over HTTPS.
- **Windows / Linux** — run a backend WebSocket endpoint and pass its URL in
  `PushConfig.fallback_endpoint`. This delivers only while the app is running and
  connected (it is not a wake-from-closed OS push service).
