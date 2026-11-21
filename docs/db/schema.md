# Myos data schema

Device: SQLite + SQLCipher through `rusqlite`. Server: PostgreSQL. Sync: own
outbox (push) + per-user change feed (pull). This document is the contract
both sides implement; DDL here is normative unless marked *sketch*.

## 1. Files on device

| File | Encrypted | Opened as | Synced | Contents |
|---|---|---|---|---|
| `user.db` | SQLCipher, raw key | read-write, main | yes | everything the user creates |
| `derived.db` | SQLCipher, same key, explicit `KEY` | attached | no | recomputable caches |
| `catalog.db` | no | `file:catalog.db?mode=ro&immutable=1`, `KEY ''` | replaced as a file | Myos exercises, foods, taxonomy, reference tables |
| `off-<cc>.db` | no | same as catalog | replaced as a file | Open Food Facts products (barcode layer) |

Rules:

- Nothing the user does (searches, favourites, scans, recents) is ever
  written to `catalog.db` / `off-*.db`; those files are immutable.
- `user.db` and `derived.db` are never written in the same transaction
  (WAL atomicity is per file). `derived.db` is a cache keyed by a
  `local_seq` watermark; on version mismatch or corruption it is deleted
  and rebuilt.
- Catalog / OFF updates replace the whole file: build `*.next.db`, verify
  sha256 + ed25519 signature, fsync, `DETACH`, `rename()`, `ATTACH`.
- ATTACH once per connection, outside any transaction.

## 2. Encryption and keys

- Key: 32 random bytes generated on first launch; passed as
  `PRAGMA key = "x'<64 hex>'"` (raw key, no KDF), then
  `PRAGMA cipher_compatibility = 4`. Key bytes are zeroized in Rust after use.
- Storage of the key:
  - Android: non-exportable AES-GCM key in Android Keystore (StrongBox if
    available), no user-auth requirement; it wraps the SQLCipher key, wrapped
    blob stored in `noBackupFilesDir`.
  - iOS / macOS: Keychain, `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`.
  - Windows: DPAPI / Credential Manager.
  - Linux: Secret Service → XDG portal → Argon2id passphrase prompt; a 0600
    key file only after explicit user acknowledgement. Never silently plain.
- Backups: every DB file, `-wal`, `-shm`, wrapped key and attachment are
  excluded from OS backups (Android `dataExtractionRules` +
  `fullBackupContent`; iOS `isExcludedFromBackup`). Opening with a wrong or
  missing key (`SQLITE_NOTADB`) routes to recovery (server restore or
  encrypted export import), never a crash.
- `PRAGMA temp_store = MEMORY` (the bundled `SQLCipher` build sets
  `SQLITE_TEMP_STORE=2`, which honours the pragma); `VACUUM INTO` and
  any plaintext export of `user.db` are forbidden.
- Logout: checkpoint, close, delete `user.db*` and `derived.db*`, drop the
  key.
- Build: `bundled-sqlcipher-vendored-openssl` on Android / Windows / Linux;
  `bundled-sqlcipher` (CommonCrypto) on Apple targets built on a macOS host.

## 3. Device conventions

- Every table `STRICT`. Large synced tables `WITHOUT ROWID`.
- Ids: 16-byte UUID `BLOB`, `CHECK (length(id) = 16)`; v7 for created rows.
  Singletons (`profile`, `preferences`) use fixed v8 ids: the server keys
  rows by user, so every device already agrees. Natural keys
  (`profile_goal` by start day, `external_activity`, `volume_range_override`)
  use **deterministic ids**: UUID v8 of
  `HMAC-SHA256(user_key, entity code (u16 BE) ‖ (len u32 BE ‖ part)*)`.
  Before an account exists, `user_key` is a device-local random key
  (`sync_state.user_key`); on first login the server returns the account
  key, natural-key ids are rewritten once and `id_epoch` bumps.
- Instants: `INTEGER` Unix ms UTC (`*_at`) + `tz_offset_min` where a local
  day matters.
- Local day: `INTEGER` days since 1970-01-01, `CHECK (day BETWEEN 0 AND 73000)`.
  ISO week Monday = `day - ((day + 3) % 7)`.
- Units: suffix is the SI unit of the stored integer: `_g`, `_mg`, `_ug`,
  `_j` (energy, integer joules), `_m`, `_mm`, `_dmm` (0.1 mm), `_ms`,
  `_bpm`, `_bp` (basis points 0..=10000), `_milli` (set equivalents × 1000).
- Enums: `INTEGER` codes from `myos_core::code::Code` (`u16`), pinned by
  tests, 0 = unspecified, append-only, never reused. **No `IN (...)` CHECK**
  on growable enums — range checks only. The row keeps the raw integer;
  Rust decodes an unknown code to `None` and leaves the row out of reads
  (counted as undecodable) instead of failing.
- Text: `CHECK (length(x) <= N)` on every text column.
- FKs: `DEFERRABLE INITIALLY DEFERRED` inside an aggregate; **no FK between
  sync roots** (children may arrive before parents). Visibility rule: a row is
  live only if it and every ancestor have `deleted_at IS NULL`.
- Indexes on live rows are partial: `WHERE deleted_at IS NULL`.
- Migrations: own runner. `BEGIN IMMEDIATE` → check `user_version` → step →
  `PRAGMA foreign_key_check` empty → `user_version = N` → `COMMIT`. Refuse
  files newer than the app. Expand / contract across releases.
- `PRAGMA auto_vacuum = INCREMENTAL` before the first table; periodic
  `incremental_vacuum` and `wal_checkpoint(TRUNCATE)`; `quick_check` after an
  unclean shutdown.

### 3.1 Sync columns

Every **sync root** table carries:

```sql
hlc         INTEGER NOT NULL,   -- 48-bit ms + 16-bit counter of the last write
node        INTEGER NOT NULL,   -- writer node id (tiebreak for equal hlc)
server_rev  INTEGER,            -- per-user revision acked by the server
local_seq   INTEGER NOT NULL,   -- bumped on every local or remote write (derived watermark)
deleted_at  INTEGER,            -- tombstone (sticky: deletes win)
extra       BLOB                -- TLV fields this app version does not know
```

Header tables merged per field also carry `field_hlc BLOB` (TLV of
`tag → hlc`). Each table is declared in code as exactly one of: sync root
(per-row LWW), sync root (per-field LWW), aggregate member (travels in the
root's payload), local only.

## 4. `user.db`

### 4.1 Sync bookkeeping (local only)

```sql
CREATE TABLE sync_state (
  k                INTEGER PRIMARY KEY CHECK (k = 1),
  device_id        BLOB NOT NULL CHECK (length(device_id) = 16),
  node             INTEGER NOT NULL,
  user_id          BLOB CHECK (user_id IS NULL OR length(user_id) = 16),
  user_key         BLOB NOT NULL CHECK (length(user_key) = 32),  -- natural-key ids
  id_epoch         INTEGER NOT NULL DEFAULT 0,
  hlc_last         INTEGER NOT NULL,
  server_offset_ms INTEGER NOT NULL DEFAULT 0,
  local_seq        INTEGER NOT NULL DEFAULT 0,
  proto            INTEGER NOT NULL DEFAULT 1,
  read_only        INTEGER NOT NULL DEFAULT 0 CHECK (read_only IN (0, 1))  -- server needs a newer app
) STRICT;

CREATE TABLE sync_outbox (
  seq          INTEGER PRIMARY KEY,
  op_id        BLOB NOT NULL UNIQUE CHECK (length(op_id) = 16),
  entity       INTEGER NOT NULL,
  row_id       BLOB NOT NULL CHECK (length(row_id) = 16),
  op           INTEGER NOT NULL CHECK (op BETWEEN 1 AND 3),   -- upsert, delete, restore
  hlc          INTEGER NOT NULL,
  state        INTEGER NOT NULL DEFAULT 0 CHECK (state BETWEEN 0 AND 2),  -- pending, in_flight, failed
  attempts     INTEGER NOT NULL DEFAULT 0,
  error_code   INTEGER,                                       -- never a message (no PII)
  next_attempt_at INTEGER,                                    -- backoff
  created_at   INTEGER NOT NULL
) STRICT;
CREATE UNIQUE INDEX sync_outbox_pending ON sync_outbox(entity, row_id) WHERE state = 0;

CREATE TABLE sync_cursor (
  stream     INTEGER PRIMARY KEY,     -- 1 own, 2 assigned-to-me, 3 consent mirror
  cursor     INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
) STRICT;

CREATE TABLE sync_quarantine (        -- rows this version cannot decode yet
  stream INTEGER NOT NULL, server_rev INTEGER NOT NULL, entity INTEGER NOT NULL,
  row_id BLOB NOT NULL, payload_ver INTEGER NOT NULL, payload BLOB, reason INTEGER NOT NULL,
  PRIMARY KEY (stream, server_rev)
) STRICT;
```

The payload is built from the row when pushing, so a coalesced entry always
sends the latest state. Coalescing updates the pending row in place (keeps
`seq`). Ack deletes by
`op_id` only.

### 4.2 Profile and preferences (singletons, per-field LWW)

```sql
CREATE TABLE profile (
  id          BLOB PRIMARY KEY CHECK (length(id) = 16),
  birth_year  INTEGER CHECK (birth_year BETWEEN 1900 AND 2100),
  sex         INTEGER CHECK (sex BETWEEN 0 AND 255),
  height_mm   INTEGER CHECK (height_mm BETWEEN 500 AND 2800),
  -- sync columns + field_hlc
) STRICT;

CREATE TABLE profile_goal (           -- goal and level as a time series
  id        BLOB PRIMARY KEY CHECK (length(id) = 16),
  from_day  INTEGER NOT NULL,
  goal      INTEGER NOT NULL CHECK (goal BETWEEN 0 AND 255),
  level     INTEGER NOT NULL CHECK (level BETWEEN 0 AND 255)
  -- sync columns
) STRICT;

CREATE TABLE preferences (
  id            BLOB PRIMARY KEY CHECK (length(id) = 16),
  theme         INTEGER NOT NULL DEFAULT 0,
  unit_mass     INTEGER NOT NULL DEFAULT 0,
  unit_distance INTEGER NOT NULL DEFAULT 0,
  unit_energy   INTEGER NOT NULL DEFAULT 0,
  effort_scale  INTEGER NOT NULL DEFAULT 0,
  diet_mode     INTEGER NOT NULL DEFAULT 0,
  locale        TEXT CHECK (length(locale) <= 35)
  -- sync columns + field_hlc
) STRICT;
```

### 4.3 Exercise references

```sql
-- Local index of every exercise referenced, rebuilt from synced rows. The
-- source follows from the id: catalog ids are UUID v5, user and organization
-- exercises v7. No foreign key: sets may arrive before their exercise.
CREATE TABLE exercise_ref (
  id  BLOB PRIMARY KEY CHECK (length(id) = 16),
  src INTEGER NOT NULL CHECK (src BETWEEN 0 AND 255)  -- 0 unresolved, 1 catalog, 2 custom
) STRICT;

CREATE TABLE custom_exercise (        -- aggregate root, per-row LWW
  id              BLOB PRIMARY KEY CHECK (length(id) = 16),
  modality        INTEGER NOT NULL CHECK (modality BETWEEN 0 AND 255),
  cardio_activity INTEGER CHECK (cardio_activity BETWEEN 0 AND 65535),
  name            TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 120),
  based_on        BLOB CHECK (based_on IS NULL OR length(based_on) = 16)
  -- sync columns
) STRICT;

CREATE TABLE custom_exercise_target ( -- aggregate member
  exercise_id BLOB NOT NULL REFERENCES custom_exercise(id) ON DELETE CASCADE DEFERRABLE INITIALLY DEFERRED,
  muscle      INTEGER NOT NULL CHECK (muscle BETWEEN 0 AND 65535),
  role        INTEGER NOT NULL CHECK (role BETWEEN 0 AND 255),
  weight_bp   INTEGER CHECK (weight_bp BETWEEN 0 AND 10000),
  PRIMARY KEY (exercise_id, muscle)
) STRICT, WITHOUT ROWID;
```

Equipment profiles follow the same root + member shape
(`equipment_profile`, `equipment_profile_item(profile_id, equipment, max_load_g)`,
one default enforced by a partial unique index).

### 4.4 Training log

```sql
CREATE TABLE workout_session (        -- sync root, per-field LWW
  id                BLOB PRIMARY KEY CHECK (length(id) = 16),
  started_at        INTEGER NOT NULL,
  ended_at          INTEGER,
  day               INTEGER NOT NULL CHECK (day BETWEEN 0 AND 73000),
  tz_offset_min     INTEGER NOT NULL CHECK (tz_offset_min BETWEEN -840 AND 840),
  origin            INTEGER NOT NULL CHECK (origin BETWEEN 0 AND 255),
  routine_version_id BLOB,
  routine_day_id    BLOB,
  equipment_profile_id BLOB,
  pool_length_m     INTEGER CHECK (pool_length_m BETWEEN 10 AND 100),
  feel              INTEGER CHECK (feel BETWEEN 1 AND 5),
  import_batch_id   BLOB,
  note              TEXT CHECK (length(note) <= 2000)
  -- sync columns + field_hlc
) STRICT;
CREATE INDEX workout_session_day ON workout_session(day) WHERE deleted_at IS NULL;

CREATE TABLE set_entry (              -- sync root, per-row LWW
  id              BLOB PRIMARY KEY CHECK (length(id) = 16),
  session_id      BLOB NOT NULL CHECK (length(session_id) = 16),  -- no FK (sync roots)
  day             INTEGER NOT NULL,                               -- copied from session for history scans
  position        INTEGER NOT NULL,
  exercise_id     BLOB NOT NULL CHECK (length(exercise_id) = 16),
  routine_item_id BLOB,
  kind            INTEGER NOT NULL CHECK (kind BETWEEN 0 AND 255),
  side            INTEGER NOT NULL DEFAULT 0 CHECK (side BETWEEN 0 AND 255),
  load_g          INTEGER CHECK (load_g BETWEEN 0 AND 4000000),
  assist_g        INTEGER CHECK (assist_g BETWEEN 0 AND 400000),
  bodyweight_g    INTEGER CHECK (bodyweight_g BETWEEN 0 AND 400000),
  reps            INTEGER CHECK (reps BETWEEN 0 AND 10000),
  rir             INTEGER CHECK (rir BETWEEN 0 AND 10),
  rpe_half        INTEGER CHECK (rpe_half BETWEEN 2 AND 20),
  duration_ms     INTEGER CHECK (duration_ms >= 0),
  distance_m      INTEGER CHECK (distance_m >= 0),
  hr_avg_bpm      INTEGER CHECK (hr_avg_bpm BETWEEN 25 AND 250),
  hr_max_bpm      INTEGER CHECK (hr_max_bpm BETWEEN 25 AND 250),
  stroke          INTEGER CHECK (stroke BETWEEN 0 AND 255),
  hr_zones        BLOB,                                           -- TLV zone → ms (was set_hr_zone)
  superset        INTEGER,
  rest_ms         INTEGER CHECK (rest_ms >= 0),
  completed_at    INTEGER,
  -- sync columns
  CHECK (rir IS NULL OR rpe_half IS NULL),
  CHECK (hr_max_bpm IS NULL OR hr_avg_bpm IS NULL OR hr_max_bpm >= hr_avg_bpm)
) STRICT, WITHOUT ROWID;
CREATE INDEX set_entry_session ON set_entry(session_id, position) WHERE deleted_at IS NULL;
CREATE INDEX set_entry_history ON set_entry(exercise_id, day) WHERE deleted_at IS NULL;

CREATE TABLE external_activity (      -- sync root; id = HMAC(user_key, source ‖ external_id)
  id          BLOB PRIMARY KEY CHECK (length(id) = 16),
  source      INTEGER NOT NULL CHECK (source BETWEEN 0 AND 255),
  external_id TEXT NOT NULL CHECK (length(external_id) <= 200),
  session_id  BLOB NOT NULL,
  imported_at INTEGER NOT NULL
  -- sync columns
) STRICT;

CREATE TABLE import_batch (
  id BLOB PRIMARY KEY CHECK (length(id) = 16),
  source INTEGER NOT NULL, file_sha256 BLOB, started_at INTEGER NOT NULL,
  rows INTEGER NOT NULL DEFAULT 0, status INTEGER NOT NULL
  -- sync columns
) STRICT;
```

Edits to a set are LWW updates; the server keeps an append-only revision
history (§6).

### 4.5 Body

```sql
CREATE TABLE body_measurement (       -- sync root, per-row LWW
  id            BLOB PRIMARY KEY CHECK (length(id) = 16),
  measured_at   INTEGER NOT NULL,
  day           INTEGER NOT NULL,
  tz_offset_min INTEGER NOT NULL,
  kind          INTEGER NOT NULL CHECK (kind BETWEEN 0 AND 65535),  -- catalog measurement_kind
  side          INTEGER NOT NULL DEFAULT 0 CHECK (side BETWEEN 0 AND 2),
  method        INTEGER NOT NULL DEFAULT 0 CHECK (method BETWEEN 0 AND 255),
  value         INTEGER NOT NULL,                                   -- unit from measurement_kind
  source        INTEGER NOT NULL CHECK (source BETWEEN 0 AND 255),
  external_ref  TEXT CHECK (length(external_ref) <= 200),
  import_batch_id BLOB,
  note          TEXT CHECK (length(note) <= 500)
  -- sync columns
) STRICT, WITHOUT ROWID;
CREATE INDEX body_measurement_kind_day ON body_measurement(kind, day) WHERE deleted_at IS NULL;
```

### 4.6 Routines (single writer per routine, immutable versions)

```sql
CREATE TABLE routine (                -- head; sync root, per-field LWW; owner is the only writer
  id                 BLOB PRIMARY KEY CHECK (length(id) = 16),
  owner_user_id      BLOB,             -- professional for assigned routines
  name               TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 120),
  current_version_id BLOB NOT NULL,
  upstream_id        BLOB,             -- routine it was copied / forked from
  follow_mode        INTEGER CHECK (follow_mode BETWEEN 0 AND 255),  -- own, copy, follow
  followed_version_id BLOB
  -- sync columns + field_hlc
) STRICT;

CREATE TABLE routine_version (        -- sync root, immutable after creation
  id BLOB PRIMARY KEY CHECK (length(id) = 16),
  routine_id BLOB NOT NULL, n INTEGER NOT NULL, created_at INTEGER NOT NULL,
  content BLOB NOT NULL               -- TLV: days → blocks → items
  -- sync columns
) STRICT;
```

Days, blocks and items live inside the immutable `content`, so a version
never conflicts; editing creates a new version and moves
`current_version_id`. A client that edits a professional's routine forks it
(`upstream_id`) and is offered upstream updates as a rebase. Query-time
access to items (progression, adherence) goes through `derived.db`.

### 4.7 Plan and calibration

```sql
CREATE TABLE plan_phase (
  id BLOB PRIMARY KEY CHECK (length(id) = 16),
  starts_on INTEGER NOT NULL, ends_on INTEGER NOT NULL,
  kind INTEGER NOT NULL CHECK (kind BETWEEN 0 AND 255),
  volume_mult_bp INTEGER NOT NULL DEFAULT 10000 CHECK (volume_mult_bp BETWEEN 0 AND 30000),
  intensity_mult_bp INTEGER NOT NULL DEFAULT 10000 CHECK (intensity_mult_bp BETWEEN 0 AND 30000),
  set_by_user_id BLOB,
  CHECK (ends_on >= starts_on)
  -- sync columns
) STRICT;

CREATE TABLE plan_event (
  id BLOB PRIMARY KEY CHECK (length(id) = 16),
  on_day INTEGER NOT NULL, kind INTEGER NOT NULL, priority INTEGER NOT NULL,
  name TEXT NOT NULL CHECK (length(name) <= 120)
  -- sync columns
) STRICT;

CREATE TABLE calibration_result (     -- what was declared / measured, never a computed e1RM
  id BLOB PRIMARY KEY CHECK (length(id) = 16),
  set_id BLOB, exercise_id BLOB NOT NULL,
  protocol INTEGER NOT NULL, kind INTEGER NOT NULL,
  value INTEGER NOT NULL, unit INTEGER NOT NULL, rir_declared INTEGER
  -- sync columns
) STRICT;

CREATE TABLE volume_range_override (  -- id = HMAC(user_key, muscle ‖ goal ‖ level)
  id BLOB PRIMARY KEY CHECK (length(id) = 16),
  muscle INTEGER NOT NULL, goal INTEGER NOT NULL, level INTEGER NOT NULL,
  min_milli INTEGER NOT NULL, max_milli INTEGER NOT NULL,
  set_by_user_id BLOB, set_at INTEGER NOT NULL,
  UNIQUE (muscle, goal, level),
  CHECK (max_milli >= min_milli)
  -- sync columns
) STRICT;
```

### 4.8 Diet

```sql
CREATE TABLE meal (                   -- sync root, per-field LWW
  id            BLOB PRIMARY KEY CHECK (length(id) = 16),
  eaten_at      INTEGER NOT NULL,
  day           INTEGER NOT NULL,
  tz_offset_min INTEGER NOT NULL,
  slot          INTEGER CHECK (slot BETWEEN 0 AND 255),
  mode          INTEGER NOT NULL CHECK (mode BETWEEN 0 AND 255),
  plan_meal_id  BLOB,
  quick_size    INTEGER CHECK (quick_size BETWEEN 0 AND 255),
  quick_protein INTEGER CHECK (quick_protein BETWEEN 0 AND 255),
  import_batch_id BLOB,
  note          TEXT CHECK (length(note) <= 500)
  -- sync columns + field_hlc
) STRICT;
CREATE INDEX meal_day ON meal(day) WHERE deleted_at IS NULL;

CREATE TABLE meal_item (              -- sync root, per-row LWW
  id            BLOB PRIMARY KEY CHECK (length(id) = 16),
  meal_id       BLOB NOT NULL CHECK (length(meal_id) = 16),
  position      INTEGER NOT NULL,
  food_src      INTEGER NOT NULL CHECK (food_src BETWEEN 1 AND 4),  -- catalog, off, custom, recipe
  food_id       BLOB,
  barcode       TEXT CHECK (length(barcode) <= 32),
  grams_mg      INTEGER NOT NULL CHECK (grams_mg > 0),
  portion_id    BLOB,
  portion_qty_milli INTEGER,
  name_snapshot TEXT NOT NULL CHECK (length(name_snapshot) <= 200),
  brand_snapshot TEXT CHECK (length(brand_snapshot) <= 120),
  source_version INTEGER,                            -- catalog / OFF version of the snapshot
  nutrients     BLOB NOT NULL,                       -- TLV nutrient code → amount per 100 g (unit per nutrient)
  CHECK ((food_src = 2) = (barcode IS NOT NULL)),
  CHECK ((food_src = 2) = (food_id IS NULL))
  -- sync columns
) STRICT, WITHOUT ROWID;
CREATE INDEX meal_item_meal ON meal_item(meal_id, position) WHERE deleted_at IS NULL;
```

Totals are always `grams × density`, so editing grams never leaves stale
nutrients. Custom foods (+ portions) and recipes (+ items) are aggregate
roots with members, like custom exercises. Nutrition plans follow the routine
shape (head + immutable versions; per-day-type targets inside the version).

### 4.9 Suggestions

```sql
CREATE TABLE suggestion (             -- immutable after creation
  id BLOB PRIMARY KEY CHECK (length(id) = 16),
  created_at INTEGER NOT NULL,
  rule INTEGER NOT NULL, rule_version INTEGER NOT NULL, model INTEGER NOT NULL,
  muscle INTEGER, kind INTEGER NOT NULL,
  confidence_bp INTEGER NOT NULL CHECK (confidence_bp BETWEEN 0 AND 10000),
  inputs BLOB NOT NULL                -- TLV of the numbers used
  -- sync columns
) STRICT;

CREATE TABLE suggestion_response (    -- append-only; latest wins for display
  id BLOB PRIMARY KEY CHECK (length(id) = 16),
  suggestion_id BLOB NOT NULL,
  response INTEGER NOT NULL, reason INTEGER, at INTEGER NOT NULL
  -- sync columns
) STRICT;
```

### 4.10 Attachments, reminders

`attachment(id, owner_entity, owner_id, kind, mime, sha256, size, state,
created_at)` — blobs stored as files encrypted with a key derived from the DB
key, excluded from backup. `reminder(id, kind, rrule, enabled)` local only.

## 5. `derived.db`

```sql
CREATE TABLE derived_meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL) STRICT;  -- schema, model, formula, watermark
CREATE TABLE dirty (entity INTEGER NOT NULL, row_id BLOB NOT NULL, PRIMARY KEY (entity, row_id)) STRICT, WITHOUT ROWID;
CREATE TABLE set_load (set_id BLOB, model INTEGER, muscle INTEGER, value_milli INTEGER, flags INTEGER,
  PRIMARY KEY (set_id, model, muscle)) STRICT, WITHOUT ROWID;
CREATE TABLE e1rm (exercise_id BLOB, day INTEGER, formula INTEGER, value_g INTEGER, set_id BLOB, flags INTEGER,
  PRIMARY KEY (exercise_id, day, formula)) STRICT, WITHOUT ROWID;
CREATE TABLE week_volume (week_day INTEGER, model INTEGER, muscle INTEGER, left_milli INTEGER, right_milli INTEGER,
  PRIMARY KEY (week_day, model, muscle)) STRICT, WITHOUT ROWID;
CREATE TABLE personal_record (exercise_id BLOB, kind INTEGER, formula INTEGER, value INTEGER, set_id BLOB, achieved_at INTEGER,
  PRIMARY KEY (exercise_id, kind, formula)) STRICT, WITHOUT ROWID;
CREATE TABLE routine_item_index (version_id BLOB, item_id BLOB, exercise_id BLOB, PRIMARY KEY (version_id, item_id)) STRICT, WITHOUT ROWID;
```

`dirty` is filled from `user.db`'s `local_seq` watermark, never from HLC.

## 6. `catalog.db` (*sketch*, built in CI)

- `catalog_meta(key, value)`: version, schema_version, built_at, signature.
- `source(id, name, license, attribution, url, version, sha256)`.
- `muscle(code PK, parent_code, region, lateral)` — wire codes, independent of
  the Rust array index; subzones are children.
- `exercise(id PK = UUIDv5(slug), slug UNIQUE, modality, cardio_activity,
  level, mechanic, force, unilateral, status, replaced_by)`; never deleted,
  retired rows point to `replaced_by` (acyclic).
- `exercise_name(rid INTEGER PRIMARY KEY, exercise_id, lang, name, aliases,
  UNIQUE(exercise_id, lang))` + `exercise_search` FTS5
  (`content_rowid='rid'`, `unicode61 remove_diacritics 2`, `prefix='2 3'`,
  `lang UNINDEXED`), optimized in CI.
- `exercise_target`, `exercise_equipment`, `exercise_alternative`,
  `exercise_text(lang, kind, position)`, `exercise_media`,
  `exercise_provenance(exercise_id, source_id, external_id)`.
- `cardio_factor(activity, muscle, factor_bp, version)`,
  `volume_range(goal, level, muscle, min_milli, max_milli, version)`,
  `measurement_kind(code, unit, lateral, min, max, scale)`.
- `food(id, kind, status, replaced_by)` + `food_nutrient(food_id, nutrient,
  amount_100g)` as the only nutrient source; `nutrient(code, unit, name)`;
  `food_name` + `food_search` (same FTS shape), `food_portion`,
  `food_ingredient`, `food_provenance`.
- `off-<cc>.db`: `product(barcode PK, name, brand, quantity, nutrients TLV,
  off_last_modified)` + FTS; license and attribution in `meta`.

## 7. Server (PostgreSQL) (*sketch*)

- Synced tables mirror the device rows with `user_id uuid NOT NULL`,
  **PK `(user_id, id)`**, `server_rev bigint`, `hlc bigint`, `node int`,
  `received_at timestamptz`, `deleted_at timestamptz`, `extra bytea`;
  `day` as `date`; codes `smallint CHECK (x BETWEEN 0 AND 255)` with
  `code_label(domain, code, label)` for analytics; no native ENUMs; no FKs
  between sync roots; composite `(user_id, …)` keys everywhere.
- Revisions: `user_rev(user_id PK, rev bigint, tombstone_horizon bigint)`
  locked by `UPDATE … RETURNING` inside each push transaction — commit-ordered,
  gap-free per user (a global sequence silently drops rows from feeds).
- Own data is pulled per table with `(user_id, server_rev)` indexes, merged
  by rev. Cross-user deliveries (assigned routines / plans, consent mirror)
  go through a small `delivery_feed(recipient, stream, rev, entity, owner, row_id)`.
- LWW enforced in the upsert:
  `ON CONFLICT (user_id, id) DO UPDATE … WHERE (excluded.hlc, excluded.node) > (t.hlc, t.node)`;
  deletes sticky; pushes with `hlc` physical part > `now + 60 s` rejected;
  `push_receipt(user_id, op_id)` (30-day TTL) for idempotency; `user_id`
  only from the auth token; revoked devices refused; per-user rate limit,
  ≤ 500 ops / 256 KB per batch.
- `set_entry_revision(user_id, set_id, hlc, payload, at)` append-only.
- Partitioning from day one: `HASH (user_id)`, 128 partitions, same modulus on
  `set_entry`, `workout_session`, `meal`, `meal_item`, `body_measurement`,
  `suggestion*`; `enable_partitionwise_join`; parents ANALYZEd by cron;
  audit / consent events range-partitioned by month.
- RLS: app role non-owner `NOBYPASSRLS`; `FORCE ROW LEVEL SECURITY`
  everywhere; partitions revoked; views `security_invoker`; every request is
  an explicit transaction starting with
  `set_config('app.user_id', $1, true)`; policies are plain equalities on
  `current_setting(..., true)` (missing = no rows). Professional reads go
  through `SECURITY DEFINER` functions (pinned `search_path`) that check
  consent, write `access_audit`, then set `app.subject_id`. CI test asserts
  every table has policies + FORCE.
- Consent: server command, never LWW. `consent_grant(client_id,
  professional_id, org_id, category, access, data_from, expires_at,
  terms_version, granted_at, revoked_at)` + append-only `consent_event`;
  static category → table map, default deny; read from the primary.
  Professional client data is read server-side only, never synced to the
  professional's device.
- `client_summary(user_id PK, last_session_at, week_adherence_bp,
  alert_flags, updated_rev)` maintained by an ingest worker for dashboards.
- Field encryption: per-user DEK wrapped by a KMS KEK for free text
  (`note`, names, `suggestion.inputs`), email (`email_enc` + KMS-peppered
  HMAC lookup); destroying the DEK on erasure crypto-shreds backups.
- Erasure: request freezes sync, revokes consents, links and dataset
  inclusion at once; after the grace period, batched deletes per table +
  DEK destruction; documented backup retention window.
- Analytics: separate cluster fed by an export job; random per-user
  pseudonym in a deletable `analytics_subject` table; dates shifted per
  subject, coarsened, free text and external ids dropped; k ≥ 10 per release.

## 8. Sync protocol

1. Push: `POST /sync/push {device_id, proto, hlc_now, ops[op_id, seq, entity,
   row_id, op, hlc, node, payload_ver, tlv]}` in `seq` order.
2. Server: one transaction per batch — receipts, LWW (per row or per field
   group), sticky deletes, bump `user_rev`, write rows + receipts.
3. Response per op: `applied(rev)` | `superseded(rev)` | `rejected(code)`.
   Device deletes by `op_id`; sets `server_rev` only if the row's hlc still
   equals the pushed one; `superseded` triggers a pull first.
4. Pull: `GET /sync/pull?stream&after&limit` →
   `{rows, next, horizon, server_time, min_proto}`; applied in one
   transaction with deferred FKs; remote wins when `(hlc, node)` is greater;
   HLC receive rule advances the local clock; a remote win drops the pending
   outbox entry.
5. Payload: TLV `(tag varint · length varint · bytes)*`, LEB128 varints,
   tags strictly increasing (`crates/store/src/tlv.rs`); tags
   append-only; unknown tags kept in `extra` and sent back unchanged.
   Below `min_proto` a client is read-only until it updates.
6. Bootstrap / cursor below `horizon` / lost key: read `user_rev.rev = H`,
   page each table by keyset (topological order, no tombstones), set cursors
   to `H`, pull, then push local pre-login data.
7. `failed` outbox rows are permanent and shown to the user; the rest of the
   queue keeps flowing; transient errors back off with jitter.
8. Account deleted → push answers 410 → device wipes.

## 9. Product rules that shape the data

- Accounts are optional. Without one, data lives only on the device; the app
  offers an encrypted export file (`.myos`, password-derived key, Argon2id)
  that the user stores anywhere and can import on a new device. With an
  account, a new device restores from the server.
- Minimum age 16; no parental-consent flow.
- Professionals read client data online only (server-side, audited); their
  own notes, routines and plans work offline.

## 10. Implementation notes (v1)

Where `crates/store` refines or departs from the sections above.

- **Migrations stay editable until the first release.** Nothing ships yet, so
  `0001`/`0002` are amended in place; after release they are frozen and only
  appended to.
- **Merge granularity.** v1 writes whole rows: every local write sets
  `field_hlc = NULL` ("every field at the row hlc"), so all synced tables
  merge per row. Per-field merge for header tables (`profile`, `preferences`,
  `workout_session`, `meal`, `routine`) needs the sync payload tags and lands
  with the sync protocol.
- **Outbox order.** A coalesced write moves its entry to the end of the queue
  (`seq INTEGER PRIMARY KEY AUTOINCREMENT`, never reused) with a fresh
  `op_id`; writers enqueue a parent after the children it references (meal
  header after its items, routine head after its new version).
- **Restores.** An explicit write to a tombstoned natural-key row
  (`profile`, `preferences`, `profile_goal`) clears `deleted_at` and queues
  `op = restore`.
- **Strict structured columns.** TLV decoders of nutrients and routine
  content reject tags or codes they do not know: the row reads as
  undecodable (or the routine fails to open) instead of losing data when an
  older build rewrites it. TLV varints must be canonical and tag 0 is
  reserved.
- **Meal item generations.** `meal.items_gen` / `meal_item.generation`
  (16-byte ids): replacing a meal's items mints a new generation and only
  items of the header's current generation count, so concurrent replacements
  from two devices end with one list, not both. `meal_item.portion_label`
  replaces `portion_id` / `portion_qty_milli`.
- **Extra tables in 0002.** `custom_food` (+ `custom_food_portion`),
  `recipe` (+ `recipe_item`), `nutrition_plan` + `nutrition_plan_version`
  (head + immutable TLV content, like routines), `equipment_profile`
  (+ item, no unique default), `attachment`, `reminder`. No unique index on
  `volume_range_override` or `external_activity`: rows minted under
  different pre-login keys coexist until the login rewrite; reads take the
  newest by `(hlc, node)`.
- **Routine versions.** `routine_version.n` is not unique across devices;
  order by `(n, hlc, node)`. A head whose current version has not synced yet
  reads the newest version present. `Copy` keeps `upstream_id` but no
  `followed_version_id`; only `Follow` tracks the upstream version.
- **Calibration.** `calibration_result` stores `protocol`, `value` and
  `unit` (grams, reps, ms, meters); there is no separate `kind`.
- **`derived.db` (schema 1).** `derived_meta` (schema, watermark on
  `local_seq`, load model, catalog version), `e1rm(exercise_id, day,
  formula, value_g, session_id, flags)`, `week_volume(week_day, model,
  muscle, left_milli, right_milli)` and `week_summary` (counted, assumed and
  skipped set counts). Refresh recomputes the days touched since the
  watermark (sets, sessions, and every day using a changed user exercise)
  and every week from the earliest of them on, since a new estimate moves
  later intensity references. A different load model or catalog version, or
  an unreadable file, rebuilds everything.
- **Reads.** Multi-statement reads (`sessions_in`, `meals_in`) run in one
  read transaction. Rows with unknown codes, out-of-range values or invalid
  instants are counted as undecodable, never fatal; body measurements are
  re-validated on read.
- **Sync client (protocol 1, payload layout 1).**
  - Payload tags: 1 `deleted_at`, 10.. data columns in `crates/store/src/spec.rs`
    order (pinned by `tests/golden/payload_tags.txt`, append-only), 999
    reserved for `field_hlc`, 1000 aggregate members (list of member TLVs,
    unknown member tags kept in the member's `extra`), > 1000 reserved. An
    absent tag on a `NOT NULL DEFAULT` column means the default.
  - Nothing is pushed before `attach_account`, which rewrites natural-key ids
    with the account key and bumps `id_epoch`.
  - The first write of a natural-key or singleton row is a `restore` (it may
    exist tombstoned on the server).
  - Pull: the server is the authority. A row without an unpushed local change
    is applied as is; with one, deletes win unless restored, otherwise the
    greater `(hlc, node)`; the winner drops the local outbox entry. A pulled
    row equal to the local `(hlc, node)` only records `server_rev`.
  - Each pulled row is applied in a savepoint; unknown entity, undecodable
    payload or a failed constraint sends it to `sync_quarantine` (with op,
    hlc, node) and marks local changes of that row as failed "needs update";
    the cursor still advances.
  - Remote applies bump `local_seq`; a row leaving a `day` records it in the
    local `derived_touched_day` so the derived cache recomputes it.
  - Clock: offset to the server clamped to ±24 h; remote clocks more than
    5 min ahead are not adopted. `min_proto > PROTO` sets `read_only` and
    stops applying pulls until an update clears it.
  - Transport failures requeue in-flight entries with exponential backoff and
    jitter (an explicit restore survives the requeue); entries left in flight
    by a crash are requeued at open.
- **Key storage (`crates/vault`).** `keyring-core` with one store per target:
  Secret Service (zbus) on Linux/BSD, legacy Keychain on macOS (needs a
  stable signing identity, or each update prompts), data-protection Keychain
  with `after-first-unlock-this-device-only` on iOS, Credential Manager with
  `persistence = Local` on Windows, Android `SharedPreferences` encrypted by
  a Keystore key (no StrongBox request, no user auth; needs `ndk-context`,
  so processes without an activity must not open it). The key is stored as
  hex (some stores keep only UTF-8). A stored value that cannot be decrypted
  or decoded counts as lost: it is deleted, the database files are deleted
  and the app reports `Reset::KeyLost` so the UI offers a restore.
- **Database location.** Android `istmo::path::state_dir()` (=
  `noBackupFilesDir`), elsewhere `data_local_dir()`, under `database/`.
  Backups: Android `res/xml/data_extraction_rules.xml` + `backup_rules.xml`
  exclude the wrapped key (`sharedpref/keyring-default.xml`); iOS / macOS
  flag `database/` with `NSURLIsExcludedFromBackupKey` on every open
  (`crates/backup`).
- **Windows build gap.** `openssl-src` builds MSVC targets with `nmake`, so
  the vendored OpenSSL behind SQLCipher cannot be cross-built from Linux;
  the Windows target needs a prebuilt or separately built static libcrypto
  (`OPENSSL_LIB_DIR` / `OPENSSL_INCLUDE_DIR` with `bundled-sqlcipher`).
- **Exports, passphrases, logout.** `.myos` = the whole `user.db` copied by
  `sqlcipher_export` under a passphrase (SQLCipher KDF: PBKDF2-HMAC-SHA512,
  600 000 iterations, the file's own salt — replaces the Argon2id note in
  §9; no extra crypto dependency). `cipher_compatibility`, `kdf_iter`,
  `auto_vacuum` and `user_version` are set on the export explicitly.
  Import opens it with the passphrase, copies it into a new file under the
  device key, rotates `device_id` / `node`, clears the quarantine and keeps
  the outbox (re-sent ops are answered by receipt). Passphrases have at
  least 8 characters and may not look like an `x'…'` raw key. A
  passphrase-unlocked `user.db` (no credential store) keeps a random raw key
  for `derived.db` in `local_secret`, so opening runs the KDF once;
  `Store::rekey` moves between passphrase and device key. Logout drops the
  stored key first, then deletes `user.db*` and `user.derived.db*`.
