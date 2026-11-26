import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import prune_test_builds  # noqa: E402


def release(tag, created, prerelease=True, draft=False):
    return {"tag_name": tag, "created_at": created, "prerelease": prerelease, "draft": draft}


class PruneTestBuildsTest(unittest.TestCase):
    def test_keeps_the_newest_and_deletes_the_rest_newest_first(self):
        releases = [release(f"test-build-202611{d:02}-nightly", f"2026-11-{d:02}T02:30:00Z") for d in range(1, 11)]
        self.assertEqual(
            prune_test_builds.to_delete(releases, 7),
            ["test-build-20261103-nightly", "test-build-20261102-nightly", "test-build-20261101-nightly"],
        )

    def test_never_touches_anything_but_published_test_build_prereleases(self):
        releases = [
            release("v0.1.0", "2026-01-01T00:00:00Z", prerelease=False),
            release("v0.2.0-beta", "2026-01-02T00:00:00Z"),
            release("test-build-20260103-draft", "2026-01-03T00:00:00Z", draft=True),
            release("test-build-20260104-final", "2026-01-04T00:00:00Z", prerelease=False),
            release("my-test-build-20260105", "2026-01-05T00:00:00Z"),
            release("test-build-", "2026-01-06T00:00:00Z"),
            release("test-build-20260107; rm -rf /", "2026-01-07T00:00:00Z"),
            {"tag_name": "test-build-20260108", "prerelease": True, "draft": False},
            "not a release",
        ]
        self.assertEqual(prune_test_builds.to_delete(releases, 1), [])

    def test_fewer_than_keep_deletes_nothing(self):
        releases = [release("test-build-20261126-1131", "2026-11-26T11:39:43Z")]
        self.assertEqual(prune_test_builds.to_delete(releases, 7), [])

    def test_keep_must_be_positive(self):
        with self.assertRaises(ValueError):
            prune_test_builds.to_delete([], 0)


if __name__ == "__main__":
    unittest.main()
