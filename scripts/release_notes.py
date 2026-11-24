#!/usr/bin/env python3
"""Turn the fragments in docs/changelog.d/ into release notes.

Usage:
    python scripts/release_notes.py <version> [--date YYYY-MM-DD] [--apply]
                                    [--include-internal]

Default is a dry run that prints the Markdown section. With --apply the
section is put at the top of CHANGELOG.md and the used fragments are deleted
(with `git rm` inside a git repo). Standard library only.
"""

import argparse
import datetime
import re
import subprocess
import sys
from pathlib import Path

# (type, heading), in output order.
GROUPS = [
    ("security", "Security"),
    ("feature", "New"),
    ("change", "Changed"),
    ("fix", "Fixed"),
    ("internal", "Internal"),
]
TYPES = [t for t, _ in GROUPS]
NAME_RE = re.compile(r"^([a-z0-9-]+)\.([a-z]+)\.md$")


class ReleaseNotesError(Exception):
    """A problem the user must fix; reported with exit code 1."""


def collect(fragment_dir):
    """Return a list of (path, id, type, text). Raises before changing anything."""
    problems = []
    entries = []
    for path in sorted(Path(fragment_dir).glob("*.md")):
        if path.name == "README.md":
            continue
        match = NAME_RE.match(path.name)
        if not match:
            problems.append(f"{path.name}: name must be <id>.<type>.md")
            continue
        frag_id, frag_type = match.groups()
        if frag_type not in TYPES:
            problems.append(
                f"{path.name}: unknown type '{frag_type}' (use {', '.join(TYPES)})"
            )
            continue
        try:
            raw = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError) as err:
            problems.append(f"{path.name}: cannot read ({err})")
            continue
        text = " ".join(raw.split())
        if not text:
            problems.append(f"{path.name}: fragment is empty")
            continue
        entries.append((path, frag_id, frag_type, text))
    if problems:
        raise ReleaseNotesError(
            "Bad changelog fragments:\n" + "\n".join(f"  {p}" for p in problems)
        )
    return entries


def render(entries, version, date, include_internal):
    lines = [f"## {version} ({date})"]
    for frag_type, heading in GROUPS:
        if frag_type == "internal" and not include_internal:
            continue
        group = [e for e in entries if e[2] == frag_type]
        if not group:
            continue
        lines += ["", f"### {heading}", ""]
        for _, frag_id, _, text in group:
            suffix = f" (#{frag_id})" if frag_id.isdigit() else ""
            lines.append(f"- {text}{suffix}")
    return "\n".join(lines) + "\n"


def prepend_changelog(changelog, section):
    heading = "# Changelog"
    existing = ""
    if changelog.exists():
        existing = changelog.read_text(encoding="utf-8")
    if existing.startswith(heading):
        rest = existing[len(heading):].lstrip("\n")
    else:
        rest = existing.lstrip("\n")
    new = f"{heading}\n\n{section}"
    if rest:
        new += f"\n{rest}"
    changelog.write_text(new, encoding="utf-8")


def remove_fragments(paths, cwd):
    paths = [str(p) for p in paths]
    try:
        inside = subprocess.run(
            ["git", "rev-parse", "--is-inside-work-tree"],
            cwd=cwd, capture_output=True, text=True,
        ).stdout.strip() == "true"
    except OSError:
        inside = False
    if inside:
        result = subprocess.run(
            ["git", "rm", "-q", "-f", "--", *paths], cwd=cwd, capture_output=True
        )
        if result.returncode == 0:
            return
    for p in paths:  # not in git, or files not tracked
        Path(p).unlink(missing_ok=True)


def main(argv=None, root=None):
    root = Path(root) if root else Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("version")
    parser.add_argument("--date", default=None, help="YYYY-MM-DD (default: today)")
    parser.add_argument("--apply", action="store_true")
    parser.add_argument("--include-internal", action="store_true")
    args = parser.parse_args(argv)

    try:
        date = args.date or datetime.date.today().isoformat()
        try:
            datetime.date.fromisoformat(date)
        except ValueError:
            raise ReleaseNotesError(f"Bad date '{date}': use YYYY-MM-DD")
        fragment_dir = root / "docs" / "changelog.d"
        entries = collect(fragment_dir)
        section = render(entries, args.version, date, args.include_internal)
        if not args.apply:
            sys.stdout.write(section)
            return 0
        prepend_changelog(root / "CHANGELOG.md", section)
        # Internal fragments are removed even when not shown.
        remove_fragments([e[0] for e in entries], root)
        return 0
    except ReleaseNotesError as err:
        print(f"error: {err}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
