#!/usr/bin/env python3
"""Generate a local roadmap snapshot from Mailcrane's public GitHub issues."""

from __future__ import annotations

import json
import os
import re
import sys
from datetime import datetime, timezone
from pathlib import Path
from urllib.error import HTTPError, URLError
from urllib.parse import urlencode
from urllib.request import Request, urlopen


REPOSITORY = "level-98/mailcrane"
API_URL = f"https://api.github.com/repos/{REPOSITORY}/issues"
TASK_PATTERN = re.compile(r"^\[((?:M[0-7])|P)-(\d{2})\]\s+(.+)$", re.IGNORECASE)
MILESTONES = {
    "M0": {
        "title": "Foundations",
        "description": "Set up the shared core, app shells, and contributor workflow.",
    },
    "M1": {
        "title": "Read mail",
        "description": "Connect an account and read your inbox, online or offline.",
    },
    "M2": {
        "title": "Send mail",
        "description": "Compose, reply, and reliably deliver your messages.",
    },
    "M3": {
        "title": "Providers & sign-in",
        "description": "Make setup straightforward across popular providers.",
    },
    "M4": {
        "title": "Everyday features",
        "description": "Bring together the thoughtful tools that make email better.",
    },
    "M5": {
        "title": "Privacy & security",
        "description": "Keep personal mail protected and in your control.",
    },
    "M6": {
        "title": "Release",
        "description": "Test, package, and bring Mailcrane to its first users on Android.",
    },
    "M7": {
        "title": "iOS app",
        "description": "Bring the iOS app up to the Android release and ship it on the App Store.",
    },
    "P": {
        "title": "Before public launch",
        "description": "Resolve the project, store, domain, and naming checks before going public.",
    },
}

# Tasks that moved milestone keep their ID (and issue title), so they are
# placed by this map instead: task ID -> (new milestone, position). The
# position sorts the task among the new milestone's own numbered tasks, in the
# order ROADMAP.md on `main` lists them. Keep the two in step.
MOVED_TASKS = {
    # Android first; the iOS work follows after the release (decision 0002).
    "M0-16": ("M7", 2.1),
    "M1-11": ("M7", 2.2),
    "M6-06": ("M7", 99.0),
}


def fetch_issues() -> list[dict]:
    issues = []
    page = 1
    headers = {
        "Accept": "application/vnd.github+json",
        "User-Agent": "Mailcrane-roadmap-generator",
        "X-GitHub-Api-Version": "2022-11-28",
    }
    token = os.environ.get("GITHUB_TOKEN")
    if token:
        headers["Authorization"] = f"Bearer {token}"

    while True:
        url = f"{API_URL}?{urlencode({'state': 'all', 'per_page': 100, 'page': page})}"
        request = Request(url, headers=headers)
        try:
            with urlopen(request, timeout=30) as response:
                batch = json.load(response)
        except HTTPError as error:
            detail = error.read().decode("utf-8", errors="replace")
            raise RuntimeError(
                f"GitHub API request failed with HTTP {error.code}: {detail}"
            ) from error
        except URLError as error:
            raise RuntimeError(f"Could not reach the GitHub API: {error.reason}") from error

        if not isinstance(batch, list):
            raise RuntimeError("GitHub API returned an unexpected issue list.")
        issues.extend(issue for issue in batch if "pull_request" not in issue)
        if len(batch) < 100:
            return issues
        page += 1


def percentage(completed: int, total: int) -> int:
    return int(completed / total * 100 + 0.5) if total else 0


def build_snapshot(issues: list[dict]) -> dict:
    tasks_by_group = {key: [] for key in MILESTONES}
    for issue in issues:
        title = issue.get("title")
        number = issue.get("number")
        state = issue.get("state")
        issue_url = issue.get("html_url")
        if not isinstance(title, str) or not isinstance(number, int):
            raise RuntimeError("GitHub API returned an issue without a title or number.")
        match = TASK_PATTERN.match(title)
        if not match:
            continue
        if state not in {"open", "closed"} or not isinstance(issue_url, str):
            raise RuntimeError(f"GitHub API returned invalid state or URL for issue #{number}.")

        group_id, sequence, task_title = match.groups()
        group_id = group_id.upper()
        task_id = f"{group_id}-{sequence}"
        group_id, position = MOVED_TASKS.get(task_id, (group_id, float(sequence)))
        tasks_by_group[group_id].append(
            {
                "id": task_id,
                "position": position,
                "number": int(sequence),
                "issue_number": number,
                "title": task_title,
                "state": state,
                "url": issue_url,
            }
        )

    groups = []
    for group_id, details in MILESTONES.items():
        tasks = sorted(
            tasks_by_group[group_id],
            key=lambda task: (task["position"], task["issue_number"]),
        )
        if not tasks:
            continue
        completed = sum(task["state"] == "closed" for task in tasks)
        total = len(tasks)
        groups.append(
            {
                "id": group_id,
                "title": details["title"],
                "description": details["description"],
                "total": total,
                "completed": completed,
                "percent": percentage(completed, total),
                "tasks": tasks,
            }
        )

    total = sum(group["total"] for group in groups)
    completed = sum(group["completed"] for group in groups)
    return {
        "repository": REPOSITORY,
        "generated_at": datetime.now(timezone.utc).isoformat(timespec="seconds").replace(
            "+00:00", "Z"
        ),
        "total": total,
        "completed": completed,
        "percent": percentage(completed, total),
        "groups": groups,
    }


def main() -> int:
    output_path = Path(__file__).resolve().parents[1] / "data" / "roadmap.json"
    snapshot = build_snapshot(fetch_issues())
    output_path.write_text(
        json.dumps(snapshot, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    print(
        f"Wrote {snapshot['completed']}/{snapshot['total']} roadmap tasks "
        f"across {len(snapshot['groups'])} groups to {output_path}"
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError) as error:
        print(f"Roadmap update failed: {error}", file=sys.stderr)
        raise SystemExit(1) from error
