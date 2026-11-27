import tempfile
import unittest
from pathlib import Path
from unittest import mock

from scripts import build_privacy
from scripts.build_privacy import PolicyError, build_page, policy_body

TEMPLATE = "+++\ntitle = \"Privacy\"\n+++\n\n## This website\n\nStatic.\n\n## The app\n\n{{ policy }}\n"
POLICY = """# Privacy policy

<!-- Edit the policy here only. -->

> **Draft:** not in effect yet.

Mailcrane collects nothing.

## Where your data goes

On your phone.

### Detail

- **Push wake-ups.** Optional.
"""


class PolicyBodyTest(unittest.TestCase):
    def test_drops_title_and_comments_and_shifts_headings(self):
        body = policy_body(POLICY)
        self.assertNotIn("# Privacy policy", body)
        self.assertNotIn("<!--", body)
        self.assertTrue(body.startswith("> **Draft:**"))
        self.assertIn("\n### Where your data goes\n", body)
        self.assertIn("\n#### Detail\n", body)
        self.assertIn("- **Push wake-ups.** Optional.", body)

    def test_every_policy_line_reaches_the_page(self):
        # The oracle for "can't drift": apart from the title and comments,
        # each line of the policy is in the page, unchanged or one level down.
        page = build_page(POLICY, TEMPLATE)
        for line in POLICY.splitlines()[1:]:
            if not line or line.startswith("<!--"):
                continue
            expected = "#" + line if line.startswith("#") else line
            self.assertIn(expected, page.splitlines())

    def test_windows_line_endings(self):
        self.assertEqual(policy_body(POLICY.replace("\n", "\r\n")), policy_body(POLICY))

    def test_headings_inside_code_blocks_stay(self):
        body = policy_body("# T\n\n```\n# not a heading\n## nor this\n```\n\n## Real\n")
        self.assertIn("\n# not a heading\n## nor this\n", body)
        self.assertIn("### Real", body)

    def test_hashtag_without_space_is_not_a_heading(self):
        self.assertIn("#475 is text", policy_body("# T\n\n#475 is text\n"))

    def test_refuses_a_policy_it_cannot_place(self):
        for bad in ("", "No title\n", "# T\n", "# T\n\n# Second title\n", "# T\n\n###### Too deep\n"):
            with self.subTest(bad=bad), self.assertRaises(PolicyError):
                policy_body(bad)


class BuildPageTest(unittest.TestCase):
    def test_policy_goes_under_the_app(self):
        page = build_page(POLICY, TEMPLATE)
        self.assertLess(page.index("## This website"), page.index("## The app"))
        self.assertLess(page.index("## The app"), page.index("### Where your data goes"))
        self.assertNotIn("{{ policy }}", page)
        self.assertTrue(page.endswith("- **Push wake-ups.** Optional.\n"))

    def test_template_needs_one_placeholder(self):
        for template in ("no placeholder", "{{ policy }} {{ policy }}"):
            with self.subTest(template=template), self.assertRaises(PolicyError):
                build_page(POLICY, template)


class MainTest(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)
        root = Path(self.dir.name)
        self.policy = root / "privacy.md"
        self.policy.write_text(POLICY, encoding="utf-8")
        self.output = root / "out.md"
        template = root / "template.md"
        template.write_text(TEMPLATE, encoding="utf-8")
        patches = [
            mock.patch.object(build_privacy, "OUTPUT", self.output),
            mock.patch.object(build_privacy, "TEMPLATE", template),
            mock.patch.object(build_privacy, "ROOT", root),
        ]
        for patch in patches:
            patch.start()
            self.addCleanup(patch.stop)

    def test_writes_then_check_passes(self):
        self.assertEqual(build_privacy.main([str(self.policy)]), 0)
        self.assertEqual(self.output.read_text(encoding="utf-8"), build_page(POLICY, TEMPLATE))
        self.assertEqual(build_privacy.main([str(self.policy), "--check"]), 0)

    def test_check_fails_when_the_page_drifts(self):
        build_privacy.main([str(self.policy)])
        self.policy.write_text(POLICY + "\n- **New connection.**\n", encoding="utf-8")
        with mock.patch("sys.stderr"):
            self.assertEqual(build_privacy.main([str(self.policy), "--check"]), 1)

    def test_missing_policy_fails_without_writing(self):
        with mock.patch("sys.stderr"):
            self.assertEqual(build_privacy.main([str(self.policy.with_name("missing.md"))]), 1)
        self.assertFalse(self.output.exists())

    def test_usage(self):
        with mock.patch("sys.stderr"):
            self.assertEqual(build_privacy.main([]), 2)


if __name__ == "__main__":
    unittest.main()
