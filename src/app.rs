//! Shared UI shell — the same code drives desktop and mobile.
//!
//! [`App`] owns the runtime handle and the user's database; the UI reads
//! from it.

use std::sync::Arc;

use istmo::Runtime;
use myos_store::Store;

use crate::storage::{self, StorageError};

/// Application state shared by every shell.
#[derive(Debug)]
pub struct App {
    runtime: Arc<Runtime>,
    store: Result<Store, StorageError>,
}

impl App {
    /// Build the app state and open the user's database. A database that
    /// cannot be opened (no credential store, disk failure) is kept as the
    /// error so the UI can explain it instead of crashing.
    #[must_use]
    pub fn new(runtime: Arc<Runtime>) -> Self {
        let store = storage::open();
        if let Err(err) = &store {
            log::error!("cannot open the database: {err}");
        }
        Self { runtime, store }
    }

    /// Runtime the plugin clients are acquired from.
    #[must_use]
    pub const fn runtime(&self) -> &Arc<Runtime> {
        &self.runtime
    }

    /// The user's database, or why it could not be opened.
    pub const fn store(&self) -> &Result<Store, StorageError> {
        &self.store
    }

    /// Mutable access for writes.
    pub const fn store_mut(&mut self) -> &mut Result<Store, StorageError> {
        &mut self.store
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
