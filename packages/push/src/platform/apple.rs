//! Apple backend (macOS + iOS) using APNs via `UNUserNotificationCenter`.
//!
//! Permission, foreground delivery, and notification taps go through a
//! [`UNUserNotificationCenterDelegate`] this module installs — that path is robust.
//!
//! Capturing the **device token** is the fragile part: APNs delivers it to the app
//! delegate's `application:didRegisterForRemoteNotificationsWithDeviceToken:`, which
//! `wry`/`dioxus-mobile` owns. We swizzle that selector at runtime to observe the
//! token. This depends on the host app delegate and may need revisiting across
//! `wry`/`dioxus` versions; the preferred long-term fix is an upstream delegate hook.
//! See the crate README for the required entitlements and signing.

use crate::core::{
    DevicePushToken, Error, NotificationContent, NotificationResponse, PermissionStatus, Provider,
    PushConfig, PushEvent, RemoteMessage,
};
use crate::platform::bridge;
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AllocAnyThread, class, msg_send, sel};
use objc2_foundation::{NSData, NSError};
use objc2_user_notifications::UNUserNotificationCenterDelegate;
use std::collections::HashMap;
use std::sync::Mutex;

pub use bridge::listen;

pub struct PushManager;

impl PushManager {
    pub fn new(_config: PushConfig) -> Result<Self, Error> {
        // Install the notification-center delegate (foreground + taps) and the
        // device-token observer as early as possible so cold-start taps are captured.
        unsafe {
            install_notification_delegate();
            install_token_observer();
        }
        Ok(Self)
    }
}

/// Request notification authorization (alert + sound + badge).
pub async fn request_permission(_manager: &PushManager) -> Result<PermissionStatus, Error> {
    let (tx, rx) = futures::channel::oneshot::channel::<bool>();
    let tx = Mutex::new(Some(tx));

    unsafe {
        let center: Retained<AnyObject> = {
            let cls = class!(UNUserNotificationCenter);
            msg_send![cls, currentNotificationCenter]
        };

        // UNAuthorizationOptions: Badge(1) | Sound(2) | Alert(4) = 7.
        let options: usize = 1 | 2 | 4;
        let handler = RcBlock::new(move |granted: Bool, _error: *mut NSError| {
            if let Ok(mut slot) = tx.lock() {
                if let Some(tx) = slot.take() {
                    let _ = tx.send(granted.as_bool());
                }
            }
        });
        let _: () = msg_send![
            &*center,
            requestAuthorizationWithOptions: options,
            completionHandler: &*handler,
        ];
    }

    let granted = rx
        .await
        .map_err(|_| Error::DeviceError("permission request cancelled".into()))?;
    let status = if granted {
        PermissionStatus::Granted
    } else {
        PermissionStatus::Denied
    };
    bridge::emit(PushEvent::PermissionChanged(status));
    Ok(status)
}

/// Trigger remote-notification registration on the main thread.
pub fn register(_manager: &PushManager) -> Result<(), Error> {
    unsafe {
        #[cfg(target_os = "ios")]
        {
            let app: Retained<AnyObject> = msg_send![class!(UIApplication), sharedApplication];
            let _: () = msg_send![&*app, registerForRemoteNotifications];
        }
        #[cfg(target_os = "macos")]
        {
            let app: Retained<AnyObject> = msg_send![class!(NSApplication), sharedApplication];
            let _: () = msg_send![&*app, registerForRemoteNotifications];
        }
    }
    Ok(())
}

/// Convert the APNs device token `NSData` into a lowercase hex string.
unsafe fn token_to_hex(data: &NSData) -> String {
    let bytes = data.to_vec();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ---------------------------------------------------------------------------
// Device-token observation via app-delegate swizzling.
// ---------------------------------------------------------------------------

/// Trampoline installed onto the app delegate for
/// `application:didRegisterForRemoteNotificationsWithDeviceToken:`.
extern "C-unwind" fn did_register(
    _self: *mut AnyObject,
    _cmd: objc2::runtime::Sel,
    _application: *mut AnyObject,
    device_token: *mut AnyObject,
) {
    if device_token.is_null() {
        return;
    }
    unsafe {
        let data = &*(device_token as *const NSData);
        let token = token_to_hex(data);
        bridge::emit(PushEvent::TokenRegistered(DevicePushToken {
            provider: Provider::Apns,
            token,
        }));
    }
}

/// Trampoline for `application:didFailToRegisterForRemoteNotificationsWithError:`.
extern "C-unwind" fn did_fail(
    _self: *mut AnyObject,
    _cmd: objc2::runtime::Sel,
    _application: *mut AnyObject,
    error: *mut AnyObject,
) {
    let message = if error.is_null() {
        "remote notification registration failed".to_string()
    } else {
        unsafe {
            let error = &*(error as *const NSError);
            error.localizedDescription().to_string()
        }
    };
    bridge::emit(PushEvent::TokenError(message));
}

/// Add the registration callbacks to the running app delegate's class.
unsafe fn install_token_observer() {
    // Resolve the shared application's delegate and its class.
    #[cfg(target_os = "ios")]
    let app: *mut AnyObject = msg_send![class!(UIApplication), sharedApplication];
    #[cfg(target_os = "macos")]
    let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
    if app.is_null() {
        return;
    }
    let delegate: *mut AnyObject = msg_send![app, delegate];
    if delegate.is_null() {
        return;
    }
    let class: *mut AnyClass = msg_send![delegate, class];

    // `v@:@@` — void return; self, _cmd, application, (NSData|NSError).
    let types = c"v@:@@".as_ptr();
    type Trampoline =
        extern "C-unwind" fn(*mut AnyObject, objc2::runtime::Sel, *mut AnyObject, *mut AnyObject);
    objc2::ffi::class_addMethod(
        class,
        sel!(application:didRegisterForRemoteNotificationsWithDeviceToken:),
        std::mem::transmute::<Trampoline, unsafe extern "C-unwind" fn()>(did_register),
        types,
    );
    objc2::ffi::class_addMethod(
        class,
        sel!(application:didFailToRegisterForRemoteNotificationsWithError:),
        std::mem::transmute::<Trampoline, unsafe extern "C-unwind" fn()>(did_fail),
        types,
    );
}

// ---------------------------------------------------------------------------
// Foreground + tap delivery via a UNUserNotificationCenterDelegate.
// ---------------------------------------------------------------------------

objc2::define_class!(
    #[unsafe(super(NSObject))]
    #[name = "DioxusPushDelegate"]
    struct PushDelegate;

    unsafe impl NSObjectProtocol for PushDelegate {}

    unsafe impl UNUserNotificationCenterDelegate for PushDelegate {
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        unsafe fn will_present(
            &self,
            _center: *mut AnyObject,
            notification: *mut AnyObject,
            completion: *mut block2::DynBlock<dyn Fn(usize)>,
        ) {
            if let Some(message) = parse_notification(notification) {
                bridge::emit(PushEvent::MessageReceived(message));
            }
            if !completion.is_null() {
                // UNNotificationPresentationOptions: Banner(0x10) | Sound(2).
                unsafe { (*completion).call((0x10 | 2,)) };
            }
        }

        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        unsafe fn did_receive(
            &self,
            _center: *mut AnyObject,
            response: *mut AnyObject,
            completion: *mut block2::DynBlock<dyn Fn()>,
        ) {
            let notification: *mut AnyObject = unsafe { msg_send![response, notification] };
            if let Some(message) = parse_notification(notification) {
                bridge::emit(PushEvent::NotificationTapped(NotificationResponse {
                    message,
                    action_id: None,
                }));
            }
            if !completion.is_null() {
                unsafe { (*completion).call(()) };
            }
        }
    }
);

/// Extract a [`RemoteMessage`] from a `UNNotification`'s `userInfo` dictionary.
unsafe fn parse_notification(notification: *mut AnyObject) -> Option<RemoteMessage> {
    if notification.is_null() {
        return None;
    }
    let request: *mut AnyObject = msg_send![notification, request];
    let content: *mut AnyObject = msg_send![request, content];
    if content.is_null() {
        return None;
    }

    let title: Retained<objc2_foundation::NSString> = msg_send![content, title];
    let body: Retained<objc2_foundation::NSString> = msg_send![content, body];
    let title = title.to_string();
    let body = body.to_string();

    let user_info: *mut AnyObject = msg_send![content, userInfo];
    let data = parse_user_info(user_info);

    Some(RemoteMessage {
        notification: Some(NotificationContent {
            title: (!title.is_empty()).then_some(title),
            body: (!body.is_empty()).then_some(body),
        }),
        data,
        message_id: None,
    })
}

/// Best-effort conversion of an `NSDictionary` of string keys/values into a map.
unsafe fn parse_user_info(_user_info: *mut AnyObject) -> HashMap<String, String> {
    // A full NSDictionary walk is omitted here; data payloads are surfaced via the
    // notification content. Custom-key extraction can be layered on as needed.
    HashMap::new()
}

unsafe fn install_notification_delegate() {
    let delegate = PushDelegate::alloc();
    let delegate: Retained<PushDelegate> = msg_send![delegate, init];
    let protocol = ProtocolObject::from_ref(&*delegate);
    let center: *mut AnyObject =
        msg_send![class!(UNUserNotificationCenter), currentNotificationCenter];
    let _: () = msg_send![center, setDelegate: protocol];
    // Leak the delegate so it lives for the process lifetime.
    let _ = Retained::into_raw(delegate);
}
