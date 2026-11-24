import contextlib
import datetime
import io
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import release_notes  # noqa: E402


class ReleaseNotesTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name)
        self.dir = self.root / "docs" / "changelog.d"
        self.dir.mkdir(parents=True)
        (self.dir / "README.md").write_text("readme\n")

    def add(self, name, text):
        (self.dir / name).write_text(text)

    def run_main(self, *argv):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = release_notes.main(list(argv), root=self.root)
        return code, out.getvalue(), err.getvalue()

    def files(self):
        return sorted(p.name for p in self.dir.glob("*.md"))

    def sample(self):
        self.add("3.fix.md", "Fixed a thing.\n")
        self.add("1.security.md", "Closed\na hole.\n\n")
        self.add("m0-01-licence.feature.md", "Slug entry.")
        self.add("2.change.md", "Changed it.")
        self.add("4.internal.md", "Refactor.")

    def test_dry_run_groups_and_changes_nothing(self):
        self.sample()
        before = self.files()
        code, out, _ = self.run_main("0.1.0", "--date", "2026-01-02")
        self.assertEqual(code, 0)
        self.assertEqual(
            out,
            "## 0.1.0 (2026-01-02)\n\n### Security\n\n- Closed a hole. (#1)\n\n"
            "### New\n\n- Slug entry.\n\n### Changed\n\n- Changed it. (#2)\n\n"
            "### Fixed\n\n- Fixed a thing. (#3)\n",
        )
        self.assertEqual(self.files(), before)
        self.assertFalse((self.root / "CHANGELOG.md").exists())

    def test_include_internal(self):
        self.sample()
        _, out, _ = self.run_main("0.1.0", "--date", "2026-01-02", "--include-internal")
        self.assertTrue(out.endswith("### Internal\n\n- Refactor. (#4)\n"))

    def test_default_date_is_today(self):
        self.sample()
        _, out, _ = self.run_main("0.1.0")
        self.assertTrue(out.startswith(f"## 0.1.0 ({datetime.date.today()})"))

    def test_apply_creates_changelog_and_removes_fragments(self):
        self.sample()
        code, _, _ = self.run_main("0.1.0", "--date", "2026-01-02", "--apply")
        self.assertEqual(code, 0)
        text = (self.root / "CHANGELOG.md").read_text()
        self.assertTrue(text.startswith("# Changelog\n\n## 0.1.0 (2026-01-02)\n"))
        self.assertNotIn("Refactor", text)
        self.assertEqual(self.files(), ["README.md"])

    def test_apply_prepends_to_existing(self):
        (self.root / "CHANGELOG.md").write_text(
            "# Changelog\n\n## 0.0.1 (2025-12-01)\n\n### New\n\n- Old.\n"
        )
        self.add("5.fix.md", "New fix.")
        self.run_main("0.1.0", "--date", "2026-01-02", "--apply")
        text = (self.root / "CHANGELOG.md").read_text()
        self.assertEqual(text.count("# Changelog"), 1)
        self.assertLess(text.index("## 0.1.0"), text.index("## 0.0.1"))

    def assert_fails_untouched(self, *argv, expect):
        before = self.files()
        code, out, err = self.run_main(*argv)
        self.assertEqual(code, 1)
        self.assertEqual(out, "")
        self.assertIn(expect, err)
        self.assertEqual(self.files(), before)
        self.assertFalse((self.root / "CHANGELOG.md").exists())

    def test_bad_names_fail_before_changes(self):
        self.add("1.fix.md", "Good.")
        self.add("notes.md", "Bad name.")
        self.add("Bad_Id.fix.md", "Bad id.")
        self.assert_fails_untouched("0.1.0", "--apply", expect="notes.md")
        _, _, err = self.run_main("0.1.0")
        self.assertIn("Bad_Id.fix.md", err)

    def test_unknown_type_fails(self):
        self.add("1.bogus.md", "Text.")
        self.assert_fails_untouched("0.1.0", "--apply", expect="bogus")

    def test_empty_fragment_fails(self):
        self.add("1.fix.md", "Good.")
        self.add("2.fix.md", "  \n\n")
        self.assert_fails_untouched("0.1.0", "--apply", expect="2.fix.md")

    def test_bad_date_fails(self):
        self.add("1.fix.md", "Good.")
        self.assert_fails_untouched("0.1.0", "--date", "nope", expect="date")


if __name__ == "__main__":
    unittest.main()
