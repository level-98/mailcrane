"""Tests for scripts/pr_checks.py (#606). Run by `cargo xtask ci`'s design step."""

import json
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import pr_checks  # noqa: E402


class LabelTests(unittest.TestCase):
    def test_an_android_change_needs_qa(self):
        labels = pr_checks.labels_for(["android/app/src/main/java/app/mailcrane/ui/MailHome.kt"])
        self.assertIn("needs-qa", labels)
        self.assertIn("area:android", labels)
        self.assertNotIn("qa-light", labels)

    def test_android_tests_only_is_qa_light_never_both(self):
        labels = pr_checks.labels_for(
            [
                "android/app/src/test/java/app/mailcrane/ui/SwipeUiTest.kt",
                "android/app/src/androidTest/java/app/mailcrane/Smoke.kt",
            ]
        )
        self.assertIn("qa-light", labels)
        self.assertNotIn("needs-qa", labels)
        # One UI file as well: needs-qa, and qa-light goes, so it can't let the change through.
        labels = pr_checks.labels_for(
            ["android/app/src/test/java/X.kt", "android/app/src/main/res/values/strings.xml"]
        )
        self.assertEqual({"needs-qa", "area:android"} & labels, {"needs-qa", "area:android"})
        self.assertNotIn("qa-light", labels)

    def test_a_docs_only_change_needs_no_qa(self):
        labels = pr_checks.labels_for(["docs/fleet.md", "README.md", "website/content/_index.md"])
        self.assertEqual(labels, {"area:docs"})

    def test_areas_and_security(self):
        labels = pr_checks.labels_for(
            [
                "core/crates/mailcrane-imap/src/tls.rs",
                ".github/workflows/ci.yml",
                "core/crates/mailcrane-core/src/account/credentials.rs",
            ]
        )
        self.assertEqual(labels, {"area:core", "area:ci", "security"})
        self.assertNotIn("security", pr_checks.labels_for(["core/crates/mailcrane-mime/src/snippet.rs"]))
        # dev-mail.md is read by an xtask test: code, not docs.
        self.assertNotIn("area:docs", pr_checks.labels_for(["docs/dev-mail.md"]))


class FragmentTests(unittest.TestCase):
    def test_code_without_a_fragment_fails_with_a_clear_message(self):
        problem = pr_checks.fragment_problem(["core/crates/mailcrane-store/src/lib.rs"], set())
        self.assertIsNotNone(problem)
        self.assertIn("docs/changelog.d/", problem)
        self.assertIn("no-changelog", problem)

    def test_a_fragment_or_the_label_passes(self):
        files = ["android/app/src/main/java/X.kt", "docs/changelog.d/553.feature.md"]
        self.assertIsNone(pr_checks.fragment_problem(files, set()))
        self.assertIsNone(pr_checks.fragment_problem(["core/x.rs"], {"no-changelog"}))

    def test_changes_outside_the_app_need_none(self):
        for files in (["docs/fleet.md"], ["android/app/src/test/java/X.kt"], [".github/workflows/ci.yml"]):
            self.assertIsNone(pr_checks.fragment_problem(files, set()), files)


class QaGateTests(unittest.TestCase):
    def test_needs_qa_waits_until_verified_or_light(self):
        self.assertIsNotNone(pr_checks.qa_gate_problem({"needs-qa", "fleet"}))
        self.assertIsNone(pr_checks.qa_gate_problem({"needs-qa", "qa-verified"}))
        self.assertIsNone(pr_checks.qa_gate_problem({"needs-qa", "qa-light"}))
        self.assertIsNone(pr_checks.qa_gate_problem({"area:docs"}))


class MainTests(unittest.TestCase):
    def run_main(self, *argv):
        return pr_checks.main(list(argv))

    def test_the_commands(self):
        with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False, encoding="utf-8") as f:
            f.write("core/crates/mailcrane-core/src/lib.rs\n")
            files = f.name
        try:
            self.assertEqual(self.run_main("fragment", files, json.dumps([])), 1)
            self.assertEqual(self.run_main("fragment", files, json.dumps(["no-changelog"])), 0)
            self.assertEqual(self.run_main("labels", files), 0)
        finally:
            os.unlink(files)
        self.assertEqual(self.run_main("qa-gate", json.dumps(["needs-qa"])), 1)
        self.assertEqual(self.run_main("qa-gate", json.dumps(["needs-qa", "qa-verified"])), 0)
        self.assertEqual(self.run_main("nonsense"), 2)


if __name__ == "__main__":
    unittest.main()
