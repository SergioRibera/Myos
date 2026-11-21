-- Sync store v1: one opaque row version per (user, entity, row). Typed
-- projections for the professional panel and analytics read from here later.

CREATE TABLE user_rev (
  user_id uuid PRIMARY KEY,
  rev     bigint NOT NULL DEFAULT 0
);

-- Current version of every synced row. `rev` comes from the user's own
-- counter, taken inside the push transaction (row lock on user_rev): revisions
-- are gap-free and commit-ordered per user, so pulling `rev > cursor` never
-- skips a row that commits late.
CREATE TABLE sync_row (
  user_id     uuid     NOT NULL,
  entity      smallint NOT NULL,
  row_id      uuid     NOT NULL,
  op          smallint NOT NULL CHECK (op BETWEEN 1 AND 3),
  hlc         bigint   NOT NULL,
  node        bigint   NOT NULL,
  rev         bigint   NOT NULL,
  payload_ver smallint NOT NULL,
  payload     bytea    NOT NULL,
  received_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (user_id, entity, row_id)
);
CREATE INDEX sync_row_feed ON sync_row (user_id, rev);

-- Answers to pushed ops by op id: a retried push gets the same answer.
CREATE TABLE push_receipt (
  user_id uuid     NOT NULL,
  op_id   uuid     NOT NULL,
  outcome smallint NOT NULL,
  rev     bigint,
  code    integer,
  at      timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (user_id, op_id)
);
CREATE INDEX push_receipt_at ON push_receipt (at);

-- Every query runs with `app.user_id` set for its transaction; a missing
-- setting matches no rows. FORCE applies the policies to the table owner too.
ALTER TABLE user_rev ENABLE ROW LEVEL SECURITY;
ALTER TABLE user_rev FORCE ROW LEVEL SECURITY;
CREATE POLICY own ON user_rev
  USING (user_id = nullif(current_setting('app.user_id', true), '')::uuid)
  WITH CHECK (user_id = nullif(current_setting('app.user_id', true), '')::uuid);

ALTER TABLE sync_row ENABLE ROW LEVEL SECURITY;
ALTER TABLE sync_row FORCE ROW LEVEL SECURITY;
CREATE POLICY own ON sync_row
  USING (user_id = nullif(current_setting('app.user_id', true), '')::uuid)
  WITH CHECK (user_id = nullif(current_setting('app.user_id', true), '')::uuid);

ALTER TABLE push_receipt ENABLE ROW LEVEL SECURITY;
ALTER TABLE push_receipt FORCE ROW LEVEL SECURITY;
CREATE POLICY own ON push_receipt
  USING (user_id = nullif(current_setting('app.user_id', true), '')::uuid)
  WITH CHECK (user_id = nullif(current_setting('app.user_id', true), '')::uuid);

-- Receipt purge (all users) runs with `app.maintenance = on` for its
-- transaction; it can only delete receipts.
CREATE POLICY maintenance ON push_receipt FOR DELETE
  USING (current_setting('app.maintenance', true) = 'on');
