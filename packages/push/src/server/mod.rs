//! Server-side push sending helpers and a configurable fan-out hub.
//!
//! Enabled with the `server` feature. Provides one concrete client per provider
//! ([`FcmClient`], [`ApnsClient`], [`WebPushClient`]) plus a [`PushHub`] that fans a
//! single event out to many devices across backend instances via a pluggable
//! [`Backplane`] and [`DeviceRegistry`].

mod apns;
mod backplane;
mod fcm;
mod hub;
mod live;
mod registry;
mod web_push;

#[cfg(test)]
mod tests;

pub use apns::{ApnsClient, ApnsPayload, Endpoint};
pub use backplane::{Backplane, InProcessBackplane};
pub use fcm::{FcmClient, FcmResponse, Message};
pub use hub::{PushEventMsg, PushHub, PushHubBuilder, Target};
pub use live::{Frame, LiveConnections};
pub use registry::{DeviceRegistry, InMemoryRegistry, RegisteredDevice};
pub use web_push::WebPushClient;

#[cfg(feature = "redis")]
pub use backplane::RedisBackplane;
#[cfg(feature = "redis")]
pub use registry::RedisRegistry;

/// The default topic every registered device is implicitly subscribed to.
pub const TOPIC_ALL: &str = "all";

/// Errors returned by the server-side helpers.
#[derive(thiserror::Error, Debug)]
pub enum ServerError {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("auth error: {0}")]
    Auth(#[from] gcp_auth::Error),
    #[error("apns error: {0}")]
    Apns(#[from] a2::Error),
    #[error("web push error: {0}")]
    WebPush(#[from] ::web_push::WebPushError),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("fcm rejected the request ({status}): {body}")]
    FcmRejected { status: u16, body: String },
    #[error("the device token is no longer valid")]
    TokenInvalid,
    #[error("configuration error: {0}")]
    Config(String),
    #[cfg(feature = "redis")]
    #[error("redis error: {0}")]
    Redis(#[from] redis::RedisError),
}

impl ServerError {
    /// Whether this error means the provider considers the token permanently dead, so
    /// the hub should evict it from the registry.
    pub fn is_token_invalid(&self) -> bool {
        match self {
            ServerError::TokenInvalid => true,
            ServerError::FcmRejected { status, body } => {
                *status == 404 || body.contains("UNREGISTERED") || body.contains("INVALID_ARGUMENT")
            }
            _ => false,
        }
    }
}
