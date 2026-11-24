#!/usr/bin/env python3
"""Turn ROADMAP.md into GitHub labels and issues, once.

Usage:
    python3 scripts/roadmap_to_issues.py            # dry run: print what would happen
    python3 scripts/roadmap_to_issues.py --apply    # create labels and issues with `gh`

Needs the GitHub CLI (`gh`), logged in with access to the repo.
Safe to re-run: tasks that already have an issue (matched by "[ID]" in the
title) are skipped, and existing labels are left alone.
"""

import json
import re
import subprocess
import sys
from pathlib import Path

ROADMAP = Path(__file__).resolve().parent.parent / "ROADMAP.md"

LABELS = {
    "good first issue": ("7057ff", "Small, with the files named. Good for newcomers."),
    "agent-ready": ("2B57D9", "Clear goal, files, acceptance criteria and tests"),
    "agent-go": ("1E44B8", "Maintainers only: starts the project's agent"),
    "needs-human": ("FBCA04", "Needs real devices, accounts or a judgement call"),
    "needs-decision": ("D93B3B", "Blocked until a maintainer decides"),
    "accepted": ("1F9D63", "Feature request agreed by maintainers"),
    "security": ("B60205", "Sign-in, credentials, TLS, encryption or the sanitiser"),
    "post-1.0": ("D5DBEA", "Not until after 1.0. Please don't claim yet."),
    "area:core": ("BFD0FF", "Rust core"),
    "area:android": ("BFD0FF", "Android app"),
    "area:ios": ("BFD0FF", "iOS app"),
    "area:design": ("BFD0FF", "Brand, icons and UI design"),
    "area:docs": ("BFD0FF", "Documentation"),
    "area:ci": ("BFD0FF", "CI, tooling and repo setup"),
    **{f"m{i}": ("E3EAFF", f"Milestone M{i}") for i in range(7)},
    "pre-launch": ("E3EAFF", "Checks before going public"),
}

MARKERS = {
    "🟢": "good first issue",
    "🤖": "agent-ready",
    "🔒": "security",
    "🧑": "needs-human",
}

TASK = re.compile(r"^- \[( |x)\] \*\*(?P<id>[A-Z]\d*-\d+)\*\* (?P<rest>.+)$")
HEADING = re.compile(r"^## (?P<title>.+)$")


def guess_area(text: str) -> str:
    t = text.lower()
    if "ios" in t and "android" not in t:
        return "area:ios"
    if "android" in t and "ios" not in t:
        return "area:android"
    if any(w in t for w in ("brand", "icon", "design", "illustration", "empty state")):
        return "area:design"
    if any(w in t for w in ("ci", "github", "dependabot", "renovate", "release", "secret", "label")):
        return "area:ci"
    if any(w in t for w in ("doc", "website", "policy", "readme")):
        return "area:docs"
    return "area:core"


def parse_roadmap(text: str):
    tasks, section, milestone = [], "", ""
    for line in text.splitlines():
        if m := HEADING.match(line):
            section = m["title"]
            mm = re.match(r"M(\d)", section)
            milestone = f"m{mm[1]}" if mm else "pre-launch"
            continue
        m = TASK.match(line)
        if not m or line.startswith("- [x]"):
            continue
        rest = m["rest"]
        labels = {lab for mark, lab in MARKERS.items() if mark in rest}
        title = rest
        for mark in MARKERS:
            title = title.replace(mark, "")
        title = re.sub(r"\s+", " ", title).strip()
        # Agent-ready only once someone writes proper acceptance criteria,
        # so imported tasks get a reminder rather than the label.
        agent_candidate = "agent-ready" in labels
        labels.discard("agent-ready")
        labels.add(milestone)
        if milestone != "pre-launch":
            labels.add(guess_area(title))
        tasks.append({
            "id": m["id"], "title": title, "section": section,
            "labels": sorted(labels), "agent_candidate": agent_candidate,
        })
    return tasks


def body_for(task) -> str:
    note = (
        "\n> 🤖 This looks suitable for an AI agent. Once the sections below are filled in properly, "
        "a maintainer can add the `agent-ready` label.\n"
        if task["agent_candidate"] else ""
    )
    return f"""**Roadmap ID:** {task['id']} ({task['section']})
{note}
## Goal

{task['title']}

## Files and places

_To be filled in._

## Acceptance criteria

- [ ] _To be filled in._

## How to test

_To be filled in._

---
Want this? Comment `/claim`. See [CONTRIBUTING.md](../blob/main/CONTRIBUTING.md).
"""


def gh(*args, capture=False):
    result = subprocess.run(["gh", *args], check=True, text=True,
                            capture_output=capture)
    return result.stdout if capture else None


def main():
    apply = "--apply" in sys.argv
    tasks = parse_roadmap(ROADMAP.read_text(encoding="utf-8"))

    existing_titles = set()
    existing_labels = set()
    if apply:
        existing_labels = {l["name"] for l in json.loads(
            gh("label", "list", "--limit", "200", "--json", "name", capture=True))}
        existing_titles = {i["title"] for i in json.loads(
            gh("issue", "list", "--state", "all", "--limit", "1000", "--json", "title", capture=True))}

    for name, (colour, desc) in LABELS.items():
        if name in existing_labels:
            continue
        print(f"label  {name}")
        if apply:
            gh("label", "create", name, "--color", colour, "--description", desc, "--force")

    for task in tasks:
        tag = f"[{task['id']}]"
        if any(t.startswith(tag) for t in existing_titles):
            print(f"skip   {tag} already has an issue")
            continue
        title = f"{tag} {task['title']}"
        print(f"issue  {title}  ({', '.join(task['labels'])})")
        if apply:
            args = ["issue", "create", "--title", title, "--body", body_for(task)]
            for lab in task["labels"]:
                args += ["--label", lab]
            gh(*args)

    print(f"\n{len(tasks)} open tasks found." + ("" if apply else " Dry run: add --apply to create them."))


if __name__ == "__main__":
    main()
