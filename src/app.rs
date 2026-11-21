//! Shared UI shell — the same code drives desktop and mobile.
//!
//! [`App`] owns the runtime handle, the user's database with its reference
//! files attached, and the background workers (sync, catalog updates). The
//! UI reads from it and calls [`App::poll`] on each frame or tick.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use istmo::Runtime;
use myos_store::Store;
use myos_store::catalog::{Catalog, ReferenceInfo};

use crate::reference::{self, Installed, Ready};
use crate::storage::{self, Opened, Reset, StorageError};
use crate::sync::{Account, SyncState, Syncer};

/// Application state shared by every shell.
#[derive(Debug)]
pub struct App {
    runtime: Arc<Runtime>,
    store: Result<Store, StorageError>,
    reset: Option<Reset>,
    catalog: Option<ReferenceInfo>,
    food_facts: Option<(String, Option<ReferenceInfo>)>,
    updates: Option<Receiver<Vec<Ready>>>,
    syncer: Option<Syncer>,
}

fn chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

impl App {
    /// Build the app state: open the user's database, install and attach
    /// the catalog, start sync when an account is signed in and look for
    /// catalog updates. A database that cannot be opened (no credential
    /// store, disk failure) is kept as the error so the UI can explain it
    /// instead of crashing.
    #[must_use]
    pub fn new(runtime: Arc<Runtime>) -> Self {
        let mut app = match storage::open() {
            Ok(Opened { store, reset }) => Self {
                runtime,
                store: Ok(store),
                reset,
                catalog: None,
                food_facts: None,
                updates: None,
                syncer: None,
            },
            Err(err) => {
                log::error!("cannot open the database: {}", chain(&err));
                return Self {
                    runtime,
                    store: Err(err),
                    reset: None,
                    catalog: None,
                    food_facts: None,
                    updates: None,
                    syncer: None,
                };
            }
        };
        if let Ok(store) = &mut app.store {
            match reference::install_catalog(store) {
                Ok(info) => app.catalog = info,
                Err(err) => log::warn!("catalog unavailable: {}", chain(&err)),
            }
        }
        app.resume_sync();
        app.check_for_updates();
        app
    }

    /// Start syncing the stored (or development) account, if any.
    fn resume_sync(&mut self) {
        let account = match Account::load() {
            Ok(Some(account)) => Some(account),
            Ok(None) => Account::from_env(),
            Err(err) => {
                log::warn!("cannot read the sync account: {}", chain(&err));
                None
            }
        };
        if let Some(account) = account
            && let Err(err) = self.start_sync(&account)
        {
            log::warn!("sync not started: {}", chain(&err));
        }
    }

    fn start_sync(&mut self, account: &Account) -> Result<(), StorageError> {
        let store = self
            .store
            .as_mut()
            .map_err(|_| StorageError::Store(myos_store::Error::NotFound("database")))?;
        store.attach_account(account.user, &account.key)?;
        self.syncer = Some(
            Syncer::start(account).map_err(|source| StorageError::Directory {
                path: "myos-sync thread".into(),
                source,
            })?,
        );
        Ok(())
    }

    /// Sign in to sync: attach the account to the database, remember it and
    /// start syncing. Fails with `AccountMismatch` when the database belongs
    /// to another account (log out first).
    pub fn sign_in(&mut self, account: &Account) -> Result<(), StorageError> {
        self.start_sync(account)?;
        account.save()
    }

    /// Stop syncing and forget the account (local data stays).
    pub fn stop_sync(&mut self) -> Result<(), StorageError> {
        self.syncer = None;
        Account::forget()
    }

    /// Sync now (after a write, on resume).
    pub fn request_sync(&self) {
        if let Some(syncer) = &self.syncer {
            syncer.request();
        }
    }

    /// Sync status; `None` when no account is signed in.
    #[must_use]
    pub fn sync_state(&self) -> Option<SyncState> {
        self.syncer.as_ref().map(Syncer::state)
    }

    /// Use Open Food Facts products of `country` (ISO 3166-1 alpha-2,
    /// lowercase) for barcode lookups: attach them when already downloaded
    /// and fetch updates for them from now on.
    pub fn set_food_facts_country(&mut self, country: &str) {
        if !myos_catalog_update::valid_country(country) {
            log::warn!("not a country code: {country:?}");
            return;
        }
        let info = self.store.as_mut().ok().and_then(|store| {
            reference::attach_food_facts(store, country).unwrap_or_else(|err| {
                log::warn!("food facts of {country} unavailable: {}", chain(&err));
                None
            })
        });
        self.food_facts = Some((country.to_owned(), info));
        self.check_for_updates();
    }

    /// Look for newer reference files in the background.
    pub fn check_for_updates(&mut self) {
        if self.store.is_err() || self.updates.is_some() {
            return;
        }
        let installed = Installed {
            catalog: self.catalog.map(|info| info.version),
            food_facts: self
                .food_facts
                .as_ref()
                .map(|(country, info)| (country.clone(), info.map(|info| info.version))),
        };
        let (done, updates) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("myos-catalog-update".into())
            .spawn(move || {
                if let Some(ready) = reference::check_for_updates(&installed) {
                    let _gone = done.send(ready);
                }
            });
        match spawned {
            Ok(_) => self.updates = Some(updates),
            Err(err) => log::warn!("cannot start the catalog update check: {err}"),
        }
    }

    /// Apply finished background work (downloaded reference files). Call it
    /// from the UI loop; cheap when nothing is pending.
    pub fn poll(&mut self) {
        let Some(updates) = &self.updates else {
            return;
        };
        let ready = match updates.try_recv() {
            Ok(ready) => ready,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Vec::new(),
        };
        self.updates = None;
        let Ok(store) = &mut self.store else {
            return;
        };
        for file in ready {
            let reference = file.reference;
            match file.apply(store) {
                Ok(info) => match reference {
                    myos_store::catalog::Reference::Catalog => self.catalog = Some(info),
                    myos_store::catalog::Reference::FoodFacts => {
                        if let Some((_, slot)) = &mut self.food_facts {
                            *slot = Some(info);
                        }
                    }
                },
                Err(err) => log::warn!("reference update failed: {}", chain(&err)),
            }
        }
    }

    /// Catalog exercises for the load engine (empty without a catalog).
    pub fn catalog(&self) -> Result<Catalog, myos_store::Error> {
        self.store
            .as_ref()
            .map_or_else(|_| Ok(Catalog::default()), Store::catalog)
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
