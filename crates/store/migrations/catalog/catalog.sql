-- catalog.db: Myos exercises and foods, built by the catalog pipeline and
-- shipped as an immutable plaintext file. History points at these ids, so
-- rows are never deleted: retired rows point at their replacement.

CREATE TABLE catalog_meta (
  key   TEXT PRIMARY KEY,           -- version, schema_version, built_at
  value TEXT NOT NULL
) STRICT, WITHOUT ROWID;

CREATE TABLE source (
  id          INTEGER PRIMARY KEY,
  name        TEXT NOT NULL,
  license     TEXT NOT NULL,
  attribution TEXT NOT NULL,
  url         TEXT,
  version     TEXT
) STRICT;

CREATE TABLE exercise (
  id          BLOB PRIMARY KEY CHECK (length(id) = 16),   -- UUIDv5(slug)
  slug        TEXT NOT NULL UNIQUE,
  modality    INTEGER NOT NULL CHECK (modality BETWEEN 1 AND 65535),
  status      INTEGER NOT NULL DEFAULT 1 CHECK (status IN (1, 2)),  -- active, retired
  replaced_by BLOB REFERENCES exercise (id)
) STRICT, WITHOUT ROWID;

CREATE TABLE exercise_target (
  exercise_id BLOB NOT NULL REFERENCES exercise (id),
  muscle      INTEGER NOT NULL CHECK (muscle BETWEEN 1 AND 65535),
  role        INTEGER NOT NULL CHECK (role BETWEEN 1 AND 65535),
  weight_bp   INTEGER CHECK (weight_bp BETWEEN 0 AND 10000),  -- NULL: role default
  PRIMARY KEY (exercise_id, muscle)
) STRICT, WITHOUT ROWID;

CREATE TABLE exercise_name (
  rid         INTEGER PRIMARY KEY,
  exercise_id BLOB NOT NULL REFERENCES exercise (id),
  lang        TEXT NOT NULL,
  name        TEXT NOT NULL,
  aliases     TEXT NOT NULL DEFAULT '',
  UNIQUE (exercise_id, lang)
) STRICT;

CREATE VIRTUAL TABLE exercise_search USING fts5 (
  name, aliases, lang UNINDEXED,
  content = 'exercise_name', content_rowid = 'rid',
  tokenize = 'unicode61 remove_diacritics 2', prefix = '2 3'
);

CREATE TABLE food (
  id          BLOB PRIMARY KEY CHECK (length(id) = 16),
  status      INTEGER NOT NULL DEFAULT 1 CHECK (status IN (1, 2)),
  replaced_by BLOB REFERENCES food (id)
) STRICT, WITHOUT ROWID;

-- The only nutrient source: joules for energy, mg otherwise, per 100 g.
CREATE TABLE food_nutrient (
  food_id     BLOB NOT NULL REFERENCES food (id),
  nutrient    INTEGER NOT NULL CHECK (nutrient BETWEEN 1 AND 65535),
  amount_100g INTEGER NOT NULL CHECK (amount_100g >= 0),
  PRIMARY KEY (food_id, nutrient)
) STRICT, WITHOUT ROWID;

CREATE TABLE food_name (
  rid     INTEGER PRIMARY KEY,
  food_id BLOB NOT NULL REFERENCES food (id),
  lang    TEXT NOT NULL,
  name    TEXT NOT NULL,
  aliases TEXT NOT NULL DEFAULT '',
  UNIQUE (food_id, lang)
) STRICT;

CREATE VIRTUAL TABLE food_search USING fts5 (
  name, aliases, lang UNINDEXED,
  content = 'food_name', content_rowid = 'rid',
  tokenize = 'unicode61 remove_diacritics 2', prefix = '2 3'
);

CREATE TABLE food_portion (
  food_id  BLOB NOT NULL REFERENCES food (id),
  position INTEGER NOT NULL,
  lang     TEXT NOT NULL,
  label    TEXT NOT NULL,
  grams_mg INTEGER NOT NULL CHECK (grams_mg > 0),
  PRIMARY KEY (food_id, lang, position)
) STRICT, WITHOUT ROWID;
