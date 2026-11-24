#!/usr/bin/env python3
"""Tick or untick the task boxes in ROADMAP.md to match the GitHub issues.

Usage:
    python3 scripts/sync_roadmap.py           # update ROADMAP.md in place
    python3 scripts/sync_roadmap.py --check   # change nothing; exit 1 if out of date

Needs the GitHub CLI (`gh`), logged in with read access to the repo's issues.

Each roadmap task is matched to the issue whose title starts with "[ID]"
(the titles that scripts/roadmap_to_issues.py creates):

- issue closed as completed  -> "- [x]"
- issue open                 -> "- [ ]"
- issue closed as not planned, or no issue at all -> left as it is

Only the box on task lines changes; every other byte of the file is kept.
"""

import json
import re
import subprocess
import sys
from pathlib import Path

ROADMAP = Path(__file__).resolve().parent.parent / "ROADMAP.md"

TASK = re.compile(r"^- \[(?P<box>[ x])\] \*\*(?P<id>[A-Z]\d*-\d+)\*\* ")
ISSUE_ID = re.compile(r"^\[(?P<id>[A-Z]\d*-\d+)\]")


def fetch_issues():
    """Return {task ID: (state, state_reason)} from the repo's issues."""
    out = subprocess.run(
        ["gh", "issue", "list", "--state", "all", "--limit", "1000",
         "--json", "number,title,state,stateReason"],
        check=True, text=True, capture_output=True,
    ).stdout
    by_id = {}
    # Oldest first, so if a task somehow has two issues the newest one wins.
    for issue in sorted(json.loads(out), key=lambda i: i["number"]):
        m = ISSUE_ID.match(issue["title"])
        if m:
            by_id[m["id"]] = (issue["state"], issue.get("stateReason") or "")
    return by_id


def wanted_box(current: str, issue) -> str:
    if issue is None:
        return current
    state, reason = issue
    if state == "OPEN":
        return " "
    if state == "CLOSED" and reason == "COMPLETED":
        return "x"
    return current  # closed as not planned, duplicate, or unknown


def sync(text: str, issues: dict):
    """Return (new_text, list of change descriptions)."""
    lines = text.splitlines(keepends=True)
    changes = []
    for n, line in enumerate(lines):
        m = TASK.match(line)
        if not m:
            continue
        box = wanted_box(m["box"], issues.get(m["id"]))
        if box != m["box"]:
            lines[n] = line[:3] + box + line[4:]
            changes.append(f"{m['id']}: {'ticked' if box == 'x' else 'unticked'}")
    return "".join(lines), changes


def main():
    check = "--check" in sys.argv
    # Read and write bytes-as-text without newline translation, so CRLF/LF
    # line endings are preserved exactly.
    with ROADMAP.open(encoding="utf-8", newline="") as f:
        text = f.read()
    new_text, changes = sync(text, fetch_issues())

    for c in changes:
        print(c)
    if not changes:
        print("ROADMAP.md already matches the issues.")
        return 0
    if check:
        print(f"\n{len(changes)} task(s) out of date.")
        return 1
    with ROADMAP.open("w", encoding="utf-8", newline="") as f:
        f.write(new_text)
    print(f"\nUpdated {len(changes)} task(s) in ROADMAP.md.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
