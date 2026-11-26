#!/usr/bin/env python3
"""Write data/nightly.json: the latest nightly test build, for the Download page (#425).

Read at build time, like data/roadmap.json, so the page itself never calls
GitHub. Only a published `test-build-*` pre-release counts, and only an APK
served from this repository's own release downloads is linked.
"""

from __future__ import annotations

import json
import os
import re
import sys
from pathlib import Path
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

REPOSITORY = "level-98/mailcrane"
API_URL = f"https://api.github.com/repos/{REPOSITORY}/releases?per_page=50"
REPO_URL = f"https://github.com/{REPOSITORY}"
DOWNLOAD_PREFIX = f"{REPO_URL}/releases/download/"
TAG = re.compile(r"test-build-[0-9A-Za-z][0-9A-Za-z.-]*")
# The commit in the APK names the builds use: mailcrane-81755b8-release-debugsigned.apk (older test builds)
# and mailcrane-dev-1a2b3c4-test.apk (nightly.yml). Never guessed from the release title.
ASSET_SHA = re.compile(r"mailcrane-(?:dev-)?([0-9a-f]{7,40})-[^/]*\.apk")
OUTPUT = Path(__file__).resolve().parents[1] / "data" / "nightly.json"
NONE = {"available": False, "releases_url": f"{REPO_URL}/releases"}
# What the Download page reads from an available snapshot.
REQUIRED = ("tag", "date", "apk_name", "apk_url", "release_url", "releases_url")


def is_test_build(release: object) -> bool:
    if not isinstance(release, dict):
        return False
    tag = release.get("tag_name")
    return (
        release.get("prerelease") is True
        and release.get("draft") is False
        and isinstance(tag, str)
        and TAG.fullmatch(tag) is not None
    )


def commit_of(release: dict, apk_name: str) -> str | None:
    """A full target SHA, else the one in the APK's name; None rather than a guess."""
    target = release.get("target_commitish")
    if isinstance(target, str) and re.fullmatch(r"[0-9a-f]{40}", target):
        return target
    match = ASSET_SHA.fullmatch(apk_name)
    return match.group(1) if match else None


def is_valid(snapshot: object) -> bool:
    """A snapshot the Download page can render: not available, or with every field it shows."""
    if not isinstance(snapshot, dict):
        return False
    if snapshot.get("available") is not True:
        return snapshot.get("available") is False
    return all(isinstance(snapshot.get(key), str) and snapshot[key] for key in REQUIRED) and str(
        snapshot["apk_url"]
    ).startswith(DOWNLOAD_PREFIX)


def build_snapshot(releases: list) -> dict:
    """The newest test build with an APK from this repository, or NONE."""
    builds = sorted(
        (r for r in releases if is_test_build(r) and isinstance(r.get("published_at"), str)),
        key=lambda r: r["published_at"],
        reverse=True,
    )
    for build in builds:
        apk = next(
            (
                a
                for a in build.get("assets") or []
                if isinstance(a, dict)
                # Still uploading ("open", "starter") would be a 404 until the next rebuild.
                and a.get("state") == "uploaded"
                and str(a.get("name", "")).endswith(".apk")
                and str(a.get("browser_download_url", "")).startswith(DOWNLOAD_PREFIX)
            ),
            None,
        )
        if apk is None:
            continue
        commit = commit_of(build, str(apk["name"]))
        return {
            "available": True,
            "tag": build["tag_name"],
            "date": build["published_at"][:10],
            "commit": commit[:7] if commit else None,
            "commit_url": f"{REPO_URL}/commit/{commit}" if commit else None,
            "apk_name": apk["name"],
            "apk_url": apk["browser_download_url"],
            "release_url": f"{REPO_URL}/releases/tag/{build['tag_name']}",
            "releases_url": NONE["releases_url"],
        }
    return dict(NONE)


def fetch_releases() -> list:
    headers = {
        "Accept": "application/vnd.github+json",
        "User-Agent": "Mailcrane-nightly-snapshot",
        "X-GitHub-Api-Version": "2022-11-28",
    }
    token = os.environ.get("GITHUB_TOKEN")
    if token:
        headers["Authorization"] = f"Bearer {token}"
    with urlopen(Request(API_URL, headers=headers), timeout=30) as response:
        releases = json.load(response)
    if not isinstance(releases, list):
        raise RuntimeError("GitHub API returned an unexpected release list.")
    return releases


def saved(output: Path) -> dict | None:
    """The saved snapshot, if it is there, readable and complete."""
    try:
        snapshot = json.loads(output.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None
    return snapshot if is_valid(snapshot) else None


def refresh(output: Path = OUTPUT) -> dict:
    """Rewrites `output`. When GitHub can't be reached, keeps a good saved snapshot, or says none."""
    try:
        snapshot = build_snapshot(fetch_releases())
    except (HTTPError, URLError, RuntimeError, ValueError, TypeError, KeyError, OSError) as error:
        kept = saved(output)
        if kept is not None:
            print(f"nightly: kept the saved snapshot ({error})", file=sys.stderr)
            return kept
        print(f"nightly: no usable snapshot, writing none ({error})", file=sys.stderr)
        snapshot = dict(NONE)
    output.write_text(json.dumps(snapshot, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"nightly: {snapshot.get('tag') or 'no test build'} -> {output}")
    return snapshot


if __name__ == "__main__":
    refresh()
    sys.exit(0)
