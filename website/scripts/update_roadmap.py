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
TASK_PATTERN = re.compile(r"^\[((?:M[0-8])|P)-(\d{2})\]\s+(.+)$", re.IGNORECASE)
# Milestones list features only. A bug is an issue labelled `bug` and never
# appears on the roadmap, even if its title carries a task ID.
BUG_LABEL = "bug"
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
    "M8": {
        "title": "After 1.0",
        "description": "Good ideas that wait until the Android 1.0 release is out.",
    },
    "P": {
        "title": "Before public launch",
        "description": "Resolve the project, store, domain, and naming checks before going public.",
    },
}

# No iOS app before 1.0 (decision 0005): its milestone, M7, isn't on the
# roadmap, and neither are the iOS tasks that kept an older ID when they
# moved there.
SKIPPED_GROUPS = {"M7"}
SKIPPED_TASKS = {"M0-16", "M1-11", "M6-06"}


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


def is_bug(issue: dict) -> bool:
    """Whether the issue is labelled as a bug (bugs are not roadmap tasks)."""
    labels = issue.get("labels") or []
    return any(
        (label.get("name") if isinstance(label, dict) else label) == BUG_LABEL
        for label in labels
    )


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
        if not match or is_bug(issue):
            continue
        if state not in {"open", "closed"} or not isinstance(issue_url, str):
            raise RuntimeError(f"GitHub API returned invalid state or URL for issue #{number}.")

        group_id, sequence, task_title = match.groups()
        group_id = group_id.upper()
        task_id = f"{group_id}-{sequence}"
        if group_id in SKIPPED_GROUPS or task_id in SKIPPED_TASKS:
            continue
        position = float(sequence)
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
    # The Download page's nightly test build (#425) comes with the same
    # refresh, so every workflow that builds the site updates it. It never
    # fails the roadmap: without GitHub it keeps the saved snapshot.
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import update_nightly  # noqa: E402

    update_nightly.refresh()
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError) as error:
        print(f"Roadmap update failed: {error}", file=sys.stderr)
        raise SystemExit(1) from error
