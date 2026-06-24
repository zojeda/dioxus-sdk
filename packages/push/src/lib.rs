//! Remote push notifications for Dioxus.
//!
//! This crate provides an Expo-style push notification abstraction across every target:
//!
//! - **Android** — Firebase Cloud Messaging (FCM)
//! - **iOS / macOS** — Apple Push Notification service (APNs)
//! - **Web** — the W3C Web Push API (VAPID + service worker)
//! - **Windows / Linux** — an app-managed live connection that surfaces messages as
//!   local desktop notifications
//!
//! The [`client`](#client) side registers the device and exposes received messages as
//! reactive signals. The optional [`server`] side (enabled with the `server` feature)
//! sends pushes to raw tokens and provides a configurable fan-out [`Hub`](server::PushHub).

pub mod core;
pub mod platform;

pub use self::core::*;

#[cfg(feature = "client")]
pub mod use_push;
#[cfg(feature = "client")]
pub use self::use_push::*;

#[cfg(all(feature = "server", not(target_family = "wasm")))]
pub mod server;
#[cfg(all(feature = "server", not(target_family = "wasm")))]
pub use self::server::*;
