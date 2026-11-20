-- Sync bookkeeping, profile, exercises, training log and body measurements.
--
-- Sync columns on every sync root:
--   hlc        last write, 48-bit ms + 16-bit counter
--   node       writer node (tiebreak for equal hlc)
--   server_rev per-user revision acked by the server
--   local_seq  bumped on every local or remote write (derived-data watermark)
--   deleted_at tombstone; deletes win
--   extra      TLV fields this app version does not know, sent back unchanged
--   field_hlc  TLV tag -> hlc for tables merged per field; NULL means every
--              field was last written at the row hlc
-- Enum columns only get range checks: newer app versions add codes.
-- Instants are Unix milliseconds within 1970 ..= 9999.

CREATE TABLE sync_state (
  k                INTEGER PRIMARY KEY CHECK (k = 1),
  device_id        BLOB NOT NULL CHECK (length(device_id) = 16),
  node             INTEGER NOT NULL,
  user_id          BLOB CHECK (user_id IS NULL OR length(user_id) = 16),
  -- Key for deterministic ids of natural keys; device-local until an
  -- account exists, then replaced by the account key (id_epoch bumps).
  user_key         BLOB NOT NULL CHECK (length(user_key) = 32),
  id_epoch         INTEGER NOT NULL DEFAULT 0,
  hlc_last         INTEGER NOT NULL,
  server_offset_ms INTEGER NOT NULL DEFAULT 0,
  local_seq        INTEGER NOT NULL DEFAULT 0,
  proto            INTEGER NOT NULL DEFAULT 1,
  -- Set when the server requires a newer protocol: local writes refused.
  read_only        INTEGER NOT NULL DEFAULT 0 CHECK (read_only IN (0, 1))
) STRICT;

-- One entry per row with unpushed changes. The payload is built from the row
-- when pushing, so coalesced writes always send the latest state.
CREATE TABLE sync_outbox (
  seq        INTEGER PRIMARY KEY,
  op_id      BLOB NOT NULL UNIQUE CHECK (length(op_id) = 16),
  entity     INTEGER NOT NULL CHECK (entity BETWEEN 1 AND 65535),
  row_id     BLOB NOT NULL CHECK (length(row_id) = 16),
  op         INTEGER NOT NULL CHECK (op BETWEEN 1 AND 3),
  hlc        INTEGER NOT NULL,
  state      INTEGER NOT NULL DEFAULT 0 CHECK (state BETWEEN 0 AND 2),
  attempts   INTEGER NOT NULL DEFAULT 0,
  error_code INTEGER,
  next_attempt_at INTEGER,
  created_at INTEGER NOT NULL
) STRICT;
CREATE UNIQUE INDEX sync_outbox_pending ON sync_outbox (entity, row_id) WHERE state = 0;

CREATE TABLE sync_cursor (
  stream     INTEGER PRIMARY KEY,
  cursor     INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
) STRICT;

CREATE TABLE sync_quarantine (
  stream      INTEGER NOT NULL,
  server_rev  INTEGER NOT NULL,
  entity      INTEGER NOT NULL,
  row_id      BLOB NOT NULL CHECK (length(row_id) = 16),
  payload_ver INTEGER NOT NULL,
  payload     BLOB,
  reason      INTEGER NOT NULL,
  PRIMARY KEY (stream, server_rev)
) STRICT;

CREATE TABLE profile (
  id         BLOB PRIMARY KEY CHECK (length(id) = 16),
  birth_year INTEGER CHECK (birth_year BETWEEN 1900 AND 2100),
  sex        INTEGER CHECK (sex BETWEEN 0 AND 255),
  height_mm  INTEGER CHECK (height_mm BETWEEN 500 AND 2800),
  field_hlc  BLOB,
  hlc INTEGER NOT NULL, node INTEGER NOT NULL, server_rev INTEGER,
  local_seq INTEGER NOT NULL, deleted_at INTEGER, extra BLOB
) STRICT;

CREATE TABLE profile_goal (
  id       BLOB PRIMARY KEY CHECK (length(id) = 16),
  from_day INTEGER NOT NULL CHECK (from_day BETWEEN 0 AND 73000),
  goal     INTEGER NOT NULL CHECK (goal BETWEEN 0 AND 255),
  level    INTEGER NOT NULL CHECK (level BETWEEN 0 AND 255),
  hlc INTEGER NOT NULL, node INTEGER NOT NULL, server_rev INTEGER,
  local_seq INTEGER NOT NULL, deleted_at INTEGER, extra BLOB
) STRICT;
CREATE INDEX profile_goal_from ON profile_goal (from_day) WHERE deleted_at IS NULL;

CREATE TABLE preferences (
  id            BLOB PRIMARY KEY CHECK (length(id) = 16),
  theme         INTEGER NOT NULL DEFAULT 0 CHECK (theme BETWEEN 0 AND 255),
  unit_mass     INTEGER NOT NULL DEFAULT 0 CHECK (unit_mass BETWEEN 0 AND 255),
  unit_distance INTEGER NOT NULL DEFAULT 0 CHECK (unit_distance BETWEEN 0 AND 255),
  unit_energy   INTEGER NOT NULL DEFAULT 0 CHECK (unit_energy BETWEEN 0 AND 255),
  effort_scale  INTEGER NOT NULL DEFAULT 0 CHECK (effort_scale BETWEEN 0 AND 255),
  diet_mode     INTEGER NOT NULL DEFAULT 0 CHECK (diet_mode BETWEEN 0 AND 255),
  locale        TEXT CHECK (length(locale) <= 35),
  field_hlc     BLOB,
  hlc INTEGER NOT NULL, node INTEGER NOT NULL, server_rev INTEGER,
  local_seq INTEGER NOT NULL, deleted_at INTEGER, extra BLOB
) STRICT;

CREATE TABLE custom_exercise (
  id              BLOB PRIMARY KEY CHECK (length(id) = 16),
  modality        INTEGER NOT NULL CHECK (modality BETWEEN 0 AND 255),
  cardio_activity INTEGER CHECK (cardio_activity BETWEEN 0 AND 65535),
  name            TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 120),
  based_on        BLOB CHECK (based_on IS NULL OR length(based_on) = 16),
  hlc INTEGER NOT NULL, node INTEGER NOT NULL, server_rev INTEGER,
  local_seq INTEGER NOT NULL, deleted_at INTEGER, extra BLOB
) STRICT;
CREATE INDEX custom_exercise_seq ON custom_exercise (local_seq);

CREATE TABLE custom_exercise_target (
  exercise_id BLOB NOT NULL
    REFERENCES custom_exercise (id) ON DELETE CASCADE DEFERRABLE INITIALLY DEFERRED,
  muscle      INTEGER NOT NULL CHECK (muscle BETWEEN 0 AND 65535),
  role        INTEGER NOT NULL CHECK (role BETWEEN 0 AND 255),
  weight_bp   INTEGER CHECK (weight_bp BETWEEN 0 AND 10000),
  PRIMARY KEY (exercise_id, muscle)
) STRICT, WITHOUT ROWID;

-- Local index of every exercise referenced, rebuilt from synced rows. The
-- source follows from the id: catalog ids are UUID v5, user and
-- organization exercises v7. No foreign key: sets may arrive before the
-- custom exercise they use.
CREATE TABLE exercise_ref (
  id  BLOB PRIMARY KEY CHECK (length(id) = 16),
  src INTEGER NOT NULL CHECK (src BETWEEN 0 AND 255)
) STRICT;

CREATE TABLE import_batch (
  id          BLOB PRIMARY KEY CHECK (length(id) = 16),
  source      INTEGER NOT NULL CHECK (source BETWEEN 0 AND 255),
  file_sha256 BLOB CHECK (file_sha256 IS NULL OR length(file_sha256) = 32),
  started_at  INTEGER NOT NULL,
  rows        INTEGER NOT NULL DEFAULT 0,
  status      INTEGER NOT NULL CHECK (status BETWEEN 0 AND 255),
  hlc INTEGER NOT NULL, node INTEGER NOT NULL, server_rev INTEGER,
  local_seq INTEGER NOT NULL, deleted_at INTEGER, extra BLOB
) STRICT;

CREATE TABLE workout_session (
  id                   BLOB PRIMARY KEY CHECK (length(id) = 16),
  started_at           INTEGER NOT NULL CHECK (started_at BETWEEN 0 AND 253402300799999),
  ended_at             INTEGER CHECK (ended_at BETWEEN 0 AND 253402300799999),
  day                  INTEGER NOT NULL CHECK (day BETWEEN 0 AND 73000),
  tz_offset_min        INTEGER NOT NULL CHECK (tz_offset_min BETWEEN -840 AND 840),
  origin               INTEGER NOT NULL CHECK (origin BETWEEN 0 AND 255),
  routine_version_id   BLOB CHECK (routine_version_id IS NULL OR length(routine_version_id) = 16),
  routine_day_id       BLOB CHECK (routine_day_id IS NULL OR length(routine_day_id) = 16),
  equipment_profile_id BLOB CHECK (equipment_profile_id IS NULL OR length(equipment_profile_id) = 16),
  pool_length_m        INTEGER CHECK (pool_length_m BETWEEN 10 AND 100),
  feel                 INTEGER CHECK (feel BETWEEN 1 AND 5),
  import_batch_id      BLOB CHECK (import_batch_id IS NULL OR length(import_batch_id) = 16),
  note                 TEXT CHECK (length(note) <= 2000),
  field_hlc            BLOB,
  hlc INTEGER NOT NULL, node INTEGER NOT NULL, server_rev INTEGER,
  local_seq INTEGER NOT NULL, deleted_at INTEGER, extra BLOB,
  CHECK (ended_at IS NULL OR ended_at >= started_at)
) STRICT;
CREATE INDEX workout_session_day ON workout_session (day) WHERE deleted_at IS NULL;
CREATE INDEX workout_session_seq ON workout_session (local_seq);

-- No foreign key to workout_session: both are sync roots and may arrive in
-- any order. A set is live only while its session is.
CREATE TABLE set_entry (
  id              BLOB PRIMARY KEY CHECK (length(id) = 16),
  session_id      BLOB NOT NULL CHECK (length(session_id) = 16),
  day             INTEGER NOT NULL CHECK (day BETWEEN 0 AND 73000),
  position        INTEGER NOT NULL,
  exercise_id     BLOB NOT NULL CHECK (length(exercise_id) = 16),
  routine_item_id BLOB CHECK (routine_item_id IS NULL OR length(routine_item_id) = 16),
  kind            INTEGER NOT NULL CHECK (kind BETWEEN 0 AND 255),
  side            INTEGER NOT NULL CHECK (side BETWEEN 0 AND 255),
  load_g          INTEGER CHECK (load_g BETWEEN 0 AND 4000000),
  assist_g        INTEGER CHECK (assist_g BETWEEN 0 AND 400000),
  bodyweight_g    INTEGER CHECK (bodyweight_g BETWEEN 0 AND 400000),
  reps            INTEGER CHECK (reps BETWEEN 0 AND 10000),
  rir             INTEGER CHECK (rir BETWEEN 0 AND 10),
  rpe_half        INTEGER CHECK (rpe_half BETWEEN 2 AND 20),
  duration_ms     INTEGER CHECK (duration_ms BETWEEN 0 AND 604800000),
  distance_m      INTEGER CHECK (distance_m BETWEEN 0 AND 4294967295),
  hr_avg_bpm      INTEGER CHECK (hr_avg_bpm BETWEEN 25 AND 250),
  hr_max_bpm      INTEGER CHECK (hr_max_bpm BETWEEN 25 AND 250),
  stroke          INTEGER CHECK (stroke BETWEEN 0 AND 255),
  hr_zones        BLOB,
  superset        INTEGER CHECK (superset BETWEEN 0 AND 65535),
  rest_ms         INTEGER CHECK (rest_ms BETWEEN 0 AND 86400000),
  completed_at    INTEGER,
  hlc INTEGER NOT NULL, node INTEGER NOT NULL, server_rev INTEGER,
  local_seq INTEGER NOT NULL, deleted_at INTEGER, extra BLOB,
  CHECK (rir IS NULL OR rpe_half IS NULL),
  CHECK (hr_max_bpm IS NULL OR hr_avg_bpm IS NULL OR hr_max_bpm >= hr_avg_bpm)
) STRICT, WITHOUT ROWID;
CREATE INDEX set_entry_session ON set_entry (session_id, position) WHERE deleted_at IS NULL;
CREATE INDEX set_entry_history ON set_entry (exercise_id, day) WHERE deleted_at IS NULL;
CREATE INDEX set_entry_day ON set_entry (day) WHERE deleted_at IS NULL;
CREATE INDEX set_entry_seq ON set_entry (local_seq);

CREATE TABLE external_activity (
  id          BLOB PRIMARY KEY CHECK (length(id) = 16),
  source      INTEGER NOT NULL CHECK (source BETWEEN 0 AND 255),
  external_id TEXT NOT NULL CHECK (length(external_id) BETWEEN 1 AND 200),
  session_id  BLOB NOT NULL CHECK (length(session_id) = 16),
  imported_at INTEGER NOT NULL,
  hlc INTEGER NOT NULL, node INTEGER NOT NULL, server_rev INTEGER,
  local_seq INTEGER NOT NULL, deleted_at INTEGER, extra BLOB
) STRICT;
-- Not unique: rows minted under different pre-login keys may coexist until
-- the login rewrite merges them; a unique index would abort pulls.
CREATE INDEX external_activity_key ON external_activity (source, external_id)
  WHERE deleted_at IS NULL;

CREATE TABLE body_measurement (
  id              BLOB PRIMARY KEY CHECK (length(id) = 16),
  measured_at     INTEGER NOT NULL CHECK (measured_at BETWEEN 0 AND 253402300799999),
  day             INTEGER NOT NULL CHECK (day BETWEEN 0 AND 73000),
  tz_offset_min   INTEGER NOT NULL CHECK (tz_offset_min BETWEEN -840 AND 840),
  kind            INTEGER NOT NULL CHECK (kind BETWEEN 0 AND 65535),
  side            INTEGER NOT NULL DEFAULT 0 CHECK (side BETWEEN 0 AND 255),  -- 0 = none
  method          INTEGER NOT NULL DEFAULT 1 CHECK (method BETWEEN 0 AND 255),  -- 1 unspecified
  value           INTEGER NOT NULL,
  source          INTEGER NOT NULL CHECK (source BETWEEN 0 AND 255),
  external_ref    TEXT CHECK (length(external_ref) <= 200),
  import_batch_id BLOB CHECK (import_batch_id IS NULL OR length(import_batch_id) = 16),
  note            TEXT CHECK (length(note) <= 500),
  hlc INTEGER NOT NULL, node INTEGER NOT NULL, server_rev INTEGER,
  local_seq INTEGER NOT NULL, deleted_at INTEGER, extra BLOB
) STRICT, WITHOUT ROWID;
CREATE INDEX body_measurement_kind_day ON body_measurement (kind, day) WHERE deleted_at IS NULL;
CREATE INDEX body_measurement_seq ON body_measurement (local_seq);
