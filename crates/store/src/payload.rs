//! Rows ↔ sync payloads, driven by [`crate::spec`].
//!
//! Values travel as their `SQLite` storage class: INTEGER as a zig-zag varint,
//! TEXT as UTF-8, BLOB raw; NULL is an absent tag. Tables are STRICT, so the
//! declared type of a column is its storage class. An absent tag on a
//! `NOT NULL DEFAULT` column means the default (older senders do not know the
//! column).

use std::collections::HashMap;

use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, params_from_iter};
use uuid::Uuid;

use crate::Error;
use crate::spec::{Members, TAG_DELETED_AT, TAG_FIELD_HLC, TAG_MEMBERS, TableSpec};
use crate::tlv::{Tlv, TlvError};

/// Size limit of one payload.
pub(crate) const PAYLOAD_LIMIT: usize = 1024 * 1024;

/// Storage class of a column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Integer,
    Text,
    Blob,
}

/// What a payload decoder needs to know about a column.
#[derive(Debug, Clone)]
struct Column {
    class: Class,
    /// Value for an absent tag.
    default: Value,
}

/// Columns of one table.
#[derive(Debug, Clone, Default)]
pub(crate) struct Columns(HashMap<String, Column>);

impl Columns {
    /// Read `table`'s declared columns.
    pub(crate) fn of(conn: &Connection, table: &str) -> Result<Self, Error> {
        let mut statement =
            conn.prepare_cached("SELECT name, type, dflt_value FROM pragma_table_info(?1)")?;
        let rows = statement.query_map([table], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?;
        let mut columns = HashMap::new();
        for row in rows {
            let (name, declared, default) = row?;
            let class = match declared.to_ascii_uppercase().as_str() {
                "INTEGER" => Class::Integer,
                "TEXT" => Class::Text,
                "BLOB" => Class::Blob,
                _ => return Err(Error::OutOfRange("column type")),
            };
            // Defaults in our schema are integer literals.
            let default = default
                .and_then(|literal| literal.parse::<i64>().ok())
                .map_or(Value::Null, Value::Integer);
            columns.insert(name, Column { class, default });
        }
        Ok(Self(columns))
    }

    fn get(&self, name: &str) -> Result<&Column, Error> {
        self.0.get(name).ok_or(Error::OutOfRange("column"))
    }
}

fn put_value(record: &mut Tlv, tag: u16, value: &Value) {
    match value {
        Value::Null | Value::Real(_) => {}
        Value::Integer(int) => {
            record.put_int(tag, *int);
        }
        Value::Text(text) => {
            record.put_text(tag, text);
        }
        Value::Blob(bytes) => {
            record.put_bytes(tag, bytes);
        }
    }
}

fn get_value(record: &Tlv, tag: u16, column: &Column) -> Result<Value, TlvError> {
    if !record.contains(tag) {
        return Ok(column.default.clone());
    }
    Ok(match column.class {
        Class::Integer => record.int(tag)?.map_or(Value::Null, Value::Integer),
        Class::Text => record
            .text(tag)?
            .map_or(Value::Null, |text| Value::Text(text.to_owned())),
        Class::Blob => record
            .bytes(tag)
            .map_or(Value::Null, |bytes| Value::Blob(bytes.to_vec())),
    })
}

/// Add the fields of a stored `extra` that the record does not set (tags that
/// became columns since are already set and win).
fn fill_extra(record: &mut Tlv, extra: &Value) -> Result<(), Error> {
    if let Value::Blob(extra) = extra {
        record.fill_from(&Tlv::decode(extra, PAYLOAD_LIMIT)?);
    }
    Ok(())
}

fn member_tag(index: usize) -> Result<u16, Error> {
    u16::try_from(index + 1).map_err(|_| Error::OutOfRange("members"))
}

/// Payload of row `id`, or `None` when the row is gone. Tombstones carry the
/// full row, so a device that never had it can store the tombstone.
pub(crate) fn build(
    conn: &Connection,
    spec: &TableSpec,
    id: Uuid,
) -> Result<Option<Vec<u8>>, Error> {
    let sql = format!(
        "SELECT deleted_at, extra, {} FROM {} WHERE id = ?1",
        spec.columns.join(", "),
        spec.table
    );
    let row: Option<Vec<Value>> = conn
        .query_row(&sql, [id.as_bytes().as_slice()], |row| {
            (0..spec.columns.len() + 2)
                .map(|at| row.get::<_, Value>(at))
                .collect()
        })
        .optional()?;
    let Some(values) = row else {
        return Ok(None);
    };
    let mut record = Tlv::default();
    put_value(&mut record, TAG_DELETED_AT, &values[0]);
    for (index, value) in values[2..].iter().enumerate() {
        put_value(&mut record, TableSpec::tag(index), value);
    }
    if let Some(members) = spec.members {
        record.put_record(TAG_MEMBERS, &build_members(conn, members, id)?);
    }
    fill_extra(&mut record, &values[1])?;
    Ok(Some(record.encode()))
}

fn build_members(conn: &Connection, members: Members, id: Uuid) -> Result<Tlv, Error> {
    let columns = members.columns.join(", ");
    let sql = format!(
        "SELECT extra, {columns} FROM {} WHERE {} = ?1 ORDER BY {columns}",
        members.table, members.parent,
    );
    let mut statement = conn.prepare_cached(&sql)?;
    let mut rows = statement.query([id.as_bytes().as_slice()])?;
    let mut list = Tlv::default();
    let mut position: u16 = 0;
    while let Some(row) = rows.next()? {
        position = position
            .checked_add(1)
            .ok_or(Error::OutOfRange("members"))?;
        let mut member = Tlv::default();
        for index in 0..members.columns.len() {
            put_value(
                &mut member,
                member_tag(index)?,
                &row.get::<_, Value>(index + 1)?,
            );
        }
        fill_extra(&mut member, &row.get::<_, Value>(0)?)?;
        list.put_record(position, &member);
    }
    Ok(list)
}

/// A decoded payload ready to write.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Decoded {
    /// `deleted_at` (NULL when live).
    pub(crate) deleted_at: Value,
    /// Data column values, in spec order.
    pub(crate) values: Vec<Value>,
    /// Member rows: values in member column order, then their `extra`.
    pub(crate) members: Vec<(Vec<Value>, Value)>,
    /// Unknown tags, stored in `extra` (replacing what was there).
    pub(crate) extra: Value,
}

impl Decoded {
    /// Value of column `name`, if the spec has it.
    pub(crate) fn value(&self, spec: &TableSpec, name: &str) -> Option<&Value> {
        spec.columns
            .iter()
            .position(|column| *column == name)
            .and_then(|index| self.values.get(index))
    }
}

fn encode_unknown(unknown: &Tlv) -> Value {
    if unknown.is_empty() {
        Value::Null
    } else {
        Value::Blob(unknown.encode())
    }
}

/// Decode `payload` for `spec`.
pub(crate) fn decode(
    payload: &[u8],
    spec: &TableSpec,
    columns: &Columns,
    member_columns: Option<&Columns>,
) -> Result<Decoded, Error> {
    let record = Tlv::decode(payload, PAYLOAD_LIMIT)?;
    let deleted_at = get_value(
        &record,
        TAG_DELETED_AT,
        &Column {
            class: Class::Integer,
            default: Value::Null,
        },
    )?;
    let mut known = vec![TAG_DELETED_AT, TAG_FIELD_HLC, TAG_MEMBERS];
    let mut values = Vec::with_capacity(spec.columns.len());
    for (index, name) in spec.columns.iter().enumerate() {
        let tag = TableSpec::tag(index);
        known.push(tag);
        values.push(get_value(&record, tag, columns.get(name)?)?);
    }
    let mut members = Vec::new();
    if let (Some(spec_members), Some(member_columns)) = (spec.members, member_columns)
        && let Some(list) = record.record(TAG_MEMBERS, PAYLOAD_LIMIT)?
    {
        for position in list.tags() {
            let member = list
                .record(position, PAYLOAD_LIMIT)?
                .ok_or(TlvError::Malformed { tag: position })?;
            let mut row = Vec::with_capacity(spec_members.columns.len());
            let mut member_known = Vec::with_capacity(spec_members.columns.len());
            for (index, name) in spec_members.columns.iter().enumerate() {
                let tag = member_tag(index)?;
                member_known.push(tag);
                row.push(get_value(&member, tag, member_columns.get(name)?)?);
            }
            members.push((row, encode_unknown(&member.unknown(&member_known))));
        }
    }
    Ok(Decoded {
        deleted_at,
        values,
        members,
        extra: encode_unknown(&record.unknown(&known)),
    })
}

/// Sync stamp written with a remote row.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RowStamp {
    pub(crate) hlc: i64,
    pub(crate) node: i64,
    pub(crate) server_rev: i64,
    pub(crate) local_seq: i64,
}

/// Write `decoded` as row `id` (insert, or update every payload column), and
/// replace its members. Never `INSERT OR REPLACE`: that would cascade-delete
/// members of other parents' rows sharing the key.
pub(crate) fn write(
    conn: &Connection,
    spec: &TableSpec,
    id: Uuid,
    decoded: &Decoded,
    stamp: RowStamp,
    has_field_hlc: bool,
) -> Result<(), Error> {
    let placeholders: Vec<String> = (1..=spec.columns.len() + 7)
        .map(|n| format!("?{n}"))
        .collect();
    let mut updates: Vec<String> = spec
        .columns
        .iter()
        .chain(
            [
                "deleted_at",
                "extra",
                "hlc",
                "node",
                "server_rev",
                "local_seq",
            ]
            .iter(),
        )
        .map(|name| format!("{name} = excluded.{name}"))
        .collect();
    if has_field_hlc {
        updates.push("field_hlc = NULL".to_owned());
    }
    let sql = format!(
        "INSERT INTO {table} (id, {columns}, deleted_at, extra, hlc, node, server_rev, local_seq)
         VALUES ({placeholders})
         ON CONFLICT (id) DO UPDATE SET {updates}",
        table = spec.table,
        columns = spec.columns.join(", "),
        placeholders = placeholders.join(", "),
        updates = updates.join(", "),
    );
    let mut params: Vec<Value> = Vec::with_capacity(spec.columns.len() + 7);
    params.push(Value::Blob(id.as_bytes().to_vec()));
    params.extend(decoded.values.iter().cloned());
    params.push(decoded.deleted_at.clone());
    params.push(decoded.extra.clone());
    params.extend([
        Value::Integer(stamp.hlc),
        Value::Integer(stamp.node),
        Value::Integer(stamp.server_rev),
        Value::Integer(stamp.local_seq),
    ]);
    conn.execute(&sql, params_from_iter(params))?;
    if let Some(members) = spec.members {
        conn.execute(
            &format!(
                "DELETE FROM {} WHERE {} = ?1",
                members.table, members.parent
            ),
            [id.as_bytes().as_slice()],
        )?;
        let sql = format!(
            "INSERT INTO {} ({}, {}, extra) VALUES ({})",
            members.table,
            members.parent,
            members.columns.join(", "),
            (1..=members.columns.len() + 2)
                .map(|n| format!("?{n}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        for (row, extra) in &decoded.members {
            let mut params = Vec::with_capacity(row.len() + 2);
            params.push(Value::Blob(id.as_bytes().to_vec()));
            params.extend(row.iter().cloned());
            params.push(extra.clone());
            conn.execute(&sql, params_from_iter(params))?;
        }
    }
    Ok(())
}
