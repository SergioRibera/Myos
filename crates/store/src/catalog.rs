//! Read-only reference data: the Myos catalog (`catalog.db`: exercises,
//! foods) and Open Food Facts products (`off-<cc>.db`, by barcode).
//!
//! Both are plaintext files attached immutable (`mode=ro&immutable=1`):
//! nothing the user does is ever written to them. An update replaces the
//! whole file ([`Store::replace_catalog`]).

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;

use myos_core::code::Code;
use myos_core::diet::Nutrients;
use myos_core::exercise::{Exercise, ExerciseId, ExerciseLookup, Modality};
use myos_core::id::Id;
use myos_core::muscle::{Muscle, MuscleRole, MuscleTarget, Share};
use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use crate::content::decode_nutrients;
use crate::{Error, FoodId, Store};

/// Schema of `catalog.db`, for the catalog pipeline and tests.
pub const CATALOG_SCHEMA: &str = include_str!("../migrations/catalog/catalog.sql");
/// Schema of `off-<cc>.db`.
pub const FOOD_FACTS_SCHEMA: &str = include_str!("../migrations/catalog/food_facts.sql");
/// `schema_version` this build reads, in both files' meta tables.
pub const SCHEMA_VERSION: u32 = 1;

/// Language searched when the requested one has no match (the source
/// names of the catalog).
const FALLBACK_LANG: &str = "en";

/// Largest number of search results.
const MAX_HITS: u32 = 100;

/// Which reference file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reference {
    /// `catalog.db`.
    Catalog,
    /// `off-<cc>.db`.
    FoodFacts,
}

impl Reference {
    const fn schema(self) -> &'static str {
        match self {
            Self::Catalog => "catalog",
            Self::FoodFacts => "off",
        }
    }

    const fn meta_table(self) -> &'static str {
        match self {
            Self::Catalog => "catalog_meta",
            Self::FoodFacts => "meta",
        }
    }
}

/// An attached reference file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReferenceInfo {
    /// Data version (derived caches rebuild when the catalog's changes).
    pub version: u32,
}

/// Catalog exercises in memory, for the load engine.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    /// Data version of `catalog.db` (0 when none is attached).
    pub version: u32,
    exercises: HashMap<ExerciseId, Exercise>,
}

impl Catalog {
    /// Number of exercises.
    #[must_use]
    pub fn len(&self) -> usize {
        self.exercises.len()
    }

    /// No exercises (no catalog attached).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.exercises.is_empty()
    }
}

impl ExerciseLookup for Catalog {
    fn exercise(&self, id: ExerciseId) -> Option<&Exercise> {
        self.exercises.get(&id)
    }
}

/// A search result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit<T> {
    /// What was found.
    pub id: T,
    /// Its name in the requested language.
    pub name: String,
}

/// A portion of a catalog food.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Portion {
    /// Label (`1 cup`).
    pub label: String,
    /// Weight, milligrams.
    pub grams_mg: u32,
}

/// A catalog food.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogFood {
    /// Id.
    pub id: FoodId,
    /// Name in the requested language.
    pub name: String,
    /// Per-100 g amounts.
    pub density: Nutrients,
    /// Portions in the requested language.
    pub portions: Vec<Portion>,
    /// Replacement when the food was retired.
    pub replaced_by: Option<FoodId>,
}

/// An Open Food Facts product.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Product {
    /// Barcode (EAN / UPC digits).
    pub barcode: String,
    /// Name.
    pub name: String,
    /// Brand.
    pub brand: Option<String>,
    /// Package quantity as printed (`500 g`).
    pub quantity: Option<String>,
    /// Per-100 g amounts.
    pub density: Nutrients,
}

/// `file:` URI of `path`, percent-encoded (`?`, `#`, `%` and spaces must
/// not reach the URI parser raw).
fn file_uri(path: &Path) -> Result<String, Error> {
    let text = path.to_str().ok_or(Error::OutOfRange("reference path"))?;
    let text = text.replace('\\', "/");
    let mut uri = String::from("file:");
    // Windows drive paths become `file:///C:/…`.
    if text.as_bytes().get(1) == Some(&b':') {
        uri.push_str("///");
    }
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~:".contains(&byte) {
            uri.push(char::from(byte));
        } else {
            let _infallible = write!(uri, "%{byte:02X}");
        }
    }
    uri.push_str("?mode=ro&immutable=1");
    Ok(uri)
}

/// FTS5 query matching every word of `query` as a prefix; `None` when it
/// has no words. Words are quoted, so FTS syntax in user input is inert.
fn fts_query(query: &str) -> Option<String> {
    let words: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .take(8)
        .map(|word| format!("\"{word}\"*"))
        .collect();
    (!words.is_empty()).then(|| words.join(" "))
}

fn uuid(bytes: &[u8]) -> Option<Uuid> {
    Uuid::from_slice(bytes).ok()
}

fn attached(conn: &Connection, reference: Reference) -> Result<bool, Error> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM pragma_database_list WHERE name = ?1",
            [reference.schema()],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn meta_number(conn: &Connection, reference: Reference, key: &str) -> Result<Option<u32>, Error> {
    let value: Option<String> = conn
        .query_row(
            &format!(
                "SELECT value FROM {}.{} WHERE key = ?1",
                reference.schema(),
                reference.meta_table()
            ),
            [key],
            |row| row.get(0),
        )
        .optional()?;
    Ok(value.and_then(|value| value.parse().ok()))
}

impl Store {
    /// Attach a reference file, replacing the one attached before. Fails
    /// with [`Error::ReferenceUnreadable`] when the file is not a reference
    /// file of a schema this build reads.
    pub fn attach_reference(
        &mut self,
        reference: Reference,
        path: &Path,
    ) -> Result<ReferenceInfo, Error> {
        if !path.is_file() {
            return Err(Error::NotFound("reference file"));
        }
        self.detach_reference(reference)?;
        let uri = file_uri(path)?;
        let conn = self.conn();
        // Plaintext file: empty key. Attached outside any transaction.
        conn.execute(
            &format!("ATTACH DATABASE ?1 AS {} KEY ''", reference.schema()),
            [uri.as_str()],
        )?;
        let info = (|| {
            let schema = meta_number(conn, reference, "schema_version")?;
            let version = meta_number(conn, reference, "version")?;
            match (schema, version) {
                (Some(SCHEMA_VERSION), Some(version)) => Ok(ReferenceInfo { version }),
                _ => Err(Error::ReferenceUnreadable),
            }
        })();
        if info.is_err() {
            self.detach_reference(reference)?;
            return Err(Error::ReferenceUnreadable);
        }
        info
    }

    /// Detach a reference file (no-op when none is attached).
    pub fn detach_reference(&mut self, reference: Reference) -> Result<(), Error> {
        let conn = self.conn();
        if !attached(conn, reference)? {
            return Ok(());
        }
        // Cached statements keep the schema in use and block the detach.
        conn.flush_prepared_statement_cache();
        conn.execute(&format!("DETACH DATABASE {}", reference.schema()), [])?;
        Ok(())
    }

    /// Swap in a new version of a reference file: detach, move `next` over
    /// `current` (atomic rename, same directory) and attach it. `next` must
    /// already be verified.
    pub fn replace_reference(
        &mut self,
        reference: Reference,
        next: &Path,
        current: &Path,
    ) -> Result<ReferenceInfo, Error> {
        self.detach_reference(reference)?;
        std::fs::rename(next, current).map_err(Error::Io)?;
        self.attach_reference(reference, current)
    }

    /// Shorthand for [`Store::replace_reference`] on `catalog.db`.
    pub fn replace_catalog(&mut self, next: &Path, current: &Path) -> Result<ReferenceInfo, Error> {
        self.replace_reference(Reference::Catalog, next, current)
    }

    /// Every catalog exercise (retired ones too: history still points at
    /// them); empty when no catalog is attached. Rows with codes this build
    /// does not know are skipped.
    pub fn catalog(&self) -> Result<Catalog, Error> {
        let conn = self.conn();
        if !attached(conn, Reference::Catalog)? {
            return Ok(Catalog::default());
        }
        let version = meta_number(conn, Reference::Catalog, "version")?.unwrap_or(0);
        let mut targets: HashMap<Vec<u8>, Option<Vec<MuscleTarget>>> = HashMap::new();
        let mut statement = conn.prepare(
            "SELECT exercise_id, muscle, role, weight_bp FROM catalog.exercise_target
             ORDER BY exercise_id, muscle",
        )?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let code = |at| -> Result<Option<u16>, rusqlite::Error> {
                Ok(u16::try_from(row.get::<_, i64>(at)?).ok())
            };
            let weight = row.get::<_, Option<i64>>(3)?.map_or(Some(None), |raw| {
                u16::try_from(raw)
                    .ok()
                    .and_then(Share::from_basis_points)
                    .map(Some)
            });
            let target = match (
                code(1)?.and_then(Muscle::from_code),
                code(2)?.and_then(MuscleRole::from_code),
                weight,
            ) {
                (Some(muscle), Some(role), Some(weight)) => Some(MuscleTarget {
                    muscle,
                    role,
                    weight,
                }),
                _ => None,
            };
            let slot = targets
                .entry(row.get(0)?)
                .or_insert_with(|| Some(Vec::new()));
            match (slot.as_mut(), target) {
                (Some(list), Some(target)) => list.push(target),
                _ => *slot = None,
            }
        }
        let mut statement = conn.prepare("SELECT id, modality FROM catalog.exercise")?;
        let mut rows = statement.query([])?;
        let mut exercises = HashMap::new();
        while let Some(row) = rows.next()? {
            let raw: Vec<u8> = row.get(0)?;
            let modality = u16::try_from(row.get::<_, i64>(1)?)
                .ok()
                .and_then(Modality::from_code);
            let exercise_targets = targets.remove(&raw).unwrap_or_else(|| Some(Vec::new()));
            if let (Some(id), Some(modality), Some(targets)) =
                (uuid(&raw), modality, exercise_targets)
            {
                let id = Id::from(id);
                exercises.insert(
                    id,
                    Exercise {
                        id,
                        modality,
                        targets,
                    },
                );
            }
        }
        Ok(Catalog { version, exercises })
    }

    /// Active catalog exercises whose name or aliases in `lang` match every
    /// word of `query` (prefixes), best match first; English names are
    /// searched when nothing matches in `lang`.
    pub fn search_exercises(
        &self,
        query: &str,
        lang: &str,
        limit: u32,
    ) -> Result<Vec<Hit<ExerciseId>>, Error> {
        self.search(
            Reference::Catalog,
            "SELECT n.exercise_id, n.name FROM catalog.exercise_search s
             JOIN catalog.exercise_name n ON n.rid = s.rowid
             JOIN catalog.exercise e ON e.id = n.exercise_id
             WHERE exercise_search MATCH ?1 AND s.lang = ?2 AND e.status = 1
             ORDER BY bm25(exercise_search), n.rid LIMIT ?3",
            query,
            lang,
            limit,
        )
    }

    /// Active catalog foods matching `query` in `lang`, best match first.
    pub fn search_foods(
        &self,
        query: &str,
        lang: &str,
        limit: u32,
    ) -> Result<Vec<Hit<FoodId>>, Error> {
        self.search(
            Reference::Catalog,
            "SELECT n.food_id, n.name FROM catalog.food_search s
             JOIN catalog.food_name n ON n.rid = s.rowid
             JOIN catalog.food f ON f.id = n.food_id
             WHERE food_search MATCH ?1 AND s.lang = ?2 AND f.status = 1
             ORDER BY bm25(food_search), n.rid LIMIT ?3",
            query,
            lang,
            limit,
        )
    }

    fn search<T: From<Uuid>>(
        &self,
        reference: Reference,
        sql: &str,
        query: &str,
        lang: &str,
        limit: u32,
    ) -> Result<Vec<Hit<T>>, Error> {
        let hits = self.search_in(reference, sql, query, lang, limit)?;
        if hits.is_empty() && lang != FALLBACK_LANG {
            return self.search_in(reference, sql, query, FALLBACK_LANG, limit);
        }
        Ok(hits)
    }

    fn search_in<T: From<Uuid>>(
        &self,
        reference: Reference,
        sql: &str,
        query: &str,
        lang: &str,
        limit: u32,
    ) -> Result<Vec<Hit<T>>, Error> {
        let conn = self.conn();
        let Some(fts) = fts_query(query) else {
            return Ok(Vec::new());
        };
        if !attached(conn, reference)? {
            return Ok(Vec::new());
        }
        let mut statement = conn.prepare_cached(sql)?;
        let hits = statement
            .query_map(params![fts, lang, limit.min(MAX_HITS)], |row| {
                Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?))
            })?
            .filter_map(|row| match row {
                Ok((id, name)) => uuid(&id).map(|id| {
                    Ok(Hit {
                        id: T::from(id),
                        name,
                    })
                }),
                Err(err) => Some(Err(err.into())),
            })
            .collect::<Result<_, Error>>()?;
        Ok(hits)
    }

    /// A catalog food with its name and portions in `lang` (English when it
    /// has none in `lang`); `None` when it is unknown, has no name in either
    /// or carries nutrients this build does not know.
    pub fn catalog_food(&self, id: FoodId, lang: &str) -> Result<Option<CatalogFood>, Error> {
        let conn = self.conn();
        if !attached(conn, Reference::Catalog)? {
            return Ok(None);
        }
        let key = id.uuid();
        let key = key.as_bytes().as_slice();
        let Some((name, replaced_by)) = conn
            .prepare_cached(
                "SELECT n.name, f.replaced_by FROM catalog.food f
                 JOIN catalog.food_name n ON n.food_id = f.id AND n.lang IN (?2, ?3)
                 WHERE f.id = ?1 ORDER BY n.lang = ?2 DESC LIMIT 1",
            )?
            .query_row(params![key, lang, FALLBACK_LANG], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<Vec<u8>>>(1)?))
            })
            .optional()?
        else {
            return Ok(None);
        };
        let mut density = Nutrients::default();
        let mut statement = conn.prepare_cached(
            "SELECT nutrient, amount_100g FROM catalog.food_nutrient WHERE food_id = ?1",
        )?;
        let mut rows = statement.query([key])?;
        while let Some(row) = rows.next()? {
            let nutrient = u16::try_from(row.get::<_, i64>(0)?)
                .ok()
                .and_then(myos_core::diet::Nutrient::from_code);
            let amount = u64::try_from(row.get::<_, i64>(1)?).ok();
            let (Some(nutrient), Some(amount)) = (nutrient, amount) else {
                return Ok(None);
            };
            density = density.with(nutrient, amount);
        }
        let portions = conn
            .prepare_cached(
                "SELECT label, grams_mg FROM catalog.food_portion
                 WHERE food_id = ?1 AND lang = (
                   SELECT lang FROM catalog.food_portion
                   WHERE food_id = ?1 AND lang IN (?2, ?3) ORDER BY lang = ?2 DESC LIMIT 1)
                 ORDER BY position",
            )?
            .query_map(params![key, lang, FALLBACK_LANG], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?
            .filter_map(|row| match row {
                Ok((label, grams)) => u32::try_from(grams)
                    .ok()
                    .map(|grams_mg| Ok(Portion { label, grams_mg })),
                Err(err) => Some(Err(err)),
            })
            .collect::<Result<_, _>>()?;
        Ok(Some(CatalogFood {
            id,
            name,
            density,
            portions,
            replaced_by: replaced_by.as_deref().and_then(uuid).map(Id::from),
        }))
    }

    /// The Open Food Facts product with this barcode; `None` when unknown,
    /// when no product file is attached or when its nutrients are malformed.
    pub fn product(&self, barcode: &str) -> Result<Option<Product>, Error> {
        let conn = self.conn();
        if !attached(conn, Reference::FoodFacts)? {
            return Ok(None);
        }
        let row = conn
            .prepare_cached(
                "SELECT barcode, name, brand, quantity, nutrients FROM off.product
                 WHERE barcode = ?1",
            )?
            .query_row([barcode], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Vec<u8>>(4)?,
                ))
            })
            .optional()?;
        Ok(row.and_then(|(barcode, name, brand, quantity, nutrients)| {
            Some(Product {
                barcode,
                name,
                brand,
                quantity,
                density: decode_nutrients(&nutrients).ok()?,
            })
        }))
    }

    /// Barcodes of products whose name or brand match `query`.
    pub fn search_products(&self, query: &str, limit: u32) -> Result<Vec<Hit<String>>, Error> {
        let conn = self.conn();
        let Some(fts) = fts_query(query) else {
            return Ok(Vec::new());
        };
        if !attached(conn, Reference::FoodFacts)? {
            return Ok(Vec::new());
        }
        let mut statement = conn.prepare_cached(
            "SELECT p.barcode, p.name FROM off.product_search s
             JOIN off.product p ON p.rid = s.rowid
             WHERE product_search MATCH ?1
             ORDER BY bm25(product_search), p.rid LIMIT ?2",
        )?;
        let hits = statement
            .query_map(params![fts, limit.min(MAX_HITS)], |row| {
                Ok(Hit {
                    id: row.get(0)?,
                    name: row.get(1)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(hits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uris_are_escaped() -> Result<(), Error> {
        assert_eq!(
            file_uri(Path::new("/data/my app/catalog?#%.db"))?,
            "file:/data/my%20app/catalog%3F%23%25.db?mode=ro&immutable=1"
        );
        assert_eq!(
            file_uri(Path::new(r"C:\Users\a\catalog.db"))?,
            "file:///C:/Users/a/catalog.db?mode=ro&immutable=1"
        );
        Ok(())
    }

    #[test]
    fn fts_syntax_is_inert() {
        assert_eq!(
            fts_query("bench \"press\" OR *"),
            Some("\"bench\"* \"press\"* \"OR\"*".to_owned())
        );
        assert_eq!(fts_query("  -- "), None);
    }
}
