#!/usr/bin/env python3
"""Write content/privacy.md from the app's privacy policy, docs/privacy.md on main (#475).

docs/privacy.md is the only copy of the policy. The site keeps its own part
("This website") in scripts/privacy_template.md, and this script puts the
policy under "The app": its title and whole-line HTML comments are dropped
and every heading moves down one level. The deploy workflows run it with
main's file before `zola build`, so the live page can't drift from the policy.

The policy goes into a Zola page, so anything Zola or a browser would treat
as code is refused rather than published: `{{`, `{%`, and HTML tags or
comments inside a line. So are headings this script can't move down
(underlined ones, or level 6).

    python scripts/build_privacy.py <path to docs/privacy.md>          # write the page
    python scripts/build_privacy.py <path to docs/privacy.md> --check  # fail if it differs
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TEMPLATE = ROOT / "scripts" / "privacy_template.md"
OUTPUT = ROOT / "content" / "privacy.md"
PLACEHOLDER = "{{ policy }}"
# CommonMark: up to 3 spaces of indent, then 3 or more ` or ~.
FENCE = re.compile(r"^ {0,3}(`{3,}|~{3,})(.*)$")
HEADING = re.compile(r"^ {0,3}(#{1,6})(?=\s|$)")
SETEXT = re.compile(r"^ {0,3}(=+|-+)\s*$")
# A tag (<p>, </div>, <br/>) or comment/declaration (<!...). Not an autolink
# such as <https://example.org>, where a colon follows the scheme.
HTML = re.compile(r"<(/?[A-Za-z][A-Za-z0-9-]*(\s|/?>|$)|!)")
TEMPLATING = re.compile(r"\{\{|\{%")


class PolicyError(ValueError):
    """The policy file isn't in the shape the page needs."""


def _refuse(number: int, why: str) -> PolicyError:
    return PolicyError(f"docs/privacy.md line {number}: {why}")


def policy_body(policy: str) -> str:
    """The policy without its title and comments, with headings one level down."""
    lines = policy.lstrip("﻿").replace("\r\n", "\n").split("\n")
    out: list[str] = []
    fence: tuple[str, int] | None = None  # (character, length) of the open fence
    in_comment = False
    title_seen = False
    previous_text = False  # the line before was a paragraph line
    for number, line in enumerate(lines, start=1):
        if TEMPLATING.search(line):
            # Zola reads these as shortcodes even inside code blocks.
            raise _refuse(number, "'{{' and '{%' would be read by Zola; reword the line")
        if fence is not None:
            marker = FENCE.match(line)
            if (
                marker
                and marker.group(1)[0] == fence[0]
                and len(marker.group(1)) >= fence[1]
                and not marker.group(2).strip()
            ):
                fence = None
            out.append(line)
            continue
        if in_comment:
            if "-->" in line:
                if not line.rstrip().endswith("-->"):
                    raise _refuse(number, "text after the end of an HTML comment")
                in_comment = False
            continue
        stripped = line.strip()
        if stripped.startswith("<!--"):
            # A comment that fills whole lines is dropped, with nothing after it.
            if "-->" in stripped:
                if not stripped.endswith("-->") or stripped.count("-->") != 1:
                    raise _refuse(number, "text after the end of an HTML comment")
            else:
                in_comment = True
            previous_text = False
            continue
        marker = FENCE.match(line)
        if marker:
            fence = (marker.group(1)[0], len(marker.group(1)))
            out.append(line)
            previous_text = False
            continue
        if HTML.search(line):
            raise _refuse(number, "raw HTML isn't allowed in the policy; use Markdown")
        if previous_text and SETEXT.match(line):
            raise _refuse(number, "underlined headings can't be moved down; use '#' headings")
        heading = HEADING.match(line)
        if heading:
            level = len(heading.group(1))
            if level == 1:
                if title_seen or out and any(o.strip() for o in out):
                    raise _refuse(number, "only one '# ' title, at the top, is allowed")
                title_seen = True
                previous_text = False
                continue
            if level == 6:
                raise _refuse(number, "headings must stay above level 6")
            line = "#" * (level + 1) + line.lstrip()[level:]
            previous_text = False
        else:
            previous_text = bool(stripped)
        if not title_seen and stripped:
            raise _refuse(number, "docs/privacy.md must start with its '# ' title")
        out.append(line)
    if in_comment:
        raise PolicyError("docs/privacy.md: an HTML comment is never closed")
    if fence is not None:
        raise PolicyError("docs/privacy.md: a code block is never closed")
    if not title_seen:
        raise PolicyError("docs/privacy.md must start with its '# ' title")
    # Dropped lines can leave blank runs; Markdown needs one blank line at most.
    body = re.sub(r"\n{3,}", "\n\n", "\n".join(out)).strip("\n")
    if not body:
        raise PolicyError("docs/privacy.md has no policy text")
    return body


def build_page(policy: str, template: str) -> str:
    template = template.lstrip("﻿").replace("\r\n", "\n")
    if template.count(PLACEHOLDER) != 1:
        raise PolicyError(f"the template must contain {PLACEHOLDER} exactly once")
    return template.replace(PLACEHOLDER, policy_body(policy)).rstrip("\n") + "\n"


def main(argv: list[str]) -> int:
    args = [a for a in argv if a != "--check"]
    if len(args) != 1:
        print("usage: build_privacy.py <path to docs/privacy.md> [--check]", file=sys.stderr)
        return 2
    try:
        page = build_page(
            Path(args[0]).read_text(encoding="utf-8-sig"),
            TEMPLATE.read_text(encoding="utf-8-sig"),
        )
    except (OSError, PolicyError) as error:
        print(f"build_privacy: {error}", file=sys.stderr)
        return 1
    if "--check" in argv:
        current = OUTPUT.read_text(encoding="utf-8-sig") if OUTPUT.exists() else ""
        if current.replace("\r\n", "\n") != page:
            print(
                f"build_privacy: {OUTPUT.relative_to(ROOT)} differs from {args[0]}; "
                "it's generated, so run scripts/build_privacy.py instead of editing it",
                file=sys.stderr,
            )
            return 1
        return 0
    OUTPUT.write_text(page, encoding="utf-8", newline="\n")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
