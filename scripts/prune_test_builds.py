"""Deletes old nightly test builds (#425).

Keeps the newest KEEP `test-build-*` pre-releases and deletes the rest,
with their tags. It only ever touches a release that is a published
pre-release (not a draft) whose tag starts with `test-build-`: never a
`v*` release, a draft or a normal release.

    python scripts/prune_test_builds.py [--keep 7] [--dry-run] [--repo OWNER/NAME]

Needs the GitHub CLI (`gh`), signed in or with GH_TOKEN set. The
repository comes from --repo, or GH_REPO.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys

TAG = re.compile(r"test-build-[0-9A-Za-z][0-9A-Za-z.-]*")


def is_test_build(release: object) -> bool:
    """A published pre-release tagged test-build-*, and nothing else."""
    if not isinstance(release, dict):
        return False
    tag = release.get("tag_name")
    return (
        release.get("prerelease") is True
        and release.get("draft") is False
        and isinstance(tag, str)
        and TAG.fullmatch(tag) is not None
        and isinstance(release.get("created_at"), str)
    )


def to_delete(releases: list, keep: int) -> list[str]:
    """The tags of the test builds beyond the newest `keep`, newest first."""
    if keep < 1:
        raise ValueError("keep must be at least 1")
    builds = sorted(
        (r for r in releases if is_test_build(r)),
        key=lambda r: r["created_at"],
        reverse=True,
    )
    return [r["tag_name"] for r in builds[keep:]]


def list_releases(repo: str) -> list:
    # One JSON object per line, from every page.
    out = subprocess.run(
        [
            "gh", "api", "--paginate", f"repos/{repo}/releases?per_page=100",
            "--jq", ".[] | {tag_name, prerelease, draft, created_at} | @json",
        ],
        check=True, capture_output=True, text=True,
    ).stdout
    return [json.loads(line) for line in out.splitlines() if line.strip()]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--keep", type=int, default=7)
    parser.add_argument("--dry-run", action="store_true", help="list what would go, delete nothing")
    parser.add_argument("--repo", default=os.environ.get("GH_REPO", ""))
    args = parser.parse_args(argv)
    if not re.fullmatch(r"[\w.-]+/[\w.-]+", args.repo):
        print("prune_test_builds: give --repo OWNER/NAME or set GH_REPO", file=sys.stderr)
        return 2
    releases = list_releases(args.repo)
    doomed = to_delete(releases, args.keep)
    kept = sum(1 for r in releases if is_test_build(r)) - len(doomed)
    print(f"{kept} test builds kept, {len(doomed)} to delete")
    for tag in doomed:
        if args.dry_run:
            print(f"would delete {tag}")
            continue
        subprocess.run(
            ["gh", "release", "delete", tag, "--repo", args.repo, "--cleanup-tag", "--yes"],
            check=True,
        )
        print(f"deleted {tag}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
