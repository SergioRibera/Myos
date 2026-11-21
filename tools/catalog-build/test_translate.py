"""Tests of the glossary side of translate.py (no model needed):
python3 -m unittest tools/catalog-build/test_translate.py"""

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import translate  # noqa: E402

HERE = Path(__file__).parent / "translations"


class Glossary(unittest.TestCase):
    exercises = translate.load_glossary(HERE / "glossary-es.tsv")
    foods = translate.load_food_glossary(HERE / "glossary-food-es.tsv")

    def test_exercises_compose_with_agreement(self):
        cases = {
            "Incline Dumbbell Press": "Press inclinado con mancuernas",
            "Dumbbell Lunges": "Zancadas con mancuernas",
            "Seated Cable Rows": "Remo sentado en polea",
            "Smith Machine Bench Press": "Press de banca en máquina Smith",
            "Lying Triceps Extension": "Extensión de tríceps tumbada",
            "Front Squats": "Sentadillas frontales",
        }
        for english, spanish in cases.items():
            self.assertEqual(translate.compose(english, self.exercises), spanish)

    def test_uncovered_words_keep_the_english_name(self):
        self.assertIsNone(translate.compose("Zottman Frankenstein Walk", self.exercises))

    def test_food_segments_use_the_glossary_and_keep_brands(self):
        names = ["Beef, ground, raw", "Cereals, QUAKER, dry"]
        self.assertEqual(
            translate.translate_foods(names, self.foods, "es", Path("/nonexistent")),
            ["Carne de res, molido, crudo", "Cereales, QUAKER, seco"],
        )


if __name__ == "__main__":
    unittest.main()
