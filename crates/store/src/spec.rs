//! Which columns of each synced table travel in a sync payload, and under
//! which tag. Tags are fixed forever: new columns get new tags at the end of
//! their table's list; a dropped column keeps its tag reserved.
//!
//! Reserved payload tags: [`TAG_DELETED_AT`], [`TAG_FIELD_HLC`],
//! [`TAG_MEMBERS`]; every tag above [`TAG_MEMBERS`] is reserved for the
//! envelope. Data columns take tags from [`FIRST_COLUMN_TAG`] on, in
//! list order.

use crate::Entity;

/// `deleted_at` of the row (1) and per-field clocks (999), shared with the
/// server through the wire crate.
pub(crate) use myos_sync_wire::merge::{TAG_DELETED_AT, TAG_FIELD_CLOCKS as TAG_FIELD_HLC};
/// Aggregate members: a list record (tag `n` = member `n`).
pub(crate) const TAG_MEMBERS: u16 = 1000;
/// Tag of a table's first data column.
pub(crate) const FIRST_COLUMN_TAG: u16 = 10;

/// Rows that travel inside their parent's payload.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Members {
    /// Member table.
    pub(crate) table: &'static str,
    /// Column pointing at the parent id.
    pub(crate) parent: &'static str,
    /// Member columns, in tag order (from 1). Unknown member tags live in
    /// the member row's `extra`.
    pub(crate) columns: &'static [&'static str],
}

/// A synced table.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TableSpec {
    /// Entity code on the wire.
    pub(crate) entity: Entity,
    /// Table name.
    pub(crate) table: &'static str,
    /// Data columns in tag order (from [`FIRST_COLUMN_TAG`]).
    pub(crate) columns: &'static [&'static str],
    /// Aggregate members, if any.
    pub(crate) members: Option<Members>,
    /// Merged field by field (the table has `field_hlc`); otherwise whole rows.
    pub(crate) per_field: bool,
}

impl TableSpec {
    /// Payload tag of the column at `index` in [`TableSpec::columns`].
    pub(crate) fn tag(index: usize) -> u16 {
        FIRST_COLUMN_TAG + u16::try_from(index).expect("invariant: tables have < 1000 columns")
    }
}

/// Every synced table.
pub(crate) const TABLES: &[TableSpec] = &[
    TableSpec {
        entity: Entity::WorkoutSession,
        table: "workout_session",
        columns: &[
            "started_at",
            "ended_at",
            "day",
            "tz_offset_min",
            "origin",
            "routine_version_id",
            "routine_day_id",
            "equipment_profile_id",
            "pool_length_m",
            "feel",
            "import_batch_id",
            "note",
        ],
        members: None,
        per_field: true,
    },
    TableSpec {
        entity: Entity::SetEntry,
        table: "set_entry",
        columns: &[
            "session_id",
            "day",
            "position",
            "exercise_id",
            "routine_item_id",
            "kind",
            "side",
            "load_g",
            "assist_g",
            "bodyweight_g",
            "reps",
            "rir",
            "rpe_half",
            "duration_ms",
            "distance_m",
            "hr_avg_bpm",
            "hr_max_bpm",
            "stroke",
            "hr_zones",
            "superset",
            "rest_ms",
            "completed_at",
        ],
        members: None,
        per_field: false,
    },
    TableSpec {
        entity: Entity::Profile,
        table: "profile",
        columns: &["birth_year", "sex", "height_mm"],
        members: None,
        per_field: true,
    },
    TableSpec {
        entity: Entity::ProfileGoal,
        table: "profile_goal",
        columns: &["from_day", "goal", "level"],
        members: None,
        per_field: false,
    },
    TableSpec {
        entity: Entity::Preferences,
        table: "preferences",
        columns: &[
            "theme",
            "unit_mass",
            "unit_distance",
            "unit_energy",
            "effort_scale",
            "diet_mode",
            "locale",
        ],
        members: None,
        per_field: true,
    },
    TableSpec {
        entity: Entity::CustomExercise,
        table: "custom_exercise",
        columns: &["modality", "cardio_activity", "name", "based_on"],
        members: Some(Members {
            table: "custom_exercise_target",
            parent: "exercise_id",
            columns: &["muscle", "role", "weight_bp"],
        }),
        per_field: false,
    },
    TableSpec {
        entity: Entity::ImportBatch,
        table: "import_batch",
        columns: &["source", "file_sha256", "started_at", "rows", "status"],
        members: None,
        per_field: false,
    },
    TableSpec {
        entity: Entity::ExternalActivity,
        table: "external_activity",
        columns: &["source", "external_id", "session_id", "imported_at"],
        members: None,
        per_field: false,
    },
    TableSpec {
        entity: Entity::BodyMeasurement,
        table: "body_measurement",
        columns: &[
            "measured_at",
            "day",
            "tz_offset_min",
            "kind",
            "side",
            "method",
            "value",
            "source",
            "external_ref",
            "import_batch_id",
            "note",
        ],
        members: None,
        per_field: false,
    },
    TableSpec {
        entity: Entity::CustomFood,
        table: "custom_food",
        columns: &["name", "brand", "barcode", "nutrients"],
        members: Some(Members {
            table: "custom_food_portion",
            parent: "food_id",
            columns: &["position", "label", "grams_mg"],
        }),
        per_field: false,
    },
    TableSpec {
        entity: Entity::Recipe,
        table: "recipe",
        columns: &["name", "yield_mg"],
        members: Some(Members {
            table: "recipe_item",
            parent: "recipe_id",
            columns: &[
                "position",
                "food_src",
                "food_id",
                "barcode",
                "grams_mg",
                "name",
                "nutrients",
            ],
        }),
        per_field: false,
    },
    TableSpec {
        entity: Entity::Meal,
        table: "meal",
        columns: &[
            "eaten_at",
            "day",
            "tz_offset_min",
            "slot",
            "mode",
            "plan_meal_id",
            "quick_size",
            "quick_protein",
            "items_gen",
            "import_batch_id",
            "note",
        ],
        members: None,
        per_field: true,
    },
    TableSpec {
        entity: Entity::MealItem,
        table: "meal_item",
        columns: &[
            "meal_id",
            "generation",
            "position",
            "food_src",
            "food_id",
            "barcode",
            "grams_mg",
            "portion_label",
            "name_snapshot",
            "brand_snapshot",
            "source_version",
            "nutrients",
        ],
        members: None,
        per_field: false,
    },
    TableSpec {
        entity: Entity::Routine,
        table: "routine",
        columns: &[
            "owner_user_id",
            "name",
            "current_version_id",
            "upstream_id",
            "follow_mode",
            "followed_version_id",
        ],
        members: None,
        per_field: true,
    },
    TableSpec {
        entity: Entity::RoutineVersion,
        table: "routine_version",
        columns: &["routine_id", "n", "created_at", "content"],
        members: None,
        per_field: false,
    },
    TableSpec {
        entity: Entity::NutritionPlan,
        table: "nutrition_plan",
        columns: &[
            "owner_user_id",
            "name",
            "current_version_id",
            "starts_on",
            "ends_on",
            "default_mode",
        ],
        members: None,
        per_field: true,
    },
    TableSpec {
        entity: Entity::NutritionPlanVersion,
        table: "nutrition_plan_version",
        columns: &["plan_id", "n", "created_at", "content"],
        members: None,
        per_field: false,
    },
    TableSpec {
        entity: Entity::PlanPhase,
        table: "plan_phase",
        columns: &[
            "starts_on",
            "ends_on",
            "kind",
            "volume_mult_bp",
            "intensity_mult_bp",
            "set_by_user_id",
        ],
        members: None,
        per_field: false,
    },
    TableSpec {
        entity: Entity::PlanEvent,
        table: "plan_event",
        columns: &["on_day", "kind", "priority", "name"],
        members: None,
        per_field: false,
    },
    TableSpec {
        entity: Entity::CalibrationResult,
        table: "calibration_result",
        columns: &[
            "set_id",
            "exercise_id",
            "day",
            "protocol",
            "value",
            "unit",
            "rir_declared",
        ],
        members: None,
        per_field: false,
    },
    TableSpec {
        entity: Entity::VolumeRangeOverride,
        table: "volume_range_override",
        columns: &[
            "muscle",
            "goal",
            "level",
            "min_milli",
            "max_milli",
            "set_by_user_id",
            "set_at",
        ],
        members: None,
        per_field: false,
    },
    TableSpec {
        entity: Entity::Suggestion,
        table: "suggestion",
        columns: &[
            "created_at",
            "rule",
            "rule_version",
            "model",
            "muscle",
            "kind",
            "confidence_bp",
            "inputs",
        ],
        members: None,
        per_field: false,
    },
    TableSpec {
        entity: Entity::SuggestionResponse,
        table: "suggestion_response",
        columns: &["suggestion_id", "response", "reason", "at"],
        members: None,
        per_field: false,
    },
    TableSpec {
        entity: Entity::EquipmentProfile,
        table: "equipment_profile",
        columns: &["name", "is_default"],
        members: Some(Members {
            table: "equipment_profile_item",
            parent: "profile_id",
            columns: &["equipment", "max_load_g"],
        }),
        per_field: false,
    },
];

/// Spec of `entity`.
pub(crate) fn of(entity: Entity) -> Option<&'static TableSpec> {
    TABLES.iter().find(|spec| spec.entity == entity)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::{DatabaseKey, Error, Store};

    /// Columns every synced root carries that are not payload data.
    const SYNC_COLUMNS: &[&str] = &[
        "id",
        "hlc",
        "node",
        "server_rev",
        "local_seq",
        "deleted_at",
        "extra",
        "field_hlc",
    ];

    fn columns(store: &Store, table: &str) -> Result<BTreeSet<String>, Error> {
        let mut statement = store
            .conn
            .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))?;
        let names = statement
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        Ok(names)
    }

    #[test]
    fn every_column_of_every_synced_table_is_in_its_spec() -> Result<(), Error> {
        let store = Store::open_in_memory(&DatabaseKey::generate()?)?;
        let mut entities = BTreeSet::new();
        for spec in TABLES {
            assert!(
                entities.insert(spec.entity as u8),
                "{} listed twice",
                spec.table
            );
            let actual = columns(&store, spec.table)?;
            let mut expected: BTreeSet<String> =
                spec.columns.iter().map(|&name| name.to_owned()).collect();
            expected.extend(
                SYNC_COLUMNS
                    .iter()
                    .filter(|&&name| actual.contains(name))
                    .map(|&name| name.to_owned()),
            );
            assert_eq!(actual, expected, "{}", spec.table);
            assert_eq!(
                spec.per_field,
                actual.contains("field_hlc"),
                "{}: per_field must match the field_hlc column",
                spec.table
            );
            if let Some(members) = spec.members {
                let actual = columns(&store, members.table)?;
                let mut expected: BTreeSet<String> = members
                    .columns
                    .iter()
                    .map(|&name| name.to_owned())
                    .collect();
                expected.insert(members.parent.to_owned());
                expected.insert("extra".to_owned());
                assert_eq!(actual, expected, "{}", members.table);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod golden {
    use super::*;
    use std::fmt::Write as _;

    use myos_core::code::Code;

    /// Every `(entity, table, column, tag)` the wire knows. Tags are forever:
    /// this file only ever grows. Regenerate with `MYOS_UPDATE_GOLDEN=1` after
    /// appending columns, and review that no existing line changed.
    #[test]
    fn payload_tags_are_pinned() {
        let mut lines = String::new();
        for spec in TABLES {
            for (index, column) in spec.columns.iter().enumerate() {
                writeln!(
                    lines,
                    "{} {} {} {}",
                    spec.entity.code(),
                    spec.table,
                    column,
                    TableSpec::tag(index)
                )
                .expect("invariant: writing to a String cannot fail");
            }
            if let Some(members) = spec.members {
                for (index, column) in members.columns.iter().enumerate() {
                    writeln!(
                        lines,
                        "{} {} {} {}.{}",
                        spec.entity.code(),
                        members.table,
                        column,
                        TAG_MEMBERS,
                        index + 1
                    )
                    .expect("invariant: writing to a String cannot fail");
                }
            }
        }
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden/payload_tags.txt");
        if std::env::var_os("MYOS_UPDATE_GOLDEN").is_some() {
            std::fs::write(path, &lines).expect("invariant: golden file is writable");
        }
        let pinned = std::fs::read_to_string(path).expect("invariant: golden file exists");
        assert_eq!(lines, pinned, "payload tags changed; tags are append-only");
    }
}

#[cfg(test)]
mod limits {
    use super::*;

    #[test]
    fn column_tags_stay_below_the_reserved_range() {
        for spec in TABLES {
            let last = TableSpec::tag(spec.columns.len().saturating_sub(1));
            assert!(last < TAG_FIELD_HLC, "{} reaches reserved tags", spec.table);
        }
    }
}
