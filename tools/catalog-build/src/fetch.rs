//! Pinned source downloads. Every source is fetched from a fixed URL and,
//! when its content is fixed too, checked against a SHA-256: a changed
//! upstream file fails the build instead of silently changing the catalog.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::BuildError;

/// A source file.
#[derive(Debug)]
pub(crate) struct Pinned {
    /// Cache file name.
    pub(crate) name: &'static str,
    /// Where it comes from.
    pub(crate) url: &'static str,
    /// Expected SHA-256 (hex); `None` for live dumps that change daily.
    pub(crate) sha256: Option<&'static str>,
}

/// free-exercise-db (Unlicense), pinned to a commit. Its images are not
/// used: upstream says they may not be used commercially.
pub(crate) const FREE_EXERCISE_DB: Pinned = Pinned {
    name: "free-exercise-db-f00c92c.json",
    url: "https://raw.githubusercontent.com/yuhonas/free-exercise-db/f00c92c7dcf1216a928a52c3706c7ce8e2f71ed5/dist/exercises.json",
    sha256: Some("5bb747e3fc658f095a60dcbf6d53c96627acdcc6ffb6fffde86f7e26995d40bf"),
};

/// USDA FoodData Central, Foundation Foods (CC0).
pub(crate) const FDC_FOUNDATION: Pinned = Pinned {
    name: "fdc-foundation-2026-10-29.zip",
    url: "https://fdc.nal.usda.gov/fdc-datasets/FoodData_Central_foundation_food_json_2026-10-29.zip",
    sha256: Some("dd414cc9f1a132626bad354ef6655972c78b44aff14f16114dc899708663c579"),
};

/// USDA FoodData Central, SR Legacy (CC0, frozen in 2018).
pub(crate) const FDC_SR_LEGACY: Pinned = Pinned {
    name: "fdc-sr-legacy-2018-04.zip",
    url: "https://fdc.nal.usda.gov/fdc-datasets/FoodData_Central_sr_legacy_food_json_2018-04.zip",
    sha256: Some("0fe8ae486a2c8eb42cb96413f058deb51863a46c8fb8eeb4b1fb45006dd338ef"),
};

/// Open Food Facts full export (ODbL / DbCL), refreshed daily upstream.
pub(crate) const OPEN_FOOD_FACTS: Pinned = Pinned {
    name: "openfoodfacts-products.csv.gz",
    url: "https://static.openfoodfacts.org/data/en.openfoodfacts.org.products.csv.gz",
    sha256: None,
};

fn sha256(path: &Path) -> Result<String, BuildError> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(crate::hex(&hasher.finalize()))
}

/// Path of `source` in `cache`, downloading it first when missing or when
/// the cached copy does not match its pin.
pub(crate) fn cached(cache: &Path, source: &Pinned) -> Result<PathBuf, BuildError> {
    std::fs::create_dir_all(cache)?;
    let path = cache.join(source.name);
    let matches = |path: &Path| -> Result<bool, BuildError> {
        Ok(match source.sha256 {
            Some(expected) => sha256(path)? == expected,
            None => true,
        })
    };
    if path.is_file() && matches(&path)? {
        return Ok(path);
    }
    eprintln!("downloading {}", source.url);
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(3600)))
        .build()
        .into();
    let mut body = agent
        .get(source.url)
        .call()?
        .into_body()
        .into_with_config()
        .limit(u64::MAX)
        .reader();
    let partial = path.with_extension("part");
    let mut out = std::fs::File::create(&partial)?;
    let mut buffer = vec![0_u8; 1 << 16];
    loop {
        let read = body.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        out.write_all(buffer.get(..read).unwrap_or_default())?;
    }
    out.sync_all()?;
    std::fs::rename(&partial, &path)?;
    if !matches(&path)? {
        return Err(BuildError::Invalid(format!(
            "{} changed upstream (SHA-256 mismatch): review it and update the pin",
            source.url
        )));
    }
    Ok(path)
}
