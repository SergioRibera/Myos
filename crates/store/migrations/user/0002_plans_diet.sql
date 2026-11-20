-- Diet, routines, plan, calibration, suggestions, equipment and local
-- attachments / reminders. Same sync-column contract as 0001.

CREATE TABLE custom_food (
  id         BLOB PRIMARY KEY CHECK (length(id) = 16),
  name       TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
  brand      TEXT CHECK (length(brand) <= 120),
  barcode    TEXT CHECK (length(barcode) <= 32),
  nutrients  BLOB NOT NULL,             -- TLV nutrient code -> amount per 100 g
  hlc INTEGER NOT NULL, node INTEGER NOT NULL, server_rev INTEGER,
  local_seq INTEGER NOT NULL, deleted_at INTEGER, extra BLOB
) STRICT;

CREATE TABLE custom_food_portion (      -- aggregate member of custom_food
  food_id   BLOB NOT NULL
    REFERENCES custom_food (id) ON DELETE CASCADE DEFERRABLE INITIALLY DEFERRED,
  position  INTEGER NOT NULL CHECK (position >= 0),
  label     TEXT NOT NULL CHECK (length(label) BETWEEN 1 AND 80),
  grams_mg  INTEGER NOT NULL CHECK (grams_mg BETWEEN 1 AND 100000000),
  PRIMARY KEY (food_id, position)
) STRICT, WITHOUT ROWID;

CREATE TABLE recipe (
  id        BLOB PRIMARY KEY CHECK (length(id) = 16),
  name      TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
  yield_mg  INTEGER NOT NULL CHECK (yield_mg BETWEEN 1 AND 100000000),
  hlc INTEGER NOT NULL, node INTEGER NOT NULL, server_rev INTEGER,
  local_seq INTEGER NOT NULL, deleted_at INTEGER, extra BLOB
) STRICT;

CREATE TABLE recipe_item (              -- aggregate member of recipe
  recipe_id BLOB NOT NULL
    REFERENCES recipe (id) ON DELETE CASCADE DEFERRABLE INITIALLY DEFERRED,
  position  INTEGER NOT NULL CHECK (position >= 0),
  food_src  INTEGER NOT NULL CHECK (food_src BETWEEN 1 AND 4),
  food_id   BLOB CHECK (food_id IS NULL OR length(food_id) = 16),
  barcode   TEXT CHECK (length(barcode) <= 32),
  grams_mg  INTEGER NOT NULL CHECK (grams_mg BETWEEN 1 AND 100000000),
  name      TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
  nutrients BLOB NOT NULL,
  PRIMARY KEY (recipe_id, position),
  CHECK ((food_src = 2) = (barcode IS NOT NULL)),
  CHECK ((food_src = 2) = (food_id IS NULL))
) STRICT, WITHOUT ROWID;

CREATE TABLE meal (
  id              BLOB PRIMARY KEY CHECK (length(id) = 16),
  eaten_at        INTEGER NOT NULL CHECK (eaten_at BETWEEN 0 AND 253402300799999),
  day             INTEGER NOT NULL CHECK (day BETWEEN 0 AND 73000),
  tz_offset_min   INTEGER NOT NULL CHECK (tz_offset_min BETWEEN -840 AND 840),
  slot            INTEGER CHECK (slot BETWEEN 0 AND 255),
  mode            INTEGER NOT NULL CHECK (mode BETWEEN 0 AND 255),  -- 1 quick, 2 detailed
  plan_meal_id    BLOB CHECK (plan_meal_id IS NULL OR length(plan_meal_id) = 16),
  quick_size      INTEGER CHECK (quick_size BETWEEN 0 AND 255),
  quick_protein   INTEGER CHECK (quick_protein BETWEEN 0 AND 255),
  import_batch_id BLOB CHECK (import_batch_id IS NULL OR length(import_batch_id) = 16),
  note            TEXT CHECK (length(note) <= 500),
  field_hlc       BLOB,
  hlc INTEGER NOT NULL, node INTEGER NOT NULL, server_rev INTEGER,
  local_seq INTEGER NOT NULL, deleted_at INTEGER, extra BLOB,
  CHECK ((quick_size IS NULL) = (quick_protein IS NULL))
) STRICT;
CREATE INDEX meal_day ON meal (day) WHERE deleted_at IS NULL;
CREATE INDEX meal_seq ON meal (local_seq);

-- No foreign key to meal: both are sync roots. An item is live only while its
-- meal is.
CREATE TABLE meal_item (
  id                BLOB PRIMARY KEY CHECK (length(id) = 16),
  meal_id           BLOB NOT NULL CHECK (length(meal_id) = 16),
  position          INTEGER NOT NULL CHECK (position >= 0),
  food_src          INTEGER NOT NULL CHECK (food_src BETWEEN 1 AND 4),  -- catalog, off, custom, recipe
  food_id           BLOB CHECK (food_id IS NULL OR length(food_id) = 16),
  barcode           TEXT CHECK (length(barcode) <= 32),
  grams_mg          INTEGER NOT NULL CHECK (grams_mg BETWEEN 1 AND 100000000),
  portion_label     TEXT CHECK (length(portion_label) <= 80),
  name_snapshot     TEXT NOT NULL CHECK (length(name_snapshot) BETWEEN 1 AND 200),
  brand_snapshot    TEXT CHECK (length(brand_snapshot) <= 120),
  source_version    INTEGER,
  nutrients         BLOB NOT NULL,     -- TLV nutrient code -> amount per 100 g
  hlc INTEGER NOT NULL, node INTEGER NOT NULL, server_rev INTEGER,
  local_seq INTEGER NOT NULL, deleted_at INTEGER, extra BLOB,
  CHECK ((food_src = 2) = (barcode IS NOT NULL)),
  CHECK ((food_src = 2) = (food_id IS NULL))
) STRICT, WITHOUT ROWID;
CREATE INDEX meal_item_meal ON meal_item (meal_id, position) WHERE deleted_at IS NULL;
CREATE INDEX meal_item_seq ON meal_item (local_seq);

-- Routines and nutrition plans: a head row (per-field LWW; one writer, the
-- owner) and immutable versions holding the content as TLV.
CREATE TABLE routine (
  id                  BLOB PRIMARY KEY CHECK (length(id) = 16),
  owner_user_id       BLOB CHECK (owner_user_id IS NULL OR length(owner_user_id) = 16),
  name                TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 120),
  current_version_id  BLOB NOT NULL CHECK (length(current_version_id) = 16),
  upstream_id         BLOB CHECK (upstream_id IS NULL OR length(upstream_id) = 16),
  follow_mode         INTEGER NOT NULL CHECK (follow_mode BETWEEN 0 AND 255),
  followed_version_id BLOB CHECK (followed_version_id IS NULL OR length(followed_version_id) = 16),
  field_hlc           BLOB,
  hlc INTEGER NOT NULL, node INTEGER NOT NULL, server_rev INTEGER,
  local_seq INTEGER NOT NULL, deleted_at INTEGER, extra BLOB
) STRICT;

CREATE TABLE routine_version (
  id         BLOB PRIMARY KEY CHECK (length(id) = 16),
  routine_id BLOB NOT NULL CHECK (length(routine_id) = 16),
  n          INTEGER NOT NULL CHECK (n >= 1),
  created_at INTEGER NOT NULL,
  content    BLOB NOT NULL,
  hlc INTEGER NOT NULL, node INTEGER NOT NULL, server_rev INTEGER,
  local_seq INTEGER NOT NULL, deleted_at INTEGER, extra BLOB
) STRICT;
CREATE INDEX routine_version_routine ON routine_version (routine_id, n);

CREATE TABLE nutrition_plan (
  id                 BLOB PRIMARY KEY CHECK (length(id) = 16),
  owner_user_id      BLOB CHECK (owner_user_id IS NULL OR length(owner_user_id) = 16),
  name               TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 120),
  current_version_id BLOB NOT NULL CHECK (length(current_version_id) = 16),
  starts_on          INTEGER CHECK (starts_on BETWEEN 0 AND 73000),
  ends_on            INTEGER CHECK (ends_on BETWEEN 0 AND 73000),
  default_mode       INTEGER CHECK (default_mode BETWEEN 0 AND 255),
  field_hlc          BLOB,
  hlc INTEGER NOT NULL, node INTEGER NOT NULL, server_rev INTEGER,
  local_seq INTEGER NOT NULL, deleted_at INTEGER, extra BLOB,
  CHECK (ends_on IS NULL OR starts_on IS NULL OR ends_on >= starts_on)
) STRICT;

CREATE TABLE nutrition_plan_version (
  id         BLOB PRIMARY KEY CHECK (length(id) = 16),
  plan_id    BLOB NOT NULL CHECK (length(plan_id) = 16),
  n          INTEGER NOT NULL CHECK (n >= 1),
  created_at INTEGER NOT NULL,
  content    BLOB NOT NULL,              -- TLV: day types, targets, meals, swaps
  hlc INTEGER NOT NULL, node INTEGER NOT NULL, server_rev INTEGER,
  local_seq INTEGER NOT NULL, deleted_at INTEGER, extra BLOB
) STRICT;
CREATE INDEX nutrition_plan_version_plan ON nutrition_plan_version (plan_id, n);

CREATE TABLE plan_phase (
  id                BLOB PRIMARY KEY CHECK (length(id) = 16),
  starts_on         INTEGER NOT NULL CHECK (starts_on BETWEEN 0 AND 73000),
  ends_on           INTEGER NOT NULL CHECK (ends_on BETWEEN 0 AND 73000),
  kind              INTEGER NOT NULL CHECK (kind BETWEEN 0 AND 255),
  volume_mult_bp    INTEGER NOT NULL DEFAULT 10000 CHECK (volume_mult_bp BETWEEN 0 AND 30000),
  intensity_mult_bp INTEGER NOT NULL DEFAULT 10000 CHECK (intensity_mult_bp BETWEEN 0 AND 30000),
  set_by_user_id    BLOB CHECK (set_by_user_id IS NULL OR length(set_by_user_id) = 16),
  hlc INTEGER NOT NULL, node INTEGER NOT NULL, server_rev INTEGER,
  local_seq INTEGER NOT NULL, deleted_at INTEGER, extra BLOB,
  CHECK (ends_on >= starts_on)
) STRICT;
CREATE INDEX plan_phase_days ON plan_phase (starts_on, ends_on) WHERE deleted_at IS NULL;

CREATE TABLE plan_event (
  id       BLOB PRIMARY KEY CHECK (length(id) = 16),
  on_day   INTEGER NOT NULL CHECK (on_day BETWEEN 0 AND 73000),
  kind     INTEGER NOT NULL CHECK (kind BETWEEN 0 AND 255),
  priority INTEGER NOT NULL CHECK (priority BETWEEN 0 AND 255),
  name     TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 120),
  hlc INTEGER NOT NULL, node INTEGER NOT NULL, server_rev INTEGER,
  local_seq INTEGER NOT NULL, deleted_at INTEGER, extra BLOB
) STRICT;
CREATE INDEX plan_event_day ON plan_event (on_day) WHERE deleted_at IS NULL;

-- What was declared or measured during calibration, never a computed value.
CREATE TABLE calibration_result (
  id           BLOB PRIMARY KEY CHECK (length(id) = 16),
  set_id       BLOB CHECK (set_id IS NULL OR length(set_id) = 16),
  exercise_id  BLOB NOT NULL CHECK (length(exercise_id) = 16),
  day          INTEGER NOT NULL CHECK (day BETWEEN 0 AND 73000),
  protocol     INTEGER NOT NULL CHECK (protocol BETWEEN 0 AND 255),
  kind         INTEGER NOT NULL CHECK (kind BETWEEN 0 AND 255),
  value        INTEGER NOT NULL,
  unit         INTEGER NOT NULL CHECK (unit BETWEEN 0 AND 255),
  rir_declared INTEGER CHECK (rir_declared BETWEEN 0 AND 10),
  hlc INTEGER NOT NULL, node INTEGER NOT NULL, server_rev INTEGER,
  local_seq INTEGER NOT NULL, deleted_at INTEGER, extra BLOB
) STRICT;
CREATE INDEX calibration_result_exercise ON calibration_result (exercise_id, day)
  WHERE deleted_at IS NULL;

-- id = HMAC(user_key, muscle, goal, level). No unique index: rows minted
-- under different pre-login keys coexist until the login rewrite; reads take
-- the highest hlc.
CREATE TABLE volume_range_override (
  id             BLOB PRIMARY KEY CHECK (length(id) = 16),
  muscle         INTEGER NOT NULL CHECK (muscle BETWEEN 0 AND 65535),
  goal           INTEGER NOT NULL CHECK (goal BETWEEN 0 AND 255),
  level          INTEGER NOT NULL CHECK (level BETWEEN 0 AND 255),
  min_milli      INTEGER NOT NULL CHECK (min_milli >= 0),
  max_milli      INTEGER NOT NULL CHECK (max_milli >= 0),
  set_by_user_id BLOB CHECK (set_by_user_id IS NULL OR length(set_by_user_id) = 16),
  set_at         INTEGER NOT NULL,
  hlc INTEGER NOT NULL, node INTEGER NOT NULL, server_rev INTEGER,
  local_seq INTEGER NOT NULL, deleted_at INTEGER, extra BLOB,
  CHECK (max_milli >= min_milli)
) STRICT;
CREATE INDEX volume_range_override_key ON volume_range_override (muscle, goal, level)
  WHERE deleted_at IS NULL;

CREATE TABLE suggestion (               -- immutable after creation
  id            BLOB PRIMARY KEY CHECK (length(id) = 16),
  created_at    INTEGER NOT NULL,
  rule          INTEGER NOT NULL CHECK (rule BETWEEN 0 AND 65535),
  rule_version  INTEGER NOT NULL CHECK (rule_version >= 0),
  model         INTEGER NOT NULL CHECK (model BETWEEN 0 AND 65535),
  muscle        INTEGER CHECK (muscle BETWEEN 0 AND 65535),
  kind          INTEGER NOT NULL CHECK (kind BETWEEN 0 AND 255),
  confidence_bp INTEGER NOT NULL CHECK (confidence_bp BETWEEN 0 AND 10000),
  inputs        BLOB NOT NULL,          -- TLV of the numbers used
  hlc INTEGER NOT NULL, node INTEGER NOT NULL, server_rev INTEGER,
  local_seq INTEGER NOT NULL, deleted_at INTEGER, extra BLOB
) STRICT;
CREATE INDEX suggestion_created ON suggestion (created_at) WHERE deleted_at IS NULL;

CREATE TABLE suggestion_response (      -- append-only; latest wins for display
  id            BLOB PRIMARY KEY CHECK (length(id) = 16),
  suggestion_id BLOB NOT NULL CHECK (length(suggestion_id) = 16),
  response      INTEGER NOT NULL CHECK (response BETWEEN 0 AND 255),
  reason        INTEGER CHECK (reason BETWEEN 0 AND 255),
  at            INTEGER NOT NULL,
  hlc INTEGER NOT NULL, node INTEGER NOT NULL, server_rev INTEGER,
  local_seq INTEGER NOT NULL, deleted_at INTEGER, extra BLOB
) STRICT;
CREATE INDEX suggestion_response_suggestion ON suggestion_response (suggestion_id, at);

CREATE TABLE equipment_profile (
  id         BLOB PRIMARY KEY CHECK (length(id) = 16),
  name       TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 80),
  is_default INTEGER NOT NULL DEFAULT 0 CHECK (is_default IN (0, 1)),
  hlc INTEGER NOT NULL, node INTEGER NOT NULL, server_rev INTEGER,
  local_seq INTEGER NOT NULL, deleted_at INTEGER, extra BLOB
) STRICT;

CREATE TABLE equipment_profile_item (   -- aggregate member of equipment_profile
  profile_id BLOB NOT NULL
    REFERENCES equipment_profile (id) ON DELETE CASCADE DEFERRABLE INITIALLY DEFERRED,
  equipment  INTEGER NOT NULL CHECK (equipment BETWEEN 0 AND 65535),
  max_load_g INTEGER CHECK (max_load_g BETWEEN 0 AND 4000000),
  PRIMARY KEY (profile_id, equipment)
) STRICT, WITHOUT ROWID;

-- Local only: files live outside the database, encrypted with a key derived
-- from the database key, excluded from OS backups.
CREATE TABLE attachment (
  id           BLOB PRIMARY KEY CHECK (length(id) = 16),
  owner_entity INTEGER NOT NULL CHECK (owner_entity BETWEEN 1 AND 65535),
  owner_id     BLOB NOT NULL CHECK (length(owner_id) = 16),
  kind         INTEGER NOT NULL CHECK (kind BETWEEN 0 AND 255),
  mime         TEXT NOT NULL CHECK (length(mime) BETWEEN 1 AND 100),
  sha256       BLOB NOT NULL CHECK (length(sha256) = 32),
  size         INTEGER NOT NULL CHECK (size >= 0),
  state        INTEGER NOT NULL CHECK (state BETWEEN 0 AND 255),
  created_at   INTEGER NOT NULL
) STRICT;
CREATE INDEX attachment_owner ON attachment (owner_entity, owner_id);

CREATE TABLE reminder (                 -- local only
  id      BLOB PRIMARY KEY CHECK (length(id) = 16),
  kind    INTEGER NOT NULL CHECK (kind BETWEEN 0 AND 255),
  rrule   TEXT NOT NULL CHECK (length(rrule) BETWEEN 1 AND 500),
  enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1))
) STRICT;
