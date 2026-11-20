//! Shared UI shell — the same code drives desktop and mobile.
//!
//! [`App`] owns the runtime handle and the user's database; the UI reads
//! from it.

use std::sync::Arc;

use istmo::Runtime;
use myos_store::Store;

use crate::storage::{self, Opened, Reset, StorageError};

/// Application state shared by every shell.
#[derive(Debug)]
pub struct App {
    runtime: Arc<Runtime>,
    store: Result<Store, StorageError>,
    reset: Option<Reset>,
}

impl App {
    /// Build the app state and open the user's database. A database that
    /// cannot be opened (no credential store, disk failure) is kept as the
    /// error so the UI can explain it instead of crashing.
    #[must_use]
    pub fn new(runtime: Arc<Runtime>) -> Self {
        match storage::open() {
            Ok(Opened { store, reset }) => Self {
                runtime,
                store: Ok(store),
                reset,
            },
            Err(err) => {
                let mut chain = err.to_string();
                let mut source = std::error::Error::source(&err);
                while let Some(cause) = source {
                    chain.push_str(": ");
                    chain.push_str(&cause.to_string());
                    source = cause.source();
                }
                log::error!("cannot open the database: {chain}");
                Self {
                    runtime,
                    store: Err(err),
                    reset: None,
                }
            }
        }
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

    /// Set when the previous database had to be discarded (lost key): the UI
    /// offers to restore from the account or an export.
    #[must_use]
    pub const fn reset(&self) -> Option<Reset> {
        self.reset
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
