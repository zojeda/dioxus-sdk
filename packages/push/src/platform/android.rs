//! Android backend using Firebase Cloud Messaging (FCM).
//!
//! Token retrieval, message delivery, tray display and tap detection live in the
//! Kotlin Multiplatform module shipped under `kotlin/` (`dev.dioxus.push`). The
//! [`kotlin_plugin`] block below embeds `manganis` plugin metadata, so `dx`
//! (>= 0.7.9) bundles that module into the generated Android project
//! automatically — no manual Kotlin or manifest setup. See the crate README for
//! the two things the app must still provide (a forwarding `MainActivity` and
//! Firebase configuration).
//!
//! The Kotlin object's `external` methods bind to the `#[unsafe(no_mangle)]` JNI
//! exports below; they may run on Firebase/Binder threads and simply forward into
//! [`bridge::emit`].

use crate::core::{
    DevicePushToken, Error, NotificationContent, NotificationResponse, PermissionStatus, Provider,
    PushConfig, PushEvent, RemoteMessage,
};
use crate::platform::bridge;
use jni::JNIEnv;
use jni::objects::{JClass, JObject, JString, JValue};
use jni::sys::jboolean;

pub use bridge::listen;

pub struct PushManager;

impl PushManager {
    pub fn new(_config: PushConfig) -> Result<Self, Error> {
        Ok(Self)
    }
}

/// Request the Android 13+ `POST_NOTIFICATIONS` runtime permission. The result is
/// delivered asynchronously as [`PushEvent::PermissionChanged`].
pub async fn request_permission(_manager: &PushManager) -> Result<PermissionStatus, Error> {
    call_shim_static("requestNotificationPermission");
    Ok(PermissionStatus::Undetermined)
}

/// Begin FCM registration. The token arrives asynchronously as
/// [`PushEvent::TokenRegistered`] via [`Java_dev_dioxus_push_DioxusPush_nativeOnToken`].
pub fn register(_manager: &PushManager) -> Result<(), Error> {
    call_shim_static("register");
    Ok(())
}

/// Dispatch a call to a `DioxusPush.<name>(Context)` static method on the UI thread.
fn call_shim_static(method: &'static str) {
    dioxus::mobile::wry::prelude::dispatch(
        move |env: &mut JNIEnv, activity: &JObject, _webview| {
            let Ok(class) = env.find_class("dev/dioxus/push/DioxusPush") else {
                bridge::emit(PushEvent::TokenError(
                    "DioxusPush class not found (dx >= 0.7.9 should bundle the Kotlin module automatically)".into(),
                ));
                return;
            };
            let _ = env.call_static_method(
                class,
                method,
                "(Landroid/content/Context;)V",
                &[JValue::Object(activity)],
            );
        },
    );
}

/// Read a (possibly null) Java string into an `Option<String>`.
fn read_string(env: &mut JNIEnv, value: &JString) -> Option<String> {
    if value.is_null() {
        return None;
    }
    env.get_string(value).ok().map(|s| s.into())
}

/// `DioxusPush.nativeOnToken(String)` — a fresh or refreshed FCM token.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dioxus_push_DioxusPush_nativeOnToken(
    mut env: JNIEnv,
    _class: JClass,
    token: JString,
) {
    if let Some(token) = read_string(&mut env, &token) {
        bridge::emit(PushEvent::TokenRegistered(DevicePushToken {
            provider: Provider::Fcm,
            token,
        }));
    }
}

/// `DioxusPush.nativeOnTokenError(String)`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dioxus_push_DioxusPush_nativeOnTokenError(
    mut env: JNIEnv,
    _class: JClass,
    error: JString,
) {
    let message = read_string(&mut env, &error).unwrap_or_else(|| "unknown error".into());
    bridge::emit(PushEvent::TokenError(message));
}

/// Parse the `(title?, body?, dataJson, messageId?)` JNI strings shared by
/// [`Java_dev_dioxus_push_DioxusPush_nativeOnMessage`] and
/// [`Java_dev_dioxus_push_DioxusPush_nativeOnTap`] into a [`RemoteMessage`].
fn read_remote_message(
    env: &mut JNIEnv,
    title: &JString,
    body: &JString,
    data_json: &JString,
    message_id: &JString,
) -> RemoteMessage {
    let title = read_string(env, title);
    let body = read_string(env, body);
    let data = read_string(env, data_json)
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default();
    let message_id = read_string(env, message_id);

    let notification = if title.is_some() || body.is_some() {
        Some(NotificationContent { title, body })
    } else {
        None
    };

    RemoteMessage {
        notification,
        data,
        message_id,
    }
}

/// `DioxusPush.nativeOnMessage(String?, String?, String, String?)` —
/// (title, body, dataJson, messageId).
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dioxus_push_DioxusPush_nativeOnMessage(
    mut env: JNIEnv,
    _class: JClass,
    title: JString,
    body: JString,
    data_json: JString,
    message_id: JString,
) {
    let message = read_remote_message(&mut env, &title, &body, &data_json, &message_id);
    bridge::emit(PushEvent::MessageReceived(message));
}

/// `DioxusPush.nativeOnTap(String?, String?, String, String?)` —
/// (title, body, dataJson, messageId). Fired when the user taps a notification
/// (tray notifications posted by the Kotlin module, or FCM system-tray
/// notifications whose tap relaunched the app).
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dioxus_push_DioxusPush_nativeOnTap(
    mut env: JNIEnv,
    _class: JClass,
    title: JString,
    body: JString,
    data_json: JString,
    message_id: JString,
) {
    let message = read_remote_message(&mut env, &title, &body, &data_json, &message_id);
    bridge::emit(PushEvent::NotificationTapped(NotificationResponse {
        message,
        action_id: None,
    }));
}

/// `DioxusPush.nativeOnPermission(boolean)`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dioxus_push_DioxusPush_nativeOnPermission(
    _env: JNIEnv,
    _class: JClass,
    granted: jboolean,
) {
    let status = if granted != 0 {
        PermissionStatus::Granted
    } else {
        PermissionStatus::Denied
    };
    bridge::emit(PushEvent::PermissionChanged(status));
}

/// Auto-bundling glue for `dx` >= 0.7.9.
///
/// The `#[manganis::ffi]` expansion embeds an
/// [`AndroidArtifactMetadata`](manganis::android::AndroidArtifactMetadata)
/// record in a linker section of the compiled library. `dx` collects it and
/// installs the crate's `kotlin/` folder (path relative to this crate's
/// `CARGO_MANIFEST_DIR`) into the generated Android project as the Gradle
/// submodule `:plugins:dioxuspushkotlin`, wiring `settings.gradle` and the app
/// module's dependencies automatically.
///
/// The generated `DioxusPushKotlin` wrapper (instantiating the empty
/// `dev.dioxus.push.DioxusPushKotlin` bridge class) is unused at runtime — all
/// push traffic flows through the hand-written JNI bindings above — hence the
/// lint allowances.
mod kotlin_plugin {
    #![allow(dead_code, unused_imports)]

    #[manganis::ffi("kotlin")]
    extern "Kotlin" {
        pub type DioxusPushKotlin;
    }
}
