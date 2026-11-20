//! Nutrient scaling and meal totals.

use jiff::Timestamp;
use jiff::civil::date;
use myos_core::diet::{
    FoodRef, JOULES_PER_KCAL, Meal, MealContent, MealItem, Nutrient, Nutrients, PortionSize,
    ProteinLevel,
};
use myos_core::id::Id;

fn rice() -> MealItem {
    MealItem {
        food: FoodRef::Catalog(Id::generate()),
        grams_mg: 150_000,
        name: "Arroz blanco cocido".into(),
        brand: None,
        density: Nutrients::default()
            .with(Nutrient::Energy, 130 * JOULES_PER_KCAL)
            .with(Nutrient::Protein, 2_700)
            .with(Nutrient::Carbohydrate, 28_200),
    }
}

#[test]
fn totals_scale_density_by_grams() {
    let totals = rice().totals();
    assert_eq!(totals.get(Nutrient::Energy), Some(195 * JOULES_PER_KCAL));
    assert_eq!(totals.get(Nutrient::Protein), Some(4_050));
    assert_eq!(totals.get(Nutrient::Fat), None);
}

#[test]
fn scaling_rounds_to_nearest() {
    let density = Nutrients::default().with(Nutrient::Sodium, 3);
    // 3 mg/100 g × 50 g = 1.5 mg → 2.
    assert_eq!(density.scaled(50_000).get(Nutrient::Sodium), Some(2));
    assert_eq!(density.scaled(0).get(Nutrient::Sodium), Some(0));
    let huge = Nutrients::default().with(Nutrient::Energy, u64::MAX);
    assert_eq!(huge.scaled(u32::MAX).get(Nutrient::Energy), Some(u64::MAX));
}

#[test]
fn meal_totals_sum_items_and_skip_quick_meals() {
    let mut chicken = rice();
    chicken.density = Nutrients::default().with(Nutrient::Protein, 31_000);
    chicken.grams_mg = 100_000;
    let meal = Meal {
        eaten_at: Timestamp::UNIX_EPOCH,
        day: date(2026, 11, 20),
        slot: None,
        content: MealContent::Detailed(vec![rice(), chicken]),
        note: None,
    };
    assert_eq!(
        meal.totals().unwrap_or_default().get(Nutrient::Protein),
        Some(35_050)
    );
    let quick = Meal {
        content: MealContent::Quick {
            size: PortionSize::Medium,
            protein: ProteinLevel::High,
        },
        ..meal
    };
    assert_eq!(quick.totals(), None);
}
