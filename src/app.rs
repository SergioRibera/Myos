//! Shared UI shell — the same code drives desktop and mobile.
//!
//! [`App`] owns the runtime handle, and later your plugin clients
//! (`DataStoreClient::from_runtime_with(&runtime, config)`, …); the UI only
//! reads from it.

use std::sync::Arc;

use istmo::Runtime;

/// Application state shared by every shell.
#[derive(Debug)]
pub struct App {
    runtime: Arc<Runtime>,
}

impl App {
    #[must_use]
    pub const fn new(runtime: Arc<Runtime>) -> Self {
        Self { runtime }
    }

    /// Runtime the plugin clients are acquired from.
    #[must_use]
    pub const fn runtime(&self) -> &Arc<Runtime> {
        &self.runtime
    }

    /// Desktop entry point of the `freya` shell.
    pub fn run_desktop(self) {
        // TODO: launch the freya UI here.
        log::info!("myos desktop shell started: {self:?}");
    }

    /// Mobile entry point of the `freya` shell.
    pub fn run_mobile(self) {
        // TODO: launch the freya UI here.
        log::info!("myos mobile shell started: {self:?}");
    }
}
