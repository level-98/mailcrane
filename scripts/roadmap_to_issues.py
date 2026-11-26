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
    "area:design": ("BFD0FF", "Brand, icons and UI design"),
    "area:docs": ("BFD0FF", "Documentation"),
    "area:ci": ("BFD0FF", "CI, tooling and repo setup"),
    **{f"m{i}": ("E3EAFF", f"Milestone M{i}") for i in range(9)},
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


AREA_WORDS = {
    # Matched as whole words or phrases, case-insensitive, so "ci" doesn't hit
    # "decision" and "label" doesn't hit "Gmail labels".
    "area:android": ["android", "jetpack compose", "compose theme", "material you",
                     "talkback", "f-droid", "google play", "glance", "tablet", "keystore"],
    "area:design": ["brand", "brand pack", "design", "illustrations", "empty states"],
    "area:ci": ["ci", "github actions", "dependabot", "renovate", "release builds",
                "release script", "secret", "issue per task",
                "branch protection"],
    "area:docs": ["docs", "doc", "website", "policy", "readme"],
}

# UI work, even though no platform is named.
APP_WORDS = ["dark mode", "swipe actions", "biometrics", "widgets",
             "compose screen", "rich-text compose"]


def _has(text: str, word: str) -> bool:
    return re.search(rf"(?<![\w-]){re.escape(word)}(?![\w-])", text, re.IGNORECASE) is not None


def guess_areas(text: str) -> set:
    """Return one or more area labels: CI and tooling first, then the
    Android app, then design or docs, and otherwise the Rust core."""
    if any(_has(text, w) for w in AREA_WORDS["area:ci"]):
        return {"area:ci"}
    if any(_has(text, w) for w in AREA_WORDS["area:android"] + APP_WORDS):
        return {"area:android"}
    for area in ("area:design", "area:docs"):
        if any(_has(text, w) for w in AREA_WORDS[area]):
            return {area}
    return {"area:core"}


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
        if milestone == "m8":
            labels.add("post-1.0")
        if milestone != "pre-launch":
            labels |= guess_areas(title)
        tasks.append({
            "id": m["id"], "title": title, "section": section, "milestone": milestone,
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
    milestones = set()
    if apply:
        milestones = set(gh("api", "repos/{owner}/{repo}/milestones?state=all&per_page=100",
                            "--jq", ".[].title", capture=True).split())
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
            # Milestones are named like the labels: m0, m1, …
            if task["milestone"] in milestones:
                args += ["--milestone", task["milestone"]]
            gh(*args)

    print(f"\n{len(tasks)} open tasks found." + ("" if apply else " Dry run: add --apply to create them."))


if __name__ == "__main__":
    main()
