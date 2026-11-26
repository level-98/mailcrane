import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from scripts import update_nightly
from scripts.update_nightly import build_snapshot

REPO = "https://github.com/level-98/mailcrane"


def release(
    tag,
    published,
    name="Test build (nightly)",
    prerelease=True,
    draft=False,
    assets=None,
    target="main",
    apk="mailcrane-dev-abc1234-test.apk",
):
    if assets is None:
        assets = [{"name": apk, "state": "uploaded", "browser_download_url": f"{REPO}/releases/download/{tag}/{apk}"}]
    return {
        "tag_name": tag,
        "name": name,
        "published_at": published,
        "prerelease": prerelease,
        "draft": draft,
        "assets": assets,
        "target_commitish": target,
    }


GOOD = {
    "available": True,
    "tag": "test-build-x",
    "date": "2026-11-26",
    "commit": None,
    "commit_url": None,
    "apk_name": "mailcrane-x.apk",
    "apk_url": f"{REPO}/releases/download/test-build-x/mailcrane-x.apk",
    "release_url": f"{REPO}/releases/tag/test-build-x",
    "releases_url": f"{REPO}/releases",
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
        self.assertEqual(
            snapshot["apk_url"], f"{REPO}/releases/download/test-build-20261126-nightly/mailcrane-dev-abc1234-test.apk"
        )
        self.assertTrue(update_nightly.is_valid(snapshot))

    def test_the_commit_comes_from_the_apk_name_never_the_title(self):
        nightly = build_snapshot([release("test-build-1", "2026-11-26T02:40:00Z")])
        self.assertEqual(nightly["commit"], "abc1234")
        older = build_snapshot(
            [release("test-build-2", "2026-11-26T11:39:43Z", apk="mailcrane-81755b8-release-debugsigned.apk")]
        )
        self.assertEqual(older["commit"], "81755b8")
        # A hex-looking word in the title is not taken for a commit.
        unknown = build_snapshot(
            [release("test-build-3", "2026-11-26T01:00:00Z", name="Test build deadbeef", apk="app-release.apk")]
        )
        self.assertIsNone(unknown["commit"])
        self.assertIsNone(unknown["commit_url"])

    def test_ignores_drafts_normal_releases_foreign_or_unfinished_assets_and_other_tags(self):
        foreign = [{"name": "evil.apk", "state": "uploaded", "browser_download_url": "https://example.com/evil.apk"}]
        uploading = [
            {
                "name": "mailcrane-dev-abc1234-test.apk",
                "state": "open",
                "browser_download_url": f"{REPO}/releases/download/test-build-e/mailcrane-dev-abc1234-test.apk",
            }
        ]
        snapshot = build_snapshot(
            [
                release("test-build-20261126-a", "2026-11-26T05:00:00Z", draft=True),
                release("test-build-20261126-b", "2026-11-26T04:00:00Z", prerelease=False),
                release("test-build-20261126-c", "2026-11-26T03:00:00Z", assets=foreign),
                release("nightly-20261126", "2026-11-26T02:00:00Z"),
                release("test-build-20261126-d", "2026-11-26T01:00:00Z", assets=[]),
                release("test-build-20261126-e", "2026-11-26T00:30:00Z", assets=uploading),
            ]
        )
        self.assertFalse(snapshot["available"])
        self.assertEqual(snapshot["releases_url"], f"{REPO}/releases")

    def test_odd_payloads_dont_crash(self):
        odd = release("test-build-1", "2026-11-26T02:40:00Z")
        odd["name"] = 12345
        odd["target_commitish"] = None
        self.assertTrue(build_snapshot([odd, None, "x", {"tag_name": 7}])["available"])


class RefreshTests(unittest.TestCase):
    def refresh_offline(self, saved_text):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "nightly.json"
            if saved_text is not None:
                out.write_text(saved_text, encoding="utf-8")
            with mock.patch.object(update_nightly, "fetch_releases", side_effect=OSError("offline")):
                result = update_nightly.refresh(out)
            return result, json.loads(out.read_text(encoding="utf-8"))

    def test_an_unreachable_github_keeps_a_good_saved_snapshot(self):
        result, on_disk = self.refresh_offline(json.dumps(GOOD))
        self.assertEqual(result["tag"], "test-build-x")
        self.assertEqual(on_disk["tag"], "test-build-x")

    def test_without_a_snapshot_it_writes_none(self):
        result, on_disk = self.refresh_offline(None)
        self.assertFalse(result["available"])
        self.assertFalse(on_disk["available"])

    def test_a_corrupt_or_incomplete_snapshot_is_replaced_by_none(self):
        for bad in ['{"available": tr', '[1, 2]', json.dumps({"available": True, "tag": "test-build-x"})]:
            with self.subTest(bad=bad):
                result, on_disk = self.refresh_offline(bad)
                self.assertFalse(result["available"])
                self.assertFalse(on_disk["available"])

    def test_unexpected_api_data_falls_back_too(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "nightly.json"
            with mock.patch.object(update_nightly, "fetch_releases", side_effect=TypeError("odd")):
                self.assertFalse(update_nightly.refresh(out)["available"])


if __name__ == "__main__":
    unittest.main()
