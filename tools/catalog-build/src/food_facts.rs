//! `off-<cc>.db`: Open Food Facts products sold in one country, by barcode.
//! One pass over the full export writes every requested country.

use std::collections::HashMap;
use std::path::Path;

use myos_core::code::Code;
use myos_core::diet::Nutrient;
use myos_store::catalog::{FOOD_FACTS_SCHEMA, SCHEMA_VERSION};
use myos_sync_wire::tlv::Tlv;
use rusqlite::{Connection, params};

use crate::BuildError;
use crate::catalog::{create, finish, meta, now_iso};

/// Open Food Facts country tag of an ISO 3166-1 alpha-2 code.
fn country_tag(code: &str) -> Option<&'static str> {
    Some(match code {
        "ar" => "en:argentina",
        "bo" => "en:bolivia",
        "br" => "en:brazil",
        "ca" => "en:canada",
        "cl" => "en:chile",
        "co" => "en:colombia",
        "cr" => "en:costa-rica",
        "de" => "en:germany",
        "ec" => "en:ecuador",
        "es" => "en:spain",
        "fr" => "en:france",
        "gb" => "en:united-kingdom",
        "it" => "en:italy",
        "mx" => "en:mexico",
        "pe" => "en:peru",
        "pt" => "en:portugal",
        "py" => "en:paraguay",
        "us" => "en:united-states",
        "uy" => "en:uruguay",
        "ve" => "en:venezuela",
        _ => return None,
    })
}

/// Export columns read, by header name.
const COLUMNS: [&str; 15] = [
    "code",
    "product_name",
    "brands",
    "quantity",
    "countries_tags",
    "last_modified_t",
    "energy-kj_100g",
    "energy_100g",
    "energy-kcal_100g",
    "proteins_100g",
    "carbohydrates_100g",
    "fat_100g",
    "fiber_100g",
    "sugars_100g",
    "saturated-fat_100g",
];
/// Sodium is read separately: it is the only nutrient kept at mg scale
/// from grams without an energy fallback.
const SODIUM: &str = "sodium_100g";

struct Output {
    conn: Connection,
    count: u64,
}

fn number(field: &[u8]) -> Option<f64> {
    std::str::from_utf8(field)
        .ok()?
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value >= 0.0)
}

/// Nutrient TLV (per 100 g, joules / mg) of one product, `None` without
/// energy or protein.
fn nutrients(field: impl Fn(&str) -> Option<f64>) -> Option<Vec<u8>> {
    // OFF stores every nutrient in grams per 100 g, energy in kJ.
    let energy_kj = field("energy-kj_100g")
        .or_else(|| field("energy_100g"))
        .or_else(|| field("energy-kcal_100g").map(|kcal| kcal * 4.184))?;
    let protein = field("proteins_100g")?;
    let mut record = Tlv::default();
    let mut put = |nutrient: Nutrient, value: f64| {
        // Out-of-range values are data errors upstream: drop the nutrient.
        if (0.0..1.0e12).contains(&value) {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "range-checked above"
            )]
            record.put_uint(nutrient.code(), value.round() as u64);
        }
    };
    // Tags must be written in increasing code order.
    put(Nutrient::Energy, energy_kj * 1000.0);
    put(Nutrient::Protein, protein * 1000.0);
    for (nutrient, column) in [
        (Nutrient::Carbohydrate, "carbohydrates_100g"),
        (Nutrient::Fat, "fat_100g"),
        (Nutrient::Fiber, "fiber_100g"),
        (Nutrient::Sugar, "sugars_100g"),
        (Nutrient::SaturatedFat, "saturated-fat_100g"),
        (Nutrient::Sodium, SODIUM),
    ] {
        if let Some(grams) = field(column) {
            put(nutrient, grams * 1000.0);
        }
    }
    Some(record.encode())
}

fn text(field: &[u8], limit: usize) -> Option<String> {
    let value = String::from_utf8_lossy(field).trim().to_owned();
    (!value.is_empty()).then(|| value.chars().take(limit).collect())
}

/// One empty `off-<cc>.db` per country, keyed by its OFF tag, each inside
/// an open transaction.
fn open_outputs(
    out: &Path,
    countries: &[String],
    version: u32,
) -> Result<HashMap<&'static str, (String, Output)>, BuildError> {
    let mut outputs = HashMap::new();
    for country in countries {
        let tag = country_tag(country)
            .ok_or_else(|| BuildError::Invalid(format!("unknown country code {country}")))?;
        let mut conn = create(&out.join(format!("off-{country}.db")), FOOD_FACTS_SCHEMA)?;
        let tx = conn.transaction()?;
        meta(
            &tx,
            "meta",
            &[
                ("schema_version", SCHEMA_VERSION.to_string()),
                ("version", version.to_string()),
                ("country", country.clone()),
                ("built_at", now_iso()),
                (
                    "license",
                    "ODbL-1.0 (database), DbCL-1.0 (contents)".to_owned(),
                ),
                (
                    "attribution",
                    "Open Food Facts contributors, https://world.openfoodfacts.org".to_owned(),
                ),
            ],
        )?;
        tx.commit()?;
        conn.execute_batch("BEGIN")?;
        outputs.insert(tag, (country.clone(), Output { conn, count: 0 }));
    }
    Ok(outputs)
}

/// Build `off-<cc>.db` for every country in `countries` from the export at
/// `input` (`.csv.gz`, tab separated).
pub(crate) fn build(
    input: &Path,
    out: &Path,
    countries: &[String],
    version: u32,
) -> Result<(), BuildError> {
    let mut outputs = open_outputs(out, countries, version)?;

    let reader = flate2::read::MultiGzDecoder::new(std::fs::File::open(input)?);
    let mut csv = csv::ReaderBuilder::new()
        .delimiter(b'\t')
        .quoting(false)
        .flexible(true)
        .from_reader(std::io::BufReader::with_capacity(1 << 20, reader));
    let headers = csv.byte_headers()?.clone();
    let index: HashMap<&str, usize> = COLUMNS
        .iter()
        .chain(std::iter::once(&SODIUM))
        .filter_map(|name| {
            headers
                .iter()
                .position(|header| header == name.as_bytes())
                .map(|at| (*name, at))
        })
        .collect();
    for required in ["code", "product_name", "countries_tags"] {
        if !index.contains_key(required) {
            return Err(BuildError::Invalid(format!(
                "export has no {required} column"
            )));
        }
    }
    let mut record = csv::ByteRecord::new();
    let mut rows = 0_u64;
    // A truncated download ends the stream with an error: report it.
    while csv.read_byte_record(&mut record)? {
        rows += 1;
        let get = |name: &str| {
            index
                .get(name)
                .and_then(|at| record.get(*at))
                .unwrap_or_default()
        };
        let tags = get("countries_tags");
        let targets: Vec<&'static str> = std::str::from_utf8(tags)
            .unwrap_or_default()
            .split(',')
            .filter_map(|tag| outputs.get_key_value(tag.trim()).map(|(key, _)| *key))
            .collect();
        if targets.is_empty() {
            continue;
        }
        let barcode = String::from_utf8_lossy(get("code")).trim().to_owned();
        if !(8..=14).contains(&barcode.len()) || !barcode.bytes().all(|byte| byte.is_ascii_digit())
        {
            continue;
        }
        let Some(name) = text(get("product_name"), 200) else {
            continue;
        };
        let Some(nutrients) = nutrients(|column| number(get(column))) else {
            continue;
        };
        let brand = text(get("brands"), 120);
        let quantity = text(get("quantity"), 60);
        let modified = std::str::from_utf8(get("last_modified_t"))
            .ok()
            .and_then(|value| value.trim().parse::<i64>().ok());
        for tag in targets {
            if let Some((_, output)) = outputs.get_mut(tag) {
                // A barcode listed twice keeps its latest edit.
                output.conn.execute(
                    "INSERT INTO product (barcode, name, brand, quantity, nutrients, off_last_modified)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT (barcode) DO UPDATE SET name = excluded.name,
                       brand = excluded.brand, quantity = excluded.quantity,
                       nutrients = excluded.nutrients,
                       off_last_modified = excluded.off_last_modified
                     WHERE coalesce(excluded.off_last_modified, 0)
                           >= coalesce(product.off_last_modified, 0)",
                    params![barcode, name, brand, quantity, nutrients, modified],
                )?;
                output.count += 1;
            }
        }
    }
    for (country, output) in outputs.into_values() {
        output.conn.execute_batch("COMMIT")?;
        finish(&output.conn, &["product_search"])?;
        eprintln!("off-{country}.db v{version}: {} products", output.count);
    }
    eprintln!("read {rows} export rows");
    Ok(())
}
