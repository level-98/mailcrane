#!/usr/bin/env python3
"""Mechanical pull request checks for CI (#606).

    pr_checks.py labels FILES          the labels a PR's changed files call for, one per line
    pr_checks.py fragment FILES LABELS fails if code changed with no changelog fragment
    pr_checks.py qa-gate LABELS        fails while the PR waits for QA

FILES is a file listing the changed paths, one per line. LABELS is a JSON array of the PR's current
label names. CI calls these from .github/workflows/ci.yml; the rules live here so they're tested
(scripts/test_pr_checks.py, run by `cargo xtask ci`'s design step), not written in YAML.

Fragment names and types are checked separately by `release_notes.py check`.
"""

import fnmatch
import json
import sys

# Labels a run may add. A PR's other labels (fleet, bug, ...) are never touched.
NEEDS_QA = "needs-qa"
QA_LIGHT = "qa-light"
QA_VERIFIED = "qa-verified"
NO_CHANGELOG = "no-changelog"

# Paths whose changes touch credentials or secure transport: the security label (review looks closer).
SECURITY_PATTERNS = [
    "core/crates/*/src/**/auth*",
    "core/crates/*/src/**/secret*",
    "core/crates/*/src/**/crypto*",
    "core/crates/*/src/**/tls*",
    "core/crates/*/src/**/credentials*",
    "core/crates/*/src/**/oauth/*",
]


def _match(path, pattern):
    # fnmatch's `*` crosses `/`, so `**` and `*` both match any depth here.
    return fnmatch.fnmatchcase(path, pattern.replace("**/", "*"))


def _is_android_test(path):
    return path.startswith("android/") and ("/src/test/" in path or "/src/androidTest/" in path)


def _is_docs(path):
    return (
        (path.startswith("docs/") and path != "docs/dev-mail.md")
        or path.startswith("website/")
        or ("/" not in path and path.endswith(".md"))
    )


def labels_for(files):
    """The labels these changed files call for.

    `needs-qa` when anything under android/ outside its tests changed; `qa-light` when only android
    tests did (never both, so qa-light can't let a UI change through the gate); an `area:` label per
    part of the tree; `security` for credential and TLS code.
    """
    files = [f for f in files if f]
    labels = set()
    android = [f for f in files if f.startswith("android/")]
    if android:
        labels.add("area:android")
        if any(not _is_android_test(f) for f in android):
            labels.add(NEEDS_QA)
        else:
            labels.add(QA_LIGHT)
    if any(f.startswith("core/") for f in files):
        labels.add("area:core")
    if any(_is_docs(f) for f in files):
        labels.add("area:docs")
    if any(f.startswith((".github/", "scripts/", "core/xtask/")) for f in files):
        labels.add("area:ci")
    if any(_match(f, p) for f in files for p in SECURITY_PATTERNS):
        labels.add("security")
    return labels


def fragment_problem(files, labels):
    """Why this PR fails the fragment check, or None.

    Code under core/ or android/app/src/main/ needs a changelog fragment in docs/changelog.d/ unless the
    PR is labelled no-changelog.
    """
    files = [f for f in files if f]
    code = [f for f in files if f.startswith(("core/", "android/app/src/main/"))]
    if not code or NO_CHANGELOG in labels:
        return None
    fragment = any(f.startswith("docs/changelog.d/") and f.endswith(".md") for f in files)
    if fragment:
        return None
    return (
        f"This PR changes code ({code[0]}{' and more' if len(code) > 1 else ''}) but adds no changelog "
        "fragment. Add docs/changelog.d/<issue>.<type>.md (see CONTRIBUTING.md), or label the PR "
        f"'{NO_CHANGELOG}' if the change needs no entry."
    )


def qa_gate_problem(labels):
    """Why the PR is still waiting for QA, or None."""
    if NEEDS_QA in labels and QA_VERIFIED not in labels and QA_LIGHT not in labels:
        return (
            f"Waiting for QA: the PR has '{NEEDS_QA}' and not yet '{QA_VERIFIED}'. The builds may well "
            "have passed; this check turns green once QA labels the PR."
        )
    return None


def _read_files(path):
    with open(path, encoding="utf-8") as f:
        return [line.strip() for line in f]


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    if not argv:
        print(__doc__, file=sys.stderr)
        return 2
    command, rest = argv[0], argv[1:]
    if command == "labels" and len(rest) == 1:
        for label in sorted(labels_for(_read_files(rest[0]))):
            print(label)
        return 0
    if command == "fragment" and len(rest) == 2:
        problem = fragment_problem(_read_files(rest[0]), set(json.loads(rest[1])))
    elif command == "qa-gate" and len(rest) == 1:
        problem = qa_gate_problem(set(json.loads(rest[0])))
    else:
        print(__doc__, file=sys.stderr)
        return 2
    if problem:
        # A GitHub Actions error annotation, shown on the PR's checks.
        print(f"::error::{problem}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
