//! Meals, foods and nutrients.
//!
//! Nutrient amounts are integers: energy in joules (exact for both kcal and
//! kJ sources), everything else in milligrams. Densities are per 100 g;
//! totals are always `grams × density`, so editing a portion never leaves
//! stale numbers behind.

use std::collections::BTreeMap;

use jiff::Timestamp;
use jiff::civil::Date;

use crate::id::Id;

/// A tracked nutrient.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Nutrient {
    /// Energy, joules.
    Energy,
    /// Protein, mg.
    Protein,
    /// Carbohydrates, mg.
    Carbohydrate,
    /// Fat, mg.
    Fat,
    /// Fiber, mg.
    Fiber,
    /// Sugars, mg.
    Sugar,
    /// Saturated fat, mg.
    SaturatedFat,
    /// Sodium, mg.
    Sodium,
}

crate::codes!(Nutrient {
    Energy = 1,
    Protein = 2,
    Carbohydrate = 3,
    Fat = 4,
    Fiber = 5,
    Sugar = 6,
    SaturatedFat = 7,
    Sodium = 8,
});

/// Joules in one thermochemical kilocalorie.
pub const JOULES_PER_KCAL: u64 = 4_184;

/// Nutrient amounts (per 100 g for densities, absolute for totals).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Nutrients {
    amounts: BTreeMap<Nutrient, u64>,
}

impl Nutrients {
    /// Set `nutrient` to `amount` (joules for energy, mg otherwise).
    #[must_use]
    pub fn with(mut self, nutrient: Nutrient, amount: u64) -> Self {
        self.amounts.insert(nutrient, amount);
        self
    }

    /// Amount of `nutrient`, if known.
    #[must_use]
    pub fn get(&self, nutrient: Nutrient) -> Option<u64> {
        self.amounts.get(&nutrient).copied()
    }

    /// Every known amount, in nutrient order.
    pub fn iter(&self) -> impl Iterator<Item = (Nutrient, u64)> + '_ {
        self.amounts
            .iter()
            .map(|(nutrient, amount)| (*nutrient, *amount))
    }

    /// Amounts in `grams_mg` milligrams of food with this per-100 g density,
    /// rounded to the nearest unit.
    #[must_use]
    pub fn scaled(&self, grams_mg: u32) -> Self {
        const PER_100_G_MG: u128 = 100_000;
        let amounts = self
            .amounts
            .iter()
            .map(|(nutrient, density)| {
                let total =
                    (u128::from(*density) * u128::from(grams_mg) + PER_100_G_MG / 2) / PER_100_G_MG;
                (*nutrient, u64::try_from(total).unwrap_or(u64::MAX))
            })
            .collect();
        Self { amounts }
    }

    /// Add `other` into `self`, saturating.
    pub fn add(&mut self, other: &Self) {
        for (nutrient, amount) in &other.amounts {
            let slot = self.amounts.entry(*nutrient).or_default();
            *slot = slot.saturating_add(*amount);
        }
    }
}

/// Where a food comes from.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FoodRef {
    /// Myos catalog food.
    Catalog(Id<Food>),
    /// Open Food Facts product by barcode (separate database).
    OpenFoodFacts(String),
    /// Food the user created.
    Custom(Id<Food>),
    /// Recipe the user created.
    Recipe(Id<Recipe>),
}

/// Marker for food ids.
#[derive(Debug, Clone, Copy)]
pub struct Food(std::convert::Infallible);

/// Marker for recipe ids.
#[derive(Debug, Clone, Copy)]
pub struct Recipe(std::convert::Infallible);

/// One food in a meal, with what it looked like when logged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MealItem {
    /// Food eaten.
    pub food: FoodRef,
    /// Amount eaten, milligrams.
    pub grams_mg: u32,
    /// Name at logging time (history survives catalog changes).
    pub name: String,
    /// Brand at logging time.
    pub brand: Option<String>,
    /// Per-100 g density at logging time.
    pub density: Nutrients,
}

impl MealItem {
    /// Nutrients in the eaten amount.
    #[must_use]
    pub fn totals(&self) -> Nutrients {
        self.density.scaled(self.grams_mg)
    }
}

/// Time-of-day slot of a meal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MealSlot {
    /// Breakfast.
    Breakfast,
    /// Lunch.
    Lunch,
    /// Dinner.
    Dinner,
    /// Snack.
    Snack,
    /// Before training.
    PreWorkout,
    /// After training.
    PostWorkout,
}

crate::codes!(MealSlot {
    Breakfast = 1,
    Lunch = 2,
    Dinner = 3,
    Snack = 4,
    PreWorkout = 5,
    PostWorkout = 6,
});

/// Rough size of a quickly logged meal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PortionSize {
    /// Small.
    Small,
    /// Medium.
    Medium,
    /// Large.
    Large,
}

crate::codes!(PortionSize {
    Small = 1,
    Medium = 2,
    Large = 3,
});

/// Rough protein level of a quickly logged meal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProteinLevel {
    /// Little or no protein.
    Low,
    /// Some protein.
    Medium,
    /// Protein-centred.
    High,
}

crate::codes!(ProteinLevel {
    Low = 1,
    Medium = 2,
    High = 3,
});

/// What a meal records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MealContent {
    /// Size and protein only.
    Quick {
        /// Portion size.
        size: PortionSize,
        /// Protein level.
        protein: ProteinLevel,
    },
    /// Foods and amounts.
    Detailed(Vec<MealItem>),
}

/// Identifier of a [`Meal`].
pub type MealId = Id<Meal>;

/// A meal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Meal {
    /// When it was eaten.
    pub eaten_at: Timestamp,
    /// Day on the user's clock.
    pub day: Date,
    /// Slot, if the user picked one.
    pub slot: Option<MealSlot>,
    /// What was eaten.
    pub content: MealContent,
    /// Free note.
    pub note: Option<String>,
}

impl Meal {
    /// Nutrients of a detailed meal; `None` for quick meals.
    #[must_use]
    pub fn totals(&self) -> Option<Nutrients> {
        match &self.content {
            MealContent::Quick { .. } => None,
            MealContent::Detailed(items) => {
                let mut totals = Nutrients::default();
                for item in items {
                    totals.add(&item.totals());
                }
                Some(totals)
            }
        }
    }
}

/// A household portion of a food ("1 taza", "1 unidad").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Portion {
    /// Label shown to the user.
    pub label: String,
    /// Weight of one portion, milligrams.
    pub grams_mg: u32,
}

/// A food the user created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomFood {
    /// Name.
    pub name: String,
    /// Brand.
    pub brand: Option<String>,
    /// Barcode, if the user scanned one.
    pub barcode: Option<String>,
    /// Per-100 g density.
    pub density: Nutrients,
    /// Household portions.
    pub portions: Vec<Portion>,
}
