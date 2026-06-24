//! Web Push (VAPID) sending client.

use super::ServerError;
use crate::core::NotificationContent;
use serde::Serialize;
use web_push::{
    ContentEncoding, HyperWebPushClient, SubscriptionInfo, VapidSignatureBuilder,
    WebPushClient as _, WebPushMessageBuilder,
};

/// A client that sends VAPID-signed Web Push messages to browser subscriptions.
pub struct WebPushClient {
    client: HyperWebPushClient,
    vapid_pem: Vec<u8>,
}

/// The JSON payload delivered to the service worker's `push` handler.
#[derive(Debug, Serialize)]
struct WebPushPayload<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    notification: Option<&'a NotificationContent>,
    #[serde(skip_serializing_if = "std::collections::HashMap::is_empty")]
    data: &'a std::collections::HashMap<String, String>,
}

impl WebPushClient {
    /// Build a client from the VAPID private key in PEM form.
    pub fn new(vapid_private_pem: impl Into<Vec<u8>>) -> Self {
        Self {
            client: HyperWebPushClient::new(),
            vapid_pem: vapid_private_pem.into(),
        }
    }

    /// Send to a browser subscription. `subscription_json` is the [`Provider::WebPush`]
    /// token verbatim (the serialized `PushSubscription`).
    ///
    /// [`Provider::WebPush`]: crate::Provider::WebPush
    pub async fn send(
        &self,
        subscription_json: &str,
        notification: Option<&NotificationContent>,
        data: &std::collections::HashMap<String, String>,
    ) -> Result<(), ServerError> {
        let subscription: SubscriptionInfo = serde_json::from_str(subscription_json)?;

        let signature =
            VapidSignatureBuilder::from_pem(self.vapid_pem.as_slice(), &subscription)?.build()?;

        let payload = serde_json::to_vec(&WebPushPayload { notification, data })?;

        let mut builder = WebPushMessageBuilder::new(&subscription);
        builder.set_payload(ContentEncoding::Aes128Gcm, &payload);
        builder.set_vapid_signature(signature);

        self.client.send(builder.build()?).await?;
        Ok(())
    }
}
