-- off-<cc>.db: Open Food Facts products of one country (barcode layer),
-- shipped as an immutable plaintext file. License and attribution in meta.

CREATE TABLE meta (
  key   TEXT PRIMARY KEY,           -- version, schema_version, country, license, attribution
  value TEXT NOT NULL
) STRICT, WITHOUT ROWID;

CREATE TABLE product (
  rid               INTEGER PRIMARY KEY,
  barcode           TEXT NOT NULL UNIQUE,
  name              TEXT NOT NULL,
  brand             TEXT,
  quantity          TEXT,
  nutrients         BLOB NOT NULL,  -- per 100 g, nutrient TLV
  off_last_modified INTEGER
) STRICT;

CREATE VIRTUAL TABLE product_search USING fts5 (
  name, brand,
  content = 'product', content_rowid = 'rid',
  tokenize = 'unicode61 remove_diacritics 2', prefix = '2 3'
);
