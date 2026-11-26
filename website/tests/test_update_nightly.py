import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from scripts import update_nightly
from scripts.update_nightly import build_snapshot

REPO = "https://github.com/level-98/mailcrane"


def release(tag, published, name="Test build abc1234 (nightly)", prerelease=True, draft=False, assets=None, target="main"):
    if assets is None:
        assets = [{"name": "mailcrane-test.apk", "browser_download_url": f"{REPO}/releases/download/{tag}/mailcrane-test.apk"}]
    return {
        "tag_name": tag,
        "name": name,
        "published_at": published,
        "prerelease": prerelease,
        "draft": draft,
        "assets": assets,
        "target_commitish": target,
    }


class BuildSnapshotTests(unittest.TestCase):
    def test_picks_the_newest_test_build_with_an_apk(self):
        snapshot = build_snapshot(
            [
                release("test-build-20261125-nightly", "2026-11-25T02:40:00Z"),
                release("test-build-20261126-nightly", "2026-11-26T02:40:00Z", target="a" * 40),
                release("v0.1.0", "2026-11-27T00:00:00Z", prerelease=False),
            ]
        )
        self.assertTrue(snapshot["available"])
        self.assertEqual(snapshot["tag"], "test-build-20261126-nightly")
        self.assertEqual(snapshot["date"], "2026-11-26")
        self.assertEqual(snapshot["commit"], "aaaaaaa")
        self.assertEqual(snapshot["commit_url"], f"{REPO}/commit/{'a' * 40}")
        self.assertEqual(snapshot["apk_url"], f"{REPO}/releases/download/test-build-20261126-nightly/mailcrane-test.apk")

    def test_commit_from_the_name_needs_a_hex_letter(self):
        snapshot = build_snapshot([release("test-build-20261126-1131", "2026-11-26T11:39:43Z", name="Test build 20261126 (abc1234)")])
        self.assertEqual(snapshot["commit"], "abc1234")

    def test_ignores_drafts_normal_releases_foreign_assets_and_other_tags(self):
        foreign = [{"name": "evil.apk", "browser_download_url": "https://example.com/evil.apk"}]
        snapshot = build_snapshot(
            [
                release("test-build-20261126-a", "2026-11-26T05:00:00Z", draft=True),
                release("test-build-20261126-b", "2026-11-26T04:00:00Z", prerelease=False),
                release("test-build-20261126-c", "2026-11-26T03:00:00Z", assets=foreign),
                release("nightly-20261126", "2026-11-26T02:00:00Z"),
                release("test-build-20261126-d", "2026-11-26T01:00:00Z", assets=[]),
            ]
        )
        self.assertFalse(snapshot["available"])
        self.assertEqual(snapshot["releases_url"], f"{REPO}/releases")

    def test_an_unreachable_github_keeps_the_saved_snapshot(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "nightly.json"
            out.write_text(json.dumps({"available": True, "tag": "test-build-x"}), encoding="utf-8")
            with mock.patch.object(update_nightly, "fetch_releases", side_effect=OSError("offline")):
                kept = update_nightly.refresh(out)
            self.assertEqual(kept["tag"], "test-build-x")
            self.assertEqual(json.loads(out.read_text(encoding="utf-8"))["tag"], "test-build-x")

    def test_an_unreachable_github_without_a_snapshot_writes_none(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "nightly.json"
            with mock.patch.object(update_nightly, "fetch_releases", side_effect=OSError("offline")):
                update_nightly.refresh(out)
            self.assertFalse(json.loads(out.read_text(encoding="utf-8"))["available"])


if __name__ == "__main__":
    unittest.main()
