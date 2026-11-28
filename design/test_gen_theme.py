import json
import re
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import gen_theme
import palette


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

    def test_old_outline_fails_the_non_text_check(self):
        # #395: the old outline (the divider colour) was about 1.4:1 in both schemes.
        bad = json.loads(json.dumps(self.data))
        bad["light"]["outline"] = "#D5DBEA"
        bad["dark"]["outline"] = "#2A3350"
        failures = gen_theme.check_contrast(bad)
        for theme in ("light", "dark"):
            for background in ("Surface", "SurfaceDim", "SurfaceHigh"):
                self.assertTrue(
                    any(f.startswith(f"{theme}: Outline/{background}=") and "3:1" in f for f in failures),
                    f"{theme} Outline/{background} not flagged: {failures}",
                )

    def test_off_switch_thumb_is_checked_on_its_track(self):
        # #494 review: an off switch draws its thumb in outline on a surfaceContainerHighest track.
        self.assertIn("SurfaceHighest", gen_theme.NON_TEXT_BACKGROUNDS)
        bad = json.loads(json.dumps(self.data))
        bad["light"]["outline"] = "#7F89A6"  # 2.94:1 on #E8ECF5
        failures = gen_theme.check_contrast(bad)
        self.assertTrue(any(f.startswith("light: Outline/SurfaceHighest=") for f in failures), failures)

    def test_dividers_stay_subtle_and_unchecked(self):
        # outlineVariant is for dividers; it keeps the old subtle colour and isn't held to 3:1.
        mapped = gen_theme.mapped_colours(self.data)
        self.assertEqual(mapped["light"]["OutlineVariant"], "#D5DBEA")
        self.assertEqual(mapped["dark"]["OutlineVariant"], "#2A3350")
        self.assertNotEqual(mapped["light"]["Outline"], mapped["light"]["OutlineVariant"])
        self.assertNotEqual(mapped["dark"]["Outline"], mapped["dark"]["OutlineVariant"])

    def test_outline_meets_three_to_one_with_margin(self):
        mapped = gen_theme.mapped_colours(self.data)
        for theme in ("light", "dark"):
            for background in gen_theme.NON_TEXT_BACKGROUNDS:
                ratio = gen_theme.contrast_ratio(mapped[theme]["Outline"], mapped[theme][background])
                self.assertGreaterEqual(ratio, 3.0, f"{theme} Outline/{background}")

    def test_scheme_sets_outline_variant_and_highest_container(self):
        source = gen_theme.android_source(gen_theme.mapped_colours(self.data))
        self.assertIn("outlineVariant = Color(0xFFD5DBEA)", source)
        self.assertIn("surfaceContainerHighest = Color(0xFFE8ECF5)", source)
        self.assertIn("surfaceContainerHighest = Color(0xFF252D47)", source)

    def test_generated_output_is_stable(self):
        outputs = gen_theme.expected_outputs(self.data)
        self.assertEqual(gen_theme.stale_outputs(outputs), [])


class PaletteSourceTests(unittest.TestCase):
    """#495: colors.json is the only copy of the palette. build.py reads it
    through palette.py and keeps no colours of its own, so a rebuild can't
    undo an edit to colors.json."""

    BUILD = (Path(__file__).parent / "build.py").read_text(encoding="utf-8")

    def test_build_reads_the_palette_from_colors_json(self):
        self.assertIn("C = palette.load()", self.BUILD)

    def test_build_neither_writes_colors_json_nor_keeps_a_palette(self):
        self.assertNotIn('write("colors.json"', self.BUILD)
        # No colours.json token (light or dark name) mapped to a colour of its
        # own. The crane mark's facet colours ("front", "back") aren't tokens.
        data = json.loads(palette.COLORS_PATH.read_text(encoding="utf-8"))
        names = set(data["light"]) | set(data["dark"]) | set(palette.DARK_NAMES)
        entry = re.compile(r'"(\w+)"\s*:\s*"#[0-9A-Fa-f]{3,8}"')
        self.assertEqual([n for n in entry.findall(self.BUILD) if n in names], [])

    def test_every_colour_build_draws_with_comes_from_colors_json(self):
        data = json.loads(palette.COLORS_PATH.read_text(encoding="utf-8"))
        loaded = palette.load()
        used = set(re.findall(r"""C\[["'](\w+)["']\]""", self.BUILD))
        self.assertTrue(used)
        for name in used:
            with self.subTest(name=name):
                section, key = ("dark", palette.DARK_NAMES[name]) if name in palette.DARK_NAMES else ("light", name)
                self.assertEqual(loaded[name], data[section][key])

    def test_load_follows_an_edit_to_colors_json(self):
        data = json.loads(palette.COLORS_PATH.read_text(encoding="utf-8"))
        data["light"]["error"] = "#123456"
        data["dark"]["primary"] = "#ABCDEF"
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "colors.json"
            path.write_text(json.dumps(data), encoding="utf-8")
            loaded = palette.load(path)
        self.assertEqual(loaded["error"], "#123456")
        self.assertEqual(loaded["darkPrimary"], "#ABCDEF")


if __name__ == "__main__":
    unittest.main()
