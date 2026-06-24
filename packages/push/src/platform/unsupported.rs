//! Fallback backend for targets without any push implementation.

use crate::core::{Error, PermissionStatus, PushConfig, PushEvent};
use std::sync::Arc;

pub struct PushManager;

impl PushManager {
    pub fn new(_config: PushConfig) -> Result<Self, Error> {
        Err(Error::Unsupported)
    }
}

pub async fn request_permission(_manager: &PushManager) -> Result<PermissionStatus, Error> {
    Err(Error::Unsupported)
}

pub fn register(_manager: &PushManager) -> Result<(), Error> {
    Err(Error::Unsupported)
}

pub fn listen(_callback: Arc<dyn Fn(PushEvent) + Send + Sync>) -> Result<(), Error> {
    Err(Error::Unsupported)
}
