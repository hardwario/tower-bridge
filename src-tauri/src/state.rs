//! The app's state: the shared service and the tray.

pub use tower_bridge_service::{ApiState, MqttState};
use tower_bridge_service::{Service, ServiceState};

use crate::tray::TrayController;

pub struct AppState {
    pub service: Service,
    pub tray: TrayController,
}

/// The service's state reads as the app's (`state.latest`, `state.settings()`).
impl std::ops::Deref for AppState {
    type Target = ServiceState;

    fn deref(&self) -> &ServiceState {
        &self.service
    }
}
