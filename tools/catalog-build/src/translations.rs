//! Names in other languages: `translations/<lang>.tsv`, one row per catalog
//! row, keyed by its catalog id.
//!
//! ```text
//! # id <TAB> English source <TAB> translation <TAB> origin
//! 0b8f…  Barbell Bench Press  Press de banca con barra  machine
//! ```
//!
//! `origin` is `glossary` (exercises, composed from the curated
//! `glossary-<lang>.tsv`), `machine` (foods, offline machine translation) or
//! `human` (written or corrected by hand; never overwritten). All come from
//! `translate.py`. The English
//! source is kept so a changed source name is translated again. The English
//! name becomes an alias of the translation, so English searches still find
//! the row in every language.

use std::collections::HashSet;
use std::io::Write as _;
use std::path::Path;

use rusqlite::{Connection, Transaction, params};
use uuid::Uuid;

use crate::BuildError;

/// One translated name.
struct Row {
    id: Uuid,
    source: String,
    name: String,
}

fn valid_lang(lang: &str) -> bool {
    (2..=3).contains(&lang.len()) && lang.bytes().all(|byte| byte.is_ascii_lowercase())
}

fn parse(path: &Path) -> Result<Vec<Row>, BuildError> {
    let text = std::fs::read_to_string(path)?;
    let mut rows = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let bad =
            || BuildError::Invalid(format!("{}:{}: malformed row", path.display(), index + 1));
        let [id, source, name, origin] = line.split('\t').collect::<Vec<_>>()[..] else {
            return Err(bad());
        };
        if !matches!(origin, "machine" | "glossary" | "human") || name.trim().is_empty() {
            return Err(bad());
        }
        rows.push(Row {
            id: id.parse().map_err(|_| bad())?,
            source: source.to_owned(),
            name: name.trim().to_owned(),
        });
    }
    Ok(rows)
}

/// Insert the names of every `<lang>.tsv` in `dir`; returns how many.
/// Rows whose id is not in the catalog any more are skipped.
pub(crate) fn apply(tx: &Transaction<'_>, dir: &Path) -> Result<usize, BuildError> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(0);
    };
    let mut count = 0;
    for entry in entries {
        let path = entry?.path();
        let Some(lang) = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .filter(|_| path.extension().is_some_and(|ext| ext == "tsv"))
            .filter(|lang| valid_lang(lang) && *lang != "en")
            .map(str::to_owned)
        else {
            continue;
        };
        let mut seen = HashSet::new();
        for row in parse(&path)? {
            if !seen.insert(row.id) {
                return Err(BuildError::Invalid(format!(
                    "{}: {} listed twice",
                    path.display(),
                    row.id
                )));
            }
            let id = row.id.as_bytes().as_slice();
            for (table, column, owner) in [
                ("exercise_name", "exercise_id", "exercise"),
                ("food_name", "food_id", "food"),
            ] {
                // Only rows whose English source is unchanged: a renamed
                // source waits for its new translation.
                count += tx.execute(
                    &format!(
                        "INSERT INTO {table} ({column}, lang, name, aliases)
                         SELECT ?1, ?2, ?3, n.name FROM {table} n
                         JOIN {owner} o ON o.id = n.{column}
                         WHERE n.{column} = ?1 AND n.lang = \x27en\x27 AND n.name = ?4"
                    ),
                    params![id, lang, row.name, row.source],
                )?;
            }
        }
    }
    Ok(count)
}

/// Write `id <TAB> kind <TAB> English name` of every catalog row, the
/// input of `translate.py`.
pub(crate) fn names(catalog: &Path, out: &Path) -> Result<(), BuildError> {
    let conn = Connection::open(catalog)?;
    let mut file = std::io::BufWriter::new(std::fs::File::create(out)?);
    for (table, column, kind) in [
        ("exercise_name", "exercise_id", "exercise"),
        ("food_name", "food_id", "food"),
    ] {
        let mut statement = conn.prepare(&format!(
            "SELECT {column}, name FROM {table} WHERE lang = 'en' ORDER BY name, {column}"
        ))?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let id: Vec<u8> = row.get(0)?;
            let name: String = row.get(1)?;
            let id = Uuid::from_slice(&id).map_err(|_| BuildError::Invalid("bad id".into()))?;
            writeln!(file, "{id}\t{kind}\t{}", name.replace(['\t', '\n'], " "))?;
        }
    }
    file.flush()?;
    Ok(())
}
