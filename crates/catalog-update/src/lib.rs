//! `myos-catalog-update`: the signed manifest of the reference files
//! (`catalog.db`, `off-<cc>.db`) and their verified download.
//!
//! The catalog pipeline publishes, next to the files, a `manifest` and its
//! ed25519 signature `manifest.sig` (hex). Clients trust a manifest only
//! when the signature checks against the public key built into the app,
//! then trust each file only when its size and SHA-256 match the manifest.
//!
//! Manifest format (UTF-8 lines, single spaces):
//!
//! ```text
//! myos-catalog-manifest 1
//! catalog catalog.db <schema> <version> <size> <sha256>
//! food-facts <country> off-<country>.db <schema> <version> <size> <sha256>
//! ```
//!
//! `schema` is the file's `schema_version`: clients skip entries whose
//! schema they cannot read.
//!
//! ```text
//! ```

use std::fmt::Write as _;
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};

/// First line of every manifest.
const HEADER: &str = "myos-catalog-manifest 1";
/// Largest manifest accepted.
const MANIFEST_LIMIT: u64 = 64 * 1024;
/// Largest reference file accepted.
const FILE_LIMIT: u64 = 2 * 1024 * 1024 * 1024;

/// Update failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum UpdateError {
    /// The manifest is malformed.
    #[error("malformed manifest line {line}")]
    Manifest {
        /// 1-based line number.
        line: usize,
    },
    /// The signature does not match the manifest and the built-in key.
    #[error("manifest signature does not verify")]
    Signature,
    /// A downloaded file does not match its manifest entry.
    #[error("{file} does not match the manifest")]
    Mismatch {
        /// File name.
        file: String,
    },
    /// The server answered with an error status.
    #[error("server answered {status} for {url}")]
    Status {
        /// HTTP status.
        status: u16,
        /// Requested URL.
        url: String,
    },
    /// The request failed.
    #[error("request failed")]
    Request(#[from] ureq::Error),
    /// A local file operation failed.
    #[error("file operation failed")]
    Io(#[from] std::io::Error),
    /// A key or signature is not valid hex of the right length.
    #[error("malformed key or signature")]
    Hex,
}

/// Which reference file an entry describes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// `catalog.db`.
    Catalog,
    /// `off-<country>.db` (ISO 3166-1 alpha-2, lowercase).
    FoodFacts(String),
}

/// One published file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// What it is.
    pub kind: Kind,
    /// File name next to the manifest.
    pub file: String,
    /// Schema of the file (its meta `schema_version`).
    pub schema: u32,
    /// Data version (higher is newer).
    pub version: u32,
    /// Size in bytes.
    pub size: u64,
    /// SHA-256 of the file.
    pub sha256: [u8; 32],
}

/// Every published file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Manifest {
    /// Entries, in file order.
    pub entries: Vec<Entry>,
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut text, byte| {
            let _infallible = write!(text, "{byte:02x}");
            text
        })
}

fn unhex<const N: usize>(text: &str) -> Result<[u8; N], UpdateError> {
    let text = text.trim();
    if text.len() != N * 2 {
        return Err(UpdateError::Hex);
    }
    let mut out = [0_u8; N];
    for (slot, pair) in out.iter_mut().zip(text.as_bytes().chunks_exact(2)) {
        let pair = std::str::from_utf8(pair).map_err(|_| UpdateError::Hex)?;
        *slot = u8::from_str_radix(pair, 16).map_err(|_| UpdateError::Hex)?;
    }
    Ok(out)
}

/// A country code as used in file names: two ASCII lowercase letters.
#[must_use]
pub fn valid_country(country: &str) -> bool {
    country.len() == 2 && country.bytes().all(|byte| byte.is_ascii_lowercase())
}

/// A file name the manifest may point at (no paths).
fn valid_file(file: &str) -> bool {
    !file.is_empty()
        && file.len() <= 64
        && file
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
        && !file.starts_with('.')
}

impl Manifest {
    /// Parse manifest text (signature not checked: see [`Manifest::verify`]).
    pub fn parse(text: &str) -> Result<Self, UpdateError> {
        let mut lines = text.lines().enumerate();
        if lines.next().map(|(_, line)| line) != Some(HEADER) {
            return Err(UpdateError::Manifest { line: 1 });
        }
        let mut entries = Vec::new();
        for (index, line) in lines {
            if line.is_empty() {
                continue;
            }
            let bad = || UpdateError::Manifest { line: index + 1 };
            let fields: Vec<&str> = line.split(' ').collect();
            let (kind, rest) = match fields.as_slice() {
                ["catalog", rest @ ..] => (Kind::Catalog, rest),
                ["food-facts", country, rest @ ..] if valid_country(country) => {
                    (Kind::FoodFacts((*country).to_owned()), rest)
                }
                _ => return Err(bad()),
            };
            let [file, schema, version, size, sha256] = rest else {
                return Err(bad());
            };
            if !valid_file(file) {
                return Err(bad());
            }
            entries.push(Entry {
                kind,
                file: (*file).to_owned(),
                schema: schema.parse().map_err(|_| bad())?,
                version: version.parse().map_err(|_| bad())?,
                size: size.parse().map_err(|_| bad())?,
                sha256: unhex(sha256).map_err(|_| bad())?,
            });
        }
        Ok(Self { entries })
    }

    /// Parse `text` after checking `signature` (hex) against `key`.
    pub fn verify(text: &str, signature: &str, key: &VerifyingKey) -> Result<Self, UpdateError> {
        let signature = Signature::from_bytes(&unhex::<64>(signature)?);
        key.verify_strict(text.as_bytes(), &signature)
            .map_err(|_| UpdateError::Signature)?;
        Self::parse(text)
    }

    /// Manifest text.
    #[must_use]
    pub fn render(&self) -> String {
        let mut text = format!("{HEADER}\n");
        for entry in &self.entries {
            let kind = match &entry.kind {
                Kind::Catalog => "catalog".to_owned(),
                Kind::FoodFacts(country) => format!("food-facts {country}"),
            };
            let _infallible = writeln!(
                text,
                "{kind} {} {} {} {} {}",
                entry.file,
                entry.schema,
                entry.version,
                entry.size,
                hex(&entry.sha256)
            );
        }
        text
    }

    /// The catalog entry.
    #[must_use]
    pub fn catalog(&self) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|entry| entry.kind == Kind::Catalog)
    }

    /// The food-facts entry of `country`.
    #[must_use]
    pub fn food_facts(&self, country: &str) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|entry| matches!(&entry.kind, Kind::FoodFacts(code) if code == country))
    }
}

/// Entry describing the file at `path`.
pub fn describe(kind: Kind, path: &Path, schema: u32, version: u32) -> Result<Entry, UpdateError> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let size = std::io::copy(&mut file, &mut hasher)?;
    Ok(Entry {
        kind,
        file: path
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| valid_file(name))
            .ok_or(UpdateError::Hex)?
            .to_owned(),
        schema,
        version,
        size,
        sha256: hasher.finalize().into(),
    })
}

/// Sign manifest text with a 32-byte seed (hex); returns the signature hex.
pub fn sign(text: &str, seed: &str) -> Result<String, UpdateError> {
    let key = SigningKey::from_bytes(&unhex::<32>(seed)?);
    Ok(hex(&key.sign(text.as_bytes()).to_bytes()))
}

/// Public key (hex) of a 32-byte seed (hex).
pub fn public_key(seed: &str) -> Result<String, UpdateError> {
    Ok(hex(SigningKey::from_bytes(&unhex::<32>(seed)?)
        .verifying_key()
        .as_bytes()))
}

/// Parse a public key (hex).
pub fn verifying_key(text: &str) -> Result<VerifyingKey, UpdateError> {
    VerifyingKey::from_bytes(&unhex::<32>(text)?).map_err(|_| UpdateError::Hex)
}

/// Where the files are published, and the key that signs them.
#[derive(Debug)]
pub struct Source {
    agent: ureq::Agent,
    base: String,
    key: VerifyingKey,
}

impl Source {
    /// Files under `base` (an `https://` directory URL), signed by `key`.
    #[must_use]
    pub fn new(base: &str, key: VerifyingKey) -> Self {
        Self::with_scheme(base, key, true)
    }

    fn with_scheme(base: &str, key: VerifyingKey, https_only: bool) -> Self {
        // Per-read timeout instead of a global one: a large file on a slow
        // link may take long as long as bytes keep coming.
        let config = ureq::Agent::config_builder()
            .https_only(https_only)
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_per_read(Some(Duration::from_secs(60)))
            .http_status_as_error(false)
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
            base: base.trim_end_matches('/').to_owned(),
            key,
        }
    }

    fn get(&self, file: &str) -> Result<ureq::Body, UpdateError> {
        let url = format!("{}/{file}", self.base);
        let response = self.agent.get(&url).call()?;
        match response.status().as_u16() {
            200 => Ok(response.into_body()),
            status => Err(UpdateError::Status { status, url }),
        }
    }

    fn text(&self, file: &str) -> Result<String, UpdateError> {
        Ok(self
            .get(file)?
            .into_with_config()
            .limit(MANIFEST_LIMIT)
            .read_to_string()?)
    }

    /// The published manifest, signature checked.
    pub fn manifest(&self) -> Result<Manifest, UpdateError> {
        let text = self.text("manifest")?;
        let signature = self.text("manifest.sig")?;
        Manifest::verify(&text, &signature, &self.key)
    }

    /// Download `entry` to `dest`, replaced atomically once size and hash
    /// match: a partial or forged download never lands there.
    ///
    /// Interrupted downloads resume: the bytes received so far stay in
    /// `<dest>.download` (with `<dest>.download-sha256` naming the entry they belong
    /// to) and the next call asks only for the rest (`Range`). The hash is
    /// always taken over the whole file before it is accepted.
    pub fn download(&self, entry: &Entry, dest: &Path) -> Result<(), UpdateError> {
        if entry.size > FILE_LIMIT {
            return Err(UpdateError::Mismatch {
                file: entry.file.clone(),
            });
        }
        let partial = dest.with_extension("download");
        let marker = dest.with_extension("download-sha256");
        let expected = hex(&entry.sha256);
        // A partial of another entry (older version) or longer than this
        // one is useless.
        let mut have = match std::fs::read_to_string(&marker) {
            Ok(owner) if owner == expected => std::fs::metadata(&partial)
                .map(|meta| meta.len())
                .ok()
                .filter(|len| *len <= entry.size)
                .unwrap_or(0),
            _ => 0,
        };
        if have == 0 {
            std::fs::write(&marker, &expected)?;
            std::fs::File::create(&partial)?;
        }
        if have < entry.size {
            let url = format!("{}/{}", self.base, entry.file);
            let mut request = self.agent.get(&url);
            if have > 0 {
                request = request.header("Range", format!("bytes={have}-"));
            }
            let response = request.call()?;
            let resumed = response.status().as_u16() == 206
                && response
                    .headers()
                    .get("content-range")
                    .and_then(|value| value.to_str().ok())
                    .is_some_and(|range| range.starts_with(&format!("bytes {have}-")));
            let mut out = match response.status().as_u16() {
                206 if resumed => std::fs::OpenOptions::new().append(true).open(&partial)?,
                // The server ignored the range (or there was none): start over.
                200 => {
                    have = 0;
                    std::fs::File::create(&partial)?
                }
                status => return Err(UpdateError::Status { status, url }),
            };
            let mut body = response
                .into_body()
                .into_with_config()
                .limit(entry.size - have)
                .reader();
            let mut buffer = vec![0_u8; 64 * 1024];
            loop {
                // A dropped connection leaves what arrived for the next call.
                let read = body.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                out.write_all(buffer.get(..read).unwrap_or_default())?;
            }
            out.sync_all()?;
        }
        let file = std::fs::File::open(&partial)?;
        let result = install(file, entry, dest);
        // Complete or not, these bytes are spent: a mismatch starts over.
        let _removed = std::fs::remove_file(&partial);
        let _removed = std::fs::remove_file(&marker);
        result
    }
}

/// Copy `source` to `dest` when its size and SHA-256 match `entry`
/// (written to a `.part` file first; a mismatch never lands at `dest`).
pub fn install(mut source: impl Read, entry: &Entry, dest: &Path) -> Result<(), UpdateError> {
    let partial = dest.with_extension("part");
    let result = (|| {
        let mut body = source.by_ref().take(entry.size.saturating_add(1));
        let mut out = std::fs::File::create(&partial)?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0_u8; 64 * 1024];
        let mut size = 0_u64;
        loop {
            let read = body.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            let chunk = buffer.get(..read).unwrap_or_default();
            hasher.update(chunk);
            out.write_all(chunk)?;
            size += read as u64;
        }
        out.sync_all()?;
        let digest: [u8; 32] = hasher.finalize().into();
        if size != entry.size || digest != entry.sha256 {
            return Err(UpdateError::Mismatch {
                file: entry.file.clone(),
            });
        }
        std::fs::rename(&partial, dest)?;
        Ok(())
    })();
    if result.is_err() {
        let _removed = std::fs::remove_file(&partial);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: &str = "0101010101010101010101010101010101010101010101010101010101010101";

    fn manifest() -> Manifest {
        Manifest {
            entries: vec![
                Entry {
                    kind: Kind::Catalog,
                    file: "catalog.db".into(),
                    schema: 1,
                    version: 2_026_112_114,
                    size: 10,
                    sha256: [7; 32],
                },
                Entry {
                    kind: Kind::FoodFacts("bo".into()),
                    file: "off-bo.db".into(),
                    schema: 1,
                    version: 3,
                    size: 20,
                    sha256: [9; 32],
                },
            ],
        }
    }

    #[test]
    fn manifests_round_trip_and_verify() -> Result<(), UpdateError> {
        let text = manifest().render();
        assert_eq!(Manifest::parse(&text)?, manifest());
        let signature = sign(&text, SEED)?;
        let key = verifying_key(&public_key(SEED)?)?;
        let verified = Manifest::verify(&text, &signature, &key)?;
        assert_eq!(verified.food_facts("bo").map(|entry| entry.size), Some(20));
        let forged = text.replace("off-bo.db 1 3", "off-bo.db 1 4");
        assert!(matches!(
            Manifest::verify(&forged, &signature, &key),
            Err(UpdateError::Signature)
        ));
        Ok(())
    }

    #[test]
    fn installs_only_matching_files() -> Result<(), UpdateError> {
        let dir = std::env::temp_dir().join(format!("myos-install-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let data = b"reference data";
        let entry = Entry {
            kind: Kind::Catalog,
            file: "catalog.db".into(),
            schema: 1,
            version: 1,
            size: data.len() as u64,
            sha256: Sha256::digest(data).into(),
        };
        let dest = dir.join("catalog.db");
        install(&data[..], &entry, &dest)?;
        assert_eq!(std::fs::read(&dest)?, data);
        let tampered = dir.join("tampered.db");
        assert!(install(&b"reference datA"[..], &entry, &tampered).is_err());
        assert!(install(&b"reference data and more"[..], &entry, &tampered).is_err());
        assert!(!tampered.exists() && !tampered.with_extension("part").exists());
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn signatures_made_by_the_openssl_cli_verify() -> Result<(), UpdateError> {
        // Signed by `.github/scripts/catalog-sig.sh sign` with seed SEED.
        let text = "myos-catalog-manifest 1\ncatalog catalog.db 1 7 10 \
                    0707070707070707070707070707070707070707070707070707070707070707\n";
        let signature = "5f10e1701ece0118e2ead2aa4b97cf6fe855cafb5a9a3bff3b6bef7133bc7a8f\
                         806b152991fb7d28b452da4d3c80ffa2acb63de118b0bf4c88452c8a03af8402";
        let key = verifying_key(&public_key(SEED)?)?;
        assert_eq!(
            Manifest::verify(text, signature, &key)?
                .catalog()
                .map(|e| e.version),
            Some(7)
        );
        assert_eq!(sign(text, SEED)?, signature, "ed25519 is deterministic");
        Ok(())
    }

    #[test]
    fn hostile_manifests_are_refused() {
        for text in [
            "",
            "myos-catalog-manifest 2\n",
            "myos-catalog-manifest 1\ncatalog ../etc/passwd 1 1 1 00\n",
            "myos-catalog-manifest 1\nfood-facts BO off.db 1 1 1 00\n",
            "myos-catalog-manifest 1\ncatalog catalog.db 1 x 1 00\n",
        ] {
            assert!(Manifest::parse(text).is_err(), "{text:?}");
        }
    }
}

#[cfg(test)]
mod download_tests {
    use std::io::{BufRead, BufReader, Write as _};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    use super::*;

    /// Serves `data` at `/f.db`: the first request is cut off halfway, later
    /// ones honour `Range`. Records each request's `Range` header.
    fn server(data: Vec<u8>) -> (String, Arc<Mutex<Vec<Option<String>>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("addr"));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        std::thread::spawn(move || {
            for (index, stream) in listener.incoming().enumerate() {
                let Ok(mut stream) = stream else { continue };
                let mut range = None;
                let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                let mut line = String::new();
                while reader.read_line(&mut line).is_ok_and(|read| read > 2) {
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
                        range = value.trim().trim_end_matches('-').parse::<usize>().ok();
                    }
                    line.clear();
                }
                log.lock()
                    .expect("log")
                    .push(range.map(|start| start.to_string()));
                let start = range.unwrap_or(0);
                let rest = &data[start..];
                let head = if range.is_some() {
                    format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{}/{}\r\nConnection: close\r\n\r\n",
                        rest.len(),
                        data.len() - 1,
                        data.len()
                    )
                } else {
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        rest.len()
                    )
                };
                let _ = stream.write_all(head.as_bytes());
                // First request: half the body, then the connection drops.
                let body = if index == 0 {
                    &rest[..rest.len() / 2]
                } else {
                    rest
                };
                let _ = stream.write_all(body);
            }
        });
        (base, seen)
    }

    #[test]
    fn interrupted_downloads_resume_where_they_stopped() -> Result<(), UpdateError> {
        let data: Vec<u8> = (0..200_000_u32).map(|n| (n % 251) as u8).collect();
        let entry = Entry {
            kind: Kind::Catalog,
            file: "f.db".into(),
            schema: 1,
            version: 1,
            size: data.len() as u64,
            sha256: Sha256::digest(&data).into(),
        };
        let (base, seen) = server(data.clone());
        let key = verifying_key(&public_key(&"01".repeat(32))?)?;
        let source = Source::with_scheme(&base, key, false);
        let dir = std::env::temp_dir().join(format!("myos-resume-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let dest = dir.join("catalog.db");

        assert!(source.download(&entry, &dest).is_err(), "cut off halfway");
        assert!(!dest.exists());
        let kept = std::fs::metadata(dest.with_extension("download"))?.len();
        assert!(kept > 0 && kept < entry.size);

        source.download(&entry, &dest)?;
        assert_eq!(std::fs::read(&dest)?, data);
        assert_eq!(
            seen.lock().expect("log").clone(),
            vec![None, Some(kept.to_string())],
            "the second request asks only for the rest"
        );
        assert!(!dest.with_extension("download").exists());
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }
}
