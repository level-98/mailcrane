import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import gen_theme


class ThemeGeneratorTests(unittest.TestCase):
    def setUp(self):
        self.data = json.loads((Path(__file__).parent / "colors.json").read_text(encoding="utf-8"))

    def test_mapping_dark_falls_back_to_light(self):
        mapped = gen_theme.mapped_colours(self.data)
        self.assertEqual(mapped["dark"]["PrimaryContainer"], self.data["light"]["primaryDark"])
        self.assertEqual(mapped["dark"]["OnPrimaryContainer"], self.data["light"]["primaryContainer"])
        # Dark mode has its own error and success colours (M6-01), with dark
        # text on the error colour.
        self.assertEqual(mapped["dark"]["Success"], self.data["dark"]["success"])
        self.assertEqual(mapped["dark"]["Error"], self.data["dark"]["error"])
        self.assertEqual(mapped["dark"]["OnError"], self.data["dark"]["surface"])

    def test_coloured_text_is_checked_on_every_background(self):
        bad = json.loads(json.dumps(self.data))
        bad["light"]["success"] = "#1F9D63"
        failures = gen_theme.check_contrast(bad)
        self.assertTrue(any("Success/Surface=" in failure for failure in failures))
        self.assertTrue(any("Success/SurfaceDim=" in failure for failure in failures))

    def test_current_palette_passes_contrast(self):
        self.assertEqual(gen_theme.check_contrast(self.data), [])

    def test_bad_palette_fails_contrast_with_ratio(self):
        bad = json.loads(json.dumps(self.data))
        bad["light"]["primary"] = "#FFFFFF"
        failures = gen_theme.check_contrast(bad)
        self.assertTrue(any("OnPrimary/Primary=" in failure for failure in failures))
        self.assertTrue(any("4.5:1" in failure for failure in failures))

    def test_generated_output_is_stable(self):
        outputs = gen_theme.expected_outputs(self.data)
        self.assertEqual(gen_theme.stale_outputs(outputs), [])


if __name__ == "__main__":
    unittest.main()
