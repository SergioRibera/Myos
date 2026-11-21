//! Reference files: the exercise / food catalog and Open Food Facts
//! products of one country.
//!
//! Release builds bundle the newest `catalog.db` and its manifest as app
//! assets (`assets/catalog/`, filled by CI); on start the bundled copy is
//! installed when it is newer than the one on disk. Newer versions are
//! published by the catalog pipeline with a signed manifest: a background
//! thread downloads them, checks size and SHA-256 against the manifest and
//! hands them to the app, which swaps them in.

use std::io::Read;
use std::path::{Path, PathBuf};

use myos_catalog_update::{Entry, Manifest, Source};
use myos_store::Store;
use myos_store::catalog::{Reference, ReferenceInfo};

/// Where the catalog pipeline publishes.
const PUBLISH_URL: &str = "https://github.com/SergioRibera/myos/releases/download/catalog";
/// Public key of the catalog signing key (`catalog-build keygen`). Empty:
/// remote updates are off and only the bundled catalog is used.
const PUBLIC_KEY: &str = "";

/// Directory holding the reference files.
#[must_use]
pub fn reference_dir() -> PathBuf {
    crate::storage::app_dir().join("reference")
}

/// File name of a food-facts database.
fn food_facts_file(country: &str) -> String {
    format!("off-{country}.db")
}

/// Why a reference file could not be installed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ReferenceError {
    /// A file operation failed.
    #[error("reference file operation failed on {path}")]
    Io {
        /// File involved.
        path: PathBuf,
        /// Underlying failure.
        #[source]
        source: std::io::Error,
    },
    /// The store refused the file.
    #[error(transparent)]
    Store(#[from] myos_store::Error),
    /// Downloading or verifying failed.
    #[error(transparent)]
    Update(#[from] myos_catalog_update::UpdateError),
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> ReferenceError + '_ {
    move |source| ReferenceError::Io {
        path: path.to_owned(),
        source,
    }
}

/// The manifest bundled with the app, if this build carries one.
fn bundled_manifest() -> Option<Manifest> {
    let mut text = String::new();
    istmo::assets::open("catalog/manifest")
        .ok()?
        .read_to_string(&mut text)
        .ok()?;
    match Manifest::parse(&text) {
        Ok(manifest) => Some(manifest),
        Err(err) => {
            log::warn!("bundled catalog manifest is malformed: {err}");
            None
        }
    }
}

/// Attach `catalog.db`, installing the bundled copy first when it is newer
/// than the one on disk (or there is none). `None`: no catalog anywhere.
pub fn install_catalog(store: &mut Store) -> Result<Option<ReferenceInfo>, ReferenceError> {
    let dir = reference_dir();
    std::fs::create_dir_all(&dir).map_err(io(&dir))?;
    let current = dir.join("catalog.db");
    let installed = if current.is_file() {
        match store.attach_reference(Reference::Catalog, &current) {
            Ok(info) => Some(info),
            Err(err) => {
                log::warn!("installed catalog is unreadable, replacing it: {err}");
                None
            }
        }
    } else {
        None
    };
    let bundled = bundled_manifest().and_then(|manifest| manifest.catalog().cloned());
    match bundled {
        Some(entry) if installed.is_none_or(|info| entry.version > info.version) => {
            let next = dir.join("catalog.next.db");
            let mut asset = istmo::assets::open(format!("catalog/{}", entry.file))
                .map_err(io(Path::new("catalog.db (asset)")))?;
            let mut out = std::fs::File::create(&next).map_err(io(&next))?;
            std::io::copy(&mut asset, &mut out).map_err(io(&next))?;
            out.sync_all().map_err(io(&next))?;
            Ok(Some(store.replace_reference(
                Reference::Catalog,
                &next,
                &current,
            )?))
        }
        _ => Ok(installed),
    }
}

/// Attach the food-facts file of `country` when it was downloaded before.
pub fn attach_food_facts(
    store: &mut Store,
    country: &str,
) -> Result<Option<ReferenceInfo>, ReferenceError> {
    let path = reference_dir().join(food_facts_file(country));
    if !path.is_file() {
        return Ok(None);
    }
    Ok(Some(store.attach_reference(Reference::FoodFacts, &path)?))
}

/// A downloaded file ready to be swapped in.
#[derive(Debug)]
pub struct Ready {
    /// Which file.
    pub reference: Reference,
    /// Downloaded copy.
    pub next: PathBuf,
    /// Where it goes.
    pub current: PathBuf,
}

impl Ready {
    /// Swap the file in on the app's store.
    pub fn apply(self, store: &mut Store) -> Result<ReferenceInfo, ReferenceError> {
        Ok(store.replace_reference(self.reference, &self.next, &self.current)?)
    }
}

/// Installed versions the updater compares against.
#[derive(Debug, Clone, Default)]
pub struct Installed {
    /// `catalog.db` version.
    pub catalog: Option<u32>,
    /// Food-facts country and version.
    pub food_facts: Option<(String, Option<u32>)>,
}

fn newer(entry: Option<&Entry>, installed: Option<u32>) -> Option<&Entry> {
    entry.filter(|entry| installed.is_none_or(|version| entry.version > version))
}

/// Check the published manifest once and download what is newer than
/// `installed`. Runs on a background thread; `None` when updates are off.
#[must_use]
pub fn check_for_updates(installed: &Installed) -> Option<Vec<Ready>> {
    let Ok(key) = myos_catalog_update::verifying_key(PUBLIC_KEY) else {
        return None;
    };
    let source = Source::new(PUBLISH_URL, key);
    let manifest = match source.manifest() {
        Ok(manifest) => manifest,
        Err(err) => {
            log::warn!("catalog update check failed: {err}");
            return Some(Vec::new());
        }
    };
    let dir = reference_dir();
    let mut wanted = Vec::new();
    if let Some(entry) = newer(manifest.catalog(), installed.catalog) {
        wanted.push((Reference::Catalog, entry.clone(), dir.join("catalog.db")));
    }
    if let Some((country, version)) = &installed.food_facts
        && let Some(entry) = newer(manifest.food_facts(country), *version)
    {
        wanted.push((
            Reference::FoodFacts,
            entry.clone(),
            dir.join(food_facts_file(country)),
        ));
    }
    let mut ready = Vec::new();
    for (reference, entry, current) in wanted {
        let next = current.with_extension("next.db");
        match source.download(&entry, &next) {
            Ok(()) => ready.push(Ready {
                reference,
                next,
                current,
            }),
            Err(err) => log::warn!("download of {} failed: {err}", entry.file),
        }
    }
    Some(ready)
}
