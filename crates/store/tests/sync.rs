//! Sync between devices through an in-memory server that follows the
//! server rules: one revision per row version, LWW on `(hlc, node)`, deletes
//! win unless restored, receipts by `op_id`.
#![expect(clippy::unwrap_used, reason = "tests: a panic is the failure report")]

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use jiff::Timestamp;
use jiff::civil::date;
use myos_core::code::Code;
use myos_core::exercise::{Exercise, Modality};
use myos_core::id::Id;
use myos_core::muscle::{Muscle, MuscleRole, MuscleTarget};
use myos_core::profile::{Goal, GoalPeriod, Level, Preferences, Profile, Theme};
use myos_core::session::{Effort, Rir, SessionOrigin, SetRecord};
use myos_core::units::Mass;
use myos_core::volume::DaySpan;
use myos_store::tlv::Tlv;
use myos_store::{
    DatabaseKey, Error, PullPage, PushBatch, PushOutcome, PushResponse, PushResult, RemoteRow,
    SessionDraft, Store, Transport,
};
use myos_sync_wire::merge::{self, Outcome, Version};
use uuid::Uuid;

#[derive(Debug, Clone)]
struct Stored {
    op: u16,
    hlc: u64,
    node: i64,
    rev: i64,
    payload_ver: u16,
    payload: Vec<u8>,
}

#[derive(Debug, Default)]
struct Server {
    rows: HashMap<(u16, Uuid), Stored>,
    rev: i64,
    receipts: HashMap<Uuid, PushResult>,
    min_proto: u32,
    fail_next_push: bool,
    /// Entity whose ops the server rejects.
    reject_entity: Option<u16>,
    /// Page size the server caps pulls at (0 = no cap).
    page_size: usize,
}

impl Server {
    fn apply(&mut self, op: &myos_store::PushOp) -> PushResult {
        if let Some(receipt) = self.receipts.get(&op.op_id) {
            return *receipt;
        }
        if self.reject_entity == Some(op.entity) {
            return PushResult::Rejected { code: 7 };
        }
        let key = (op.entity, op.row_id);
        let incoming_payload = Tlv::decode(&op.payload, 1 << 20).unwrap();
        // The same rules the server runs: `myos_sync_wire::merge`.
        let decided = self.rows.get(&key).map(|stored| {
            let stored_payload = Tlv::decode(&stored.payload, 1 << 20).unwrap();
            let outcome = merge::resolve(
                &Version {
                    op: stored.op,
                    hlc: stored.hlc,
                    node: stored.node,
                    payload: &stored_payload,
                },
                &Version {
                    op: op.op,
                    hlc: op.hlc,
                    node: op.node,
                    payload: &incoming_payload,
                },
            )
            .unwrap();
            (outcome, stored.rev)
        });
        let (op_code, hlc, node, payload) = match decided {
            Some((Outcome::KeepStored, rev)) => {
                let result = PushResult::Superseded { server_rev: rev };
                self.receipts.insert(op.op_id, result);
                return result;
            }
            None | Some((Outcome::TakeIncoming, _)) => (op.op, op.hlc, op.node, op.payload.clone()),
            Some((Outcome::Merged { payload, hlc, node }, _)) => (
                myos_sync_wire::protocol::op::UPSERT,
                hlc,
                node,
                payload.encode(),
            ),
        };
        self.rev += 1;
        self.rows.insert(
            key,
            Stored {
                op: op_code,
                hlc,
                node,
                rev: self.rev,
                payload_ver: op.payload_ver,
                payload,
            },
        );
        let result = PushResult::Applied {
            server_rev: self.rev,
        };
        self.receipts.insert(op.op_id, result);
        result
    }
}

#[derive(Debug)]
struct Offline;

impl std::fmt::Display for Offline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("offline")
    }
}

impl std::error::Error for Offline {}

#[derive(Clone)]
struct Link(Rc<RefCell<Server>>);

impl Transport for Link {
    type Error = Offline;

    fn push(&mut self, batch: &PushBatch) -> Result<PushResponse, Offline> {
        let mut server = self.0.borrow_mut();
        if server.fail_next_push {
            server.fail_next_push = false;
            // The server applied the batch but the answer was lost.
            for op in &batch.ops {
                server.apply(op);
            }
            return Err(Offline);
        }
        let outcomes = batch
            .ops
            .iter()
            .map(|op| PushOutcome {
                op_id: op.op_id,
                result: server.apply(op),
            })
            .collect();
        Ok(PushResponse {
            outcomes,
            server_time_ms: Timestamp::now().as_millisecond(),
            min_proto: server.min_proto,
        })
    }

    fn pull(&mut self, _stream: u16, after: i64, limit: u32) -> Result<PullPage, Offline> {
        let server = self.0.borrow();
        let mut rows: Vec<RemoteRow> = server
            .rows
            .iter()
            .filter(|(_, stored)| stored.rev > after)
            .map(|((entity, row_id), stored)| RemoteRow {
                entity: *entity,
                row_id: *row_id,
                op: stored.op,
                hlc: stored.hlc,
                node: stored.node,
                server_rev: stored.rev,
                payload_ver: stored.payload_ver,
                payload: stored.payload.clone(),
            })
            .collect();
        rows.sort_by_key(|row| row.server_rev);
        let limit = match server.page_size {
            0 => limit as usize,
            cap => cap.min(limit as usize),
        };
        let more = rows.len() > limit;
        rows.truncate(limit);
        Ok(PullPage {
            next: rows.last().map_or(after, |row| row.server_rev),
            rows,
            more,
            server_time_ms: Timestamp::now().as_millisecond(),
            min_proto: server.min_proto,
        })
    }
}

const ACCOUNT_KEY: [u8; 32] = [42; 32];

fn device(account: Uuid) -> Store {
    let mut store = Store::open_in_memory(&DatabaseKey::generate().unwrap()).unwrap();
    store.attach_account(account, &ACCOUNT_KEY).unwrap();
    store
}

fn pair() -> (Store, Store, Link) {
    let account = Uuid::now_v7();
    (
        device(account),
        device(account),
        Link(Rc::new(RefCell::new(Server::default()))),
    )
}

fn bench() -> Exercise {
    Exercise {
        id: Id::generate(),
        modality: Modality::Strength,
        targets: vec![MuscleTarget::new(Muscle::Chest, MuscleRole::Primary)],
    }
}

fn session(store: &mut Store) -> myos_core::session::SessionId {
    store
        .start_session(SessionDraft {
            started_at: Timestamp::from_second(1_795_000_000).unwrap(),
            day: date(2026, 11, 18),
            tz_offset_min: 0,
            origin: SessionOrigin::Manual,
        })
        .unwrap()
}

fn set(exercise: &Exercise, kg: f64) -> SetRecord {
    SetRecord::new(exercise.id)
        .with_lift(Mass::from_kg(kg).unwrap(), 8)
        .with_effort(Effort::Rir(Rir::new(2).unwrap()))
}

fn week() -> DaySpan {
    DaySpan::iso_week(date(2026, 11, 18))
}

#[test]
fn sync_requires_an_account() {
    let mut store = Store::open_in_memory(&DatabaseKey::generate().unwrap()).unwrap();
    let mut link = Link(Rc::new(RefCell::new(Server::default())));
    assert!(matches!(store.sync_once(&mut link), Err(Error::NoAccount)));
}

#[test]
fn two_devices_converge() {
    let (mut phone, mut tablet, mut link) = pair();
    let bench = bench();
    let today = date(2026, 11, 20);
    let id = session(&mut phone);
    phone.log_set(id, &set(&bench, 80.0)).unwrap();
    phone.log_set(id, &set(&bench, 85.0)).unwrap();
    phone
        .set_profile(
            &Profile {
                birth_year: Some(1990),
                ..Profile::default()
            },
            today,
        )
        .unwrap();
    phone
        .set_goal(GoalPeriod {
            from: date(2026, 1, 1),
            goal: Goal::Strength,
            level: Level::Intermediate,
        })
        .unwrap();
    let exercise = phone
        .create_custom_exercise(
            "Fondos",
            Modality::Calisthenics,
            &[MuscleTarget::new(Muscle::Triceps, MuscleRole::Primary)],
        )
        .unwrap();

    let report = phone.sync_once(&mut link).unwrap();
    assert_eq!(report.pushed, 6);
    assert!(phone.pending_outbox(10).unwrap().is_empty());
    let report = tablet.sync_once(&mut link).unwrap();
    assert_eq!(report.applied, 6);

    assert_eq!(
        tablet.sessions_in(week()).unwrap(),
        phone.sessions_in(week()).unwrap()
    );
    assert_eq!(tablet.profile().unwrap(), phone.profile().unwrap());
    assert_eq!(
        tablet.goal_on(today).unwrap(),
        phone.goal_on(today).unwrap()
    );
    let shared = tablet.custom_exercises(false).unwrap();
    assert_eq!(shared[0].exercise.id, exercise);
    assert_eq!(shared[0].exercise.targets[0].muscle, Muscle::Triceps);
    // The phone's own rows coming back change nothing.
    assert_eq!(phone.sync_once(&mut link).unwrap().applied, 0);
}

#[test]
fn newer_edit_wins_on_both_devices() {
    let (mut phone, mut tablet, mut link) = pair();
    let bench = bench();
    let id = session(&mut phone);
    let set_id = phone.log_set(id, &set(&bench, 80.0)).unwrap();
    phone.sync_once(&mut link).unwrap();
    tablet.sync_once(&mut link).unwrap();

    phone.update_set(set_id, &set(&bench, 82.5)).unwrap();
    // Distinct milliseconds: otherwise the node tiebreak picks the winner.
    std::thread::sleep(std::time::Duration::from_millis(5));
    tablet.update_set(set_id, &set(&bench, 90.0)).unwrap();
    // The tablet wrote last (higher hlc) and syncs second.
    phone.sync_once(&mut link).unwrap();
    tablet.sync_once(&mut link).unwrap();
    phone.sync_once(&mut link).unwrap();

    let on_phone = phone.sets_of(id).unwrap().sets;
    let on_tablet = tablet.sets_of(id).unwrap().sets;
    assert_eq!(on_phone, on_tablet);
    assert_eq!(on_phone[0].record.load, Mass::from_kg(90.0));
}

#[test]
fn delete_beats_a_concurrent_edit() {
    let (mut phone, mut tablet, mut link) = pair();
    let bench = bench();
    let id = session(&mut phone);
    let set_id = phone.log_set(id, &set(&bench, 80.0)).unwrap();
    phone.sync_once(&mut link).unwrap();
    tablet.sync_once(&mut link).unwrap();

    phone.delete_set(set_id).unwrap();
    tablet.update_set(set_id, &set(&bench, 100.0)).unwrap();
    tablet.sync_once(&mut link).unwrap();
    phone.sync_once(&mut link).unwrap();
    tablet.sync_once(&mut link).unwrap();

    assert!(phone.sets_of(id).unwrap().sets.is_empty());
    assert!(tablet.sets_of(id).unwrap().sets.is_empty());
}

#[test]
fn a_lost_answer_is_retried_without_duplicates() {
    let (mut phone, mut tablet, mut link) = pair();
    let id = session(&mut phone);
    phone.log_set(id, &set(&bench(), 80.0)).unwrap();
    link.0.borrow_mut().fail_next_push = true;
    assert!(matches!(
        phone.sync_once(&mut link),
        Err(Error::Transport(_))
    ));
    assert_eq!(phone.pending_outbox(10).unwrap().len(), 2);
    // Retried after the backoff: the pending entries are not ready yet.
    assert_eq!(phone.sync_once(&mut link).unwrap().pushed, 0);
    tablet.sync_once(&mut link).unwrap();
    assert_eq!(
        tablet.sessions_in(week()).unwrap().sessions[0].sets.len(),
        1
    );
}

#[test]
fn singletons_and_natural_keys_merge_across_devices() {
    let (mut phone, mut tablet, mut link) = pair();
    let goal = |level| GoalPeriod {
        from: date(2026, 6, 1),
        goal: Goal::Hypertrophy,
        level,
    };
    phone.set_goal(goal(Level::Beginner)).unwrap();
    tablet.set_goal(goal(Level::Advanced)).unwrap();
    phone
        .set_preferences(&Preferences {
            theme: Theme::Dark,
            ..Preferences::default()
        })
        .unwrap();
    tablet
        .set_preferences(&Preferences {
            theme: Theme::Light,
            ..Preferences::default()
        })
        .unwrap();
    for _ in 0..2 {
        phone.sync_once(&mut link).unwrap();
        tablet.sync_once(&mut link).unwrap();
    }
    let day = date(2026, 11, 20);
    // Both devices minted the same id for the same start day: one row, one
    // winner by (hlc, node), the same on both.
    assert_eq!(phone.goal_on(day).unwrap(), tablet.goal_on(day).unwrap());
    assert!(phone.goal_on(day).unwrap().is_some());
    assert_eq!(phone.preferences().unwrap(), tablet.preferences().unwrap());
}

#[test]
fn unknown_entities_and_tags_from_a_newer_build() {
    let (mut phone, mut tablet, mut link) = pair();
    let id = session(&mut phone);
    phone.log_set(id, &set(&bench(), 80.0)).unwrap();
    phone.sync_once(&mut link).unwrap();
    {
        let mut server = link.0.borrow_mut();
        // A table this build does not know.
        server.rev += 1;
        let rev = server.rev;
        server.rows.insert(
            (999, Uuid::now_v7()),
            Stored {
                op: 1,
                hlc: 1,
                node: 1,
                rev,
                payload_ver: 2,
                payload: vec![],
            },
        );
        // A field this build does not know on a known row (tag 98).
        server.rev += 1;
        let rev = server.rev;
        let session_row = server
            .rows
            .values_mut()
            .find(|stored| stored.payload.len() > 30)
            .unwrap();
        let mut record = Tlv::decode(&session_row.payload, 1 << 20).unwrap();
        record.put_uint(98, 7);
        session_row.payload = record.encode();
        session_row.rev = rev;
    }
    let report = tablet.sync_once(&mut link).unwrap();
    assert_eq!(report.quarantined, 1);
    assert!(report.applied >= 2);
    assert_eq!(tablet.sessions_in(week()).unwrap().sessions.len(), 1);
}

#[test]
fn a_newer_protocol_makes_old_builds_read_only() {
    let (mut phone, _tablet, mut link) = pair();
    link.0.borrow_mut().min_proto = myos_store::PROTO + 1;
    let id = session(&mut phone);
    let report = phone.sync_once(&mut link).unwrap();
    assert!(report.read_only);
    assert!(matches!(
        phone.log_set(id, &set(&bench(), 80.0)),
        Err(Error::ReadOnly)
    ));
    link.0.borrow_mut().min_proto = myos_store::PROTO;
    assert!(!phone.sync_once(&mut link).unwrap().read_only);
    phone.log_set(id, &set(&bench(), 80.0)).unwrap();
}

#[test]
fn a_rejected_local_write_does_not_block_remote_versions() {
    let (mut phone, mut tablet, mut link) = pair();
    let bench = bench();
    let id = session(&mut phone);
    let set_id = phone.log_set(id, &set(&bench, 80.0)).unwrap();
    phone.sync_once(&mut link).unwrap();
    tablet.sync_once(&mut link).unwrap();

    // The server refuses the phone's edit of the set …
    link.0.borrow_mut().reject_entity = Some(myos_store::Entity::SetEntry.code());
    phone.update_set(set_id, &set(&bench, 999.0)).unwrap();
    assert_eq!(phone.sync_once(&mut link).unwrap().rejected, 1);
    link.0.borrow_mut().reject_entity = None;
    // … so the tablet's later edit must still reach it.
    tablet.update_set(set_id, &set(&bench, 85.0)).unwrap();
    tablet.sync_once(&mut link).unwrap();
    phone.sync_once(&mut link).unwrap();
    assert_eq!(
        phone.sets_of(id).unwrap().sets[0].record.load,
        Mass::from_kg(85.0)
    );
}

#[test]
fn pulls_page_through_the_feed() {
    let (mut phone, mut tablet, mut link) = pair();
    let bench = bench();
    let id = session(&mut phone);
    for kg in 1..=7 {
        phone
            .log_set(id, &set(&bench, f64::from(kg) * 10.0))
            .unwrap();
    }
    phone.sync_once(&mut link).unwrap();
    link.0.borrow_mut().page_size = 3;
    let report = tablet.sync_once(&mut link).unwrap();
    assert_eq!(report.applied, 8);
    assert_eq!(tablet.sets_of(id).unwrap().sets.len(), 7);
}

#[test]
fn attaching_an_account_rewrites_natural_keys_once() {
    let mut store = Store::open_in_memory(&DatabaseKey::generate().unwrap()).unwrap();
    let goal = GoalPeriod {
        from: date(2026, 1, 1),
        goal: Goal::Health,
        level: Level::Beginner,
    };
    store.set_goal(goal).unwrap();
    let before = store.pending_outbox(10).unwrap()[0].row_id;
    let account = Uuid::now_v7();
    store.attach_account(account, &ACCOUNT_KEY).unwrap();
    let after = store.pending_outbox(10).unwrap()[0].row_id;
    assert_ne!(
        before, after,
        "device-key id replaced by the account-key id"
    );
    assert_eq!(store.goal_on(date(2026, 2, 1)).unwrap(), Some(goal));
    // Same account again: nothing changes. Another account: refused.
    store.attach_account(account, &ACCOUNT_KEY).unwrap();
    assert_eq!(store.pending_outbox(10).unwrap()[0].row_id, after);
    assert!(matches!(
        store.attach_account(Uuid::now_v7(), &[1; 32]),
        Err(Error::AccountMismatch)
    ));
    // A second device of the same account mints the same id.
    let mut other = device(account);
    other.set_goal(goal).unwrap();
    assert_eq!(other.pending_outbox(10).unwrap()[0].row_id, after);
}

#[test]
fn concurrent_edits_of_different_fields_both_survive() {
    use myos_core::profile::{MassUnit, Sex};
    use myos_core::units::Length;

    let (mut phone, mut tablet, mut link) = pair();
    let today = date(2026, 11, 20);
    let base = Profile {
        birth_year: Some(1990),
        ..Profile::default()
    };
    phone.set_profile(&base, today).unwrap();
    phone.set_preferences(&Preferences::default()).unwrap();
    phone.sync_once(&mut link).unwrap();
    tablet.sync_once(&mut link).unwrap();

    // Offline, each device edits a different field of the same rows.
    phone
        .set_profile(
            &Profile {
                height: Some(Length::from_millimeters(1_750)),
                ..base
            },
            today,
        )
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    tablet
        .set_profile(
            &Profile {
                sex: Some(Sex::Female),
                ..base
            },
            today,
        )
        .unwrap();
    phone
        .set_preferences(&Preferences {
            theme: Theme::Dark,
            ..Preferences::default()
        })
        .unwrap();
    tablet
        .set_preferences(&Preferences {
            mass_unit: MassUnit::Pound,
            ..Preferences::default()
        })
        .unwrap();

    for _ in 0..2 {
        phone.sync_once(&mut link).unwrap();
        tablet.sync_once(&mut link).unwrap();
    }
    let expected = Profile {
        birth_year: Some(1990),
        sex: Some(Sex::Female),
        height: Some(Length::from_millimeters(1_750)),
    };
    assert_eq!(phone.profile().unwrap(), expected);
    assert_eq!(tablet.profile().unwrap(), expected);
    let preferences = phone.preferences().unwrap();
    assert_eq!(tablet.preferences().unwrap(), preferences);
    assert_eq!(
        (preferences.theme, preferences.mass_unit),
        (Theme::Dark, MassUnit::Pound)
    );
    assert!(phone.pending_outbox(10).unwrap().is_empty());
    assert!(tablet.pending_outbox(10).unwrap().is_empty());
}

#[test]
fn a_note_edit_does_not_undo_a_concurrent_item_replacement() {
    use myos_core::diet::{FoodRef, Meal, MealContent, MealItem, Nutrients};

    let (mut phone, mut tablet, mut link) = pair();
    let item = |name: &str| MealItem {
        food: FoodRef::Catalog(Id::generate()),
        grams_mg: 100_000,
        name: name.into(),
        brand: None,
        density: Nutrients::default(),
    };
    let meal = Meal {
        eaten_at: Timestamp::from_second(1_795_000_000).unwrap(),
        day: date(2026, 11, 18),
        slot: None,
        content: MealContent::Detailed(vec![item("Arroz")]),
        note: None,
    };
    let id = phone.log_meal(&meal, 0).unwrap();
    phone.sync_once(&mut link).unwrap();
    tablet.sync_once(&mut link).unwrap();

    let replaced = Meal {
        content: MealContent::Detailed(vec![item("Pollo")]),
        ..meal.clone()
    };
    phone.replace_meal(id, &replaced, 0).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    let noted = Meal {
        note: Some("sin sal".into()),
        ..meal
    };
    tablet.replace_meal(id, &noted, 0).unwrap();

    for _ in 0..2 {
        phone.sync_once(&mut link).unwrap();
        tablet.sync_once(&mut link).unwrap();
    }
    let on_phone = phone.meals_in(week()).unwrap();
    let on_tablet = tablet.meals_in(week()).unwrap();
    assert_eq!(on_phone, on_tablet);
    assert_eq!(on_phone.meals.len(), 1);
    let MealContent::Detailed(items) = &on_phone.meals[0].1.content else {
        panic!("detailed meal expected");
    };
    // One list (no doubled items); both devices agree on it.
    assert_eq!(items.len(), 1);
}
