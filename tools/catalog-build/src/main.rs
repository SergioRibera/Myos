//! `catalog-build`: builds the reference files the app ships and updates
//! from, then signs the manifest that lists them. Runs in CI.
//!
//! ```text
//! catalog-build catalog    --out DIR --version N [--cache DIR]
//! catalog-build food-facts --out DIR --version N --countries bo,ar [--input FILE.csv.gz] [--cache DIR]
//! catalog-build names      --catalog FILE --out FILE  (English names, input of translate.py)
//! catalog-build manifest   --dir DIR           (signs with $CATALOG_SIGNING_KEY)
//! catalog-build keygen                          (prints a new seed and its public key)
//! ```

mod catalog;
mod fetch;
mod food_facts;
mod translations;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use myos_catalog_update::{Kind, Manifest, UpdateError};

/// Tool failure.
#[derive(Debug, thiserror::Error)]
enum BuildError {
    #[error("usage: {0}")]
    Usage(&'static str),
    #[error("{0}")]
    Invalid(String),
    #[error("database error")]
    Sqlite(#[from] rusqlite::Error),
    #[error("file operation failed")]
    Io(#[from] std::io::Error),
    #[error("download failed")]
    Http(#[from] ureq::Error),
    #[error("malformed source data")]
    Json(#[from] serde_json::Error),
    #[error("malformed archive")]
    Zip(#[from] zip::result::ZipError),
    #[error("malformed CSV")]
    Csv(#[from] csv::Error),
    #[error(transparent)]
    Manifest(#[from] UpdateError),
    #[error("system randomness unavailable")]
    Random(getrandom::Error),
}

/// `--flag value` pairs after the subcommand.
struct Args(HashMap<String, String>);

impl Args {
    fn parse(mut raw: impl Iterator<Item = String>) -> Result<Self, BuildError> {
        let mut map = HashMap::new();

        while let Some(flag) = raw.next() {
            let name = flag
                .strip_prefix("--")
                .ok_or_else(|| BuildError::Invalid(format!("unexpected argument {flag}")))?;
            let value = raw
                .next()
                .ok_or_else(|| BuildError::Invalid(format!("--{name} needs a value")))?;
            map.insert(name.to_owned(), value);
        }
        Ok(Self(map))
    }

    fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }

    fn path(&self, name: &str, usage: &'static str) -> Result<PathBuf, BuildError> {
        self.get(name)
            .map(PathBuf::from)
            .ok_or(BuildError::Usage(usage))
    }

    fn version(&self, usage: &'static str) -> Result<u32, BuildError> {
        self.get("version")
            .and_then(|value| value.parse().ok())
            .ok_or(BuildError::Usage(usage))
    }

    fn cache(&self) -> PathBuf {
        self.get("cache").map_or_else(
            || std::env::temp_dir().join("myos-catalog-cache"),
            PathBuf::from,
        )
    }
}

const CATALOG_USAGE: &str = "catalog --out DIR --version N [--cache DIR] [--translations DIR]";
const FOOD_FACTS_USAGE: &str =
    "food-facts --out DIR --version N --countries bo,ar [--input FILE.csv.gz] [--cache DIR]";
const NAMES_USAGE: &str = "names --catalog catalog.db --out names.tsv";
const MANIFEST_USAGE: &str = "manifest --dir DIR (CATALOG_SIGNING_KEY = hex seed)";

/// Write the manifest of every reference file in `dir` and sign it.
fn manifest(dir: &Path) -> Result<(), BuildError> {
    let mut manifest = Manifest::default();
    let mut names: Vec<String> = std::fs::read_dir(dir)?
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| Path::new(name).extension().is_some_and(|ext| ext == "db"))
        .collect();
    names.sort();
    for name in names {
        let path = dir.join(&name);
        let (kind, table) = if name == "catalog.db" {
            (Kind::Catalog, "catalog_meta")
        } else if let Some(country) = name
            .strip_prefix("off-")
            .and_then(|rest| rest.strip_suffix(".db"))
            .filter(|country| myos_catalog_update::valid_country(country))
        {
            (Kind::FoodFacts(country.to_owned()), "meta")
        } else {
            continue;
        };
        let schema = catalog::read_meta(&path, table, "schema_version")?;
        let version = catalog::read_meta(&path, table, "version")?;
        manifest
            .entries
            .push(myos_catalog_update::describe(kind, &path, schema, version)?);
    }
    let text = manifest.render();
    std::fs::write(dir.join("manifest"), &text)?;
    match std::env::var("CATALOG_SIGNING_KEY") {
        Ok(seed) => {
            let signature = myos_catalog_update::sign(&text, seed.trim())?;
            std::fs::write(dir.join("manifest.sig"), signature)?;
            eprintln!(
                "signed with public key {}",
                myos_catalog_update::public_key(seed.trim())?
            );
        }
        Err(_) => eprintln!("CATALOG_SIGNING_KEY unset: manifest left unsigned"),
    }
    print!("{text}");
    Ok(())
}

/// Lowercase hex of `bytes`.
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut text, byte| {
        let _infallible = write!(text, "{byte:02x}");
        text
    })
}

fn keygen() -> Result<(), BuildError> {
    let mut seed = [0_u8; 32];
    getrandom::fill(&mut seed).map_err(BuildError::Random)?;
    let seed = hex(&seed);
    println!("CATALOG_SIGNING_KEY (secret) = {seed}");
    println!(
        "public key (app)             = {}",
        myos_catalog_update::public_key(&seed)?
    );
    Ok(())
}

fn run() -> Result<(), BuildError> {
    let mut raw = std::env::args().skip(1);
    let command = raw.next().ok_or(BuildError::Usage(
        "catalog | food-facts | names | manifest | keygen",
    ))?;
    let args = Args::parse(raw)?;
    match command.as_str() {
        "catalog" => {
            let out = args.path("out", CATALOG_USAGE)?;
            std::fs::create_dir_all(&out)?;
            let translations = args.get("translations").map_or_else(
                || Path::new(env!("CARGO_MANIFEST_DIR")).join("translations"),
                PathBuf::from,
            );
            catalog::build(
                &out.join("catalog.db"),
                args.version(CATALOG_USAGE)?,
                &args.cache(),
                &translations,
            )
        }
        "food-facts" => {
            let out = args.path("out", FOOD_FACTS_USAGE)?;
            std::fs::create_dir_all(&out)?;
            let countries: Vec<String> = args
                .get("countries")
                .ok_or(BuildError::Usage(FOOD_FACTS_USAGE))?
                .split(',')
                .map(|country| country.trim().to_ascii_lowercase())
                .filter(|country| !country.is_empty())
                .collect();
            let input = match args.get("input") {
                Some(path) => PathBuf::from(path),
                None => fetch::cached(&args.cache(), &fetch::OPEN_FOOD_FACTS)?,
            };
            food_facts::build(&input, &out, &countries, args.version(FOOD_FACTS_USAGE)?)
        }
        "manifest" => manifest(&args.path("dir", MANIFEST_USAGE)?),
        "names" => translations::names(
            &args.path("catalog", NAMES_USAGE)?,
            &args.path("out", NAMES_USAGE)?,
        ),
        "keygen" => keygen(),
        _ => Err(BuildError::Usage(
            "catalog | food-facts | names | manifest | keygen",
        )),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let mut chain = error.to_string();
            let mut source = std::error::Error::source(&error);
            while let Some(cause) = source {
                chain.push_str(": ");
                chain.push_str(&cause.to_string());
                source = cause.source();
            }
            eprintln!("catalog-build: {chain}");
            ExitCode::FAILURE
        }
    }
}
