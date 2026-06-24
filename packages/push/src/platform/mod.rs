//! Per-platform push backends. Each module exposes the same surface:
//! `PushManager`, `request_permission`, `register`, and `listen`.

#[cfg(any(
    target_os = "android",
    target_os = "ios",
    target_os = "macos",
    target_family = "wasm",
    target_os = "windows",
    target_os = "linux"
))]
mod bridge;

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "android")]
pub use self::android::*;

#[cfg(any(target_os = "ios", target_os = "macos"))]
mod apple;
#[cfg(any(target_os = "ios", target_os = "macos"))]
pub use self::apple::*;

#[cfg(target_family = "wasm")]
mod web;
#[cfg(target_family = "wasm")]
pub use self::web::*;

#[cfg(all(
    not(target_family = "wasm"),
    any(target_os = "windows", target_os = "linux")
))]
mod fallback;
#[cfg(all(
    not(target_family = "wasm"),
    any(target_os = "windows", target_os = "linux")
))]
pub use self::fallback::*;

#[cfg(not(any(
    target_os = "android",
    target_os = "ios",
    target_os = "macos",
    target_family = "wasm",
    target_os = "windows",
    target_os = "linux"
)))]
mod unsupported;
#[cfg(not(any(
    target_os = "android",
    target_os = "ios",
    target_os = "macos",
    target_family = "wasm",
    target_os = "windows",
    target_os = "linux"
)))]
pub use self::unsupported::*;
