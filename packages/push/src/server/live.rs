//! Live WebSocket connections held by this instance (for `SelfHosted` fallback devices).

use crate::core::{NotificationContent, RemoteMessage};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;

/// A message frame pushed down a live connection. Serializes to the JSON shape the
/// desktop fallback client expects.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Frame {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notification: Option<NotificationContent>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub data: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
}

impl From<RemoteMessage> for Frame {
    fn from(message: RemoteMessage) -> Self {
        Self {
            notification: message.notification,
            data: message.data,
            message_id: message.message_id,
        }
    }
}

/// Tracks the live connections this instance currently owns. The server's WebSocket
/// accept loop registers a sender per connected device and drains it to the socket.
#[derive(Default)]
pub struct LiveConnections {
    connections: RwLock<HashMap<String, mpsc::UnboundedSender<Frame>>>,
}

impl LiveConnections {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Register a connected device, returning the receiver the accept loop drains.
    pub fn connect(&self, device_id: impl Into<String>) -> mpsc::UnboundedReceiver<Frame> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.connections
            .write()
            .expect("live connections lock poisoned")
            .insert(device_id.into(), tx);
        rx
    }

    /// Drop a device's connection (call when its socket closes).
    pub fn disconnect(&self, device_id: &str) {
        self.connections
            .write()
            .expect("live connections lock poisoned")
            .remove(device_id);
    }

    /// Deliver a frame to a locally-owned connection. Returns `true` if this instance
    /// held the connection and queued the frame, `false` otherwise.
    pub fn deliver(&self, device_id: &str, frame: &Frame) -> bool {
        let connections = self
            .connections
            .read()
            .expect("live connections lock poisoned");
        match connections.get(device_id) {
            Some(tx) => tx.send(frame.clone()).is_ok(),
            None => false,
        }
    }

    /// Whether this instance currently owns the given device's connection.
    pub fn owns(&self, device_id: &str) -> bool {
        self.connections
            .read()
            .expect("live connections lock poisoned")
            .contains_key(device_id)
    }
}
