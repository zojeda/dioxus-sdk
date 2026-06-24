//! APNs sending client (token / `.p8` JWT auth).

use super::ServerError;
use a2::{
    Client, ClientConfig, DefaultNotificationBuilder, NotificationBuilder, NotificationOptions,
};
use std::collections::HashMap;

/// Which APNs environment to target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endpoint {
    Production,
    Sandbox,
}

impl From<Endpoint> for a2::Endpoint {
    fn from(endpoint: Endpoint) -> Self {
        match endpoint {
            Endpoint::Production => a2::Endpoint::Production,
            Endpoint::Sandbox => a2::Endpoint::Sandbox,
        }
    }
}

/// The display + custom payload for an APNs push.
#[derive(Debug, Clone, Default)]
pub struct ApnsPayload {
    pub title: Option<String>,
    pub body: Option<String>,
    pub custom: HashMap<String, String>,
}

/// A token-authenticated (`.p8`) APNs client.
pub struct ApnsClient {
    client: Client,
}

impl ApnsClient {
    /// Build a client using token-based authentication.
    ///
    /// `p8_pem` is the contents of the `.p8` private key; `key_id` and `team_id` come
    /// from the Apple Developer portal.
    pub fn new_token(
        p8_pem: &[u8],
        key_id: impl Into<String>,
        team_id: impl Into<String>,
        endpoint: Endpoint,
    ) -> Result<Self, ServerError> {
        let mut reader = std::io::Cursor::new(p8_pem);
        let config = ClientConfig::new(endpoint.into());
        let client = Client::token(&mut reader, key_id, team_id, config)?;
        Ok(Self { client })
    }

    /// Send a push to a device token. `bundle_id` becomes the `apns-topic`.
    pub async fn send(
        &self,
        device_token: &str,
        bundle_id: &str,
        payload: ApnsPayload,
    ) -> Result<(), ServerError> {
        let options = NotificationOptions {
            apns_topic: Some(bundle_id),
            ..Default::default()
        };

        let mut builder = DefaultNotificationBuilder::new();
        if let Some(title) = &payload.title {
            builder = builder.set_title(title);
        }
        if let Some(body) = &payload.body {
            builder = builder.set_body(body);
        }

        let mut built = builder.build(device_token, options);
        for (key, value) in &payload.custom {
            built.add_custom_data(key, value)?;
        }

        match self.client.send(built).await {
            Ok(_) => Ok(()),
            Err(a2::Error::ResponseError(resp))
                if matches!(
                    resp.error.as_ref().map(|e| &e.reason),
                    Some(a2::ErrorReason::Unregistered) | Some(a2::ErrorReason::BadDeviceToken)
                ) =>
            {
                Err(ServerError::TokenInvalid)
            }
            Err(e) => Err(e.into()),
        }
    }
}
