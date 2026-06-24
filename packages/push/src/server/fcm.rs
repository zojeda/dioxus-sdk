//! FCM HTTP v1 sending client.

use super::ServerError;
use crate::core::NotificationContent;
use gcp_auth::{CustomServiceAccount, TokenProvider};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

const FCM_SCOPE: &str = "https://www.googleapis.com/auth/firebase.messaging";

/// A client for the FCM HTTP v1 API, authenticated with a Google service account.
pub struct FcmClient {
    project_id: String,
    account: CustomServiceAccount,
    http: reqwest::Client,
}

impl FcmClient {
    /// Build a client from a service-account JSON file.
    pub async fn from_service_account_file(path: impl AsRef<Path>) -> Result<Self, ServerError> {
        let json = std::fs::read_to_string(path)?;
        Self::from_service_account_json(&json).await
    }

    /// Build a client from service-account JSON.
    pub async fn from_service_account_json(json: &str) -> Result<Self, ServerError> {
        let account = CustomServiceAccount::from_json(json)
            .map_err(|e| ServerError::Config(format!("invalid service account: {e}")))?;
        let project_id = serde_json::from_str::<serde_json::Value>(json)?
            .get("project_id")
            .and_then(|v| v.as_str())
            .map(String::from)
            .ok_or_else(|| ServerError::Config("service account missing project_id".into()))?;
        Ok(Self {
            project_id,
            account,
            http: reqwest::Client::new(),
        })
    }

    /// Send a message to a single device token.
    pub async fn send(&self, message: &Message) -> Result<FcmResponse, ServerError> {
        let token = self.account.token(&[FCM_SCOPE]).await?;
        let url = format!(
            "https://fcm.googleapis.com/v1/projects/{}/messages:send",
            self.project_id
        );
        let body = serde_json::json!({ "message": message });
        let response = self
            .http
            .post(url)
            .bearer_auth(token.as_str())
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            return Err(ServerError::FcmRejected {
                status: status.as_u16(),
                body: text,
            });
        }
        Ok(serde_json::from_str(&text)?)
    }
}

/// An FCM v1 `message` object.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Message {
    pub token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notification: Option<NotificationContent>,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub data: HashMap<String, String>,
}

impl Message {
    /// Start a message addressed to a device token.
    pub fn to_token(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            ..Default::default()
        }
    }

    /// Set the display notification.
    pub fn notification(mut self, title: impl Into<String>, body: impl Into<String>) -> Self {
        self.notification = Some(NotificationContent {
            title: Some(title.into()),
            body: Some(body.into()),
        });
        self
    }

    /// Add a key/value data entry.
    pub fn data(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.data.insert(key.into(), value.into());
        self
    }
}

/// The successful response from FCM (`{ "name": "projects/.../messages/..." }`).
#[derive(Debug, Clone, Deserialize)]
pub struct FcmResponse {
    pub name: String,
}
