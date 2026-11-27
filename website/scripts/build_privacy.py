#!/usr/bin/env python3
"""Write content/privacy.md from the app's privacy policy, docs/privacy.md on main (#475).

docs/privacy.md is the only copy of the policy. The site keeps its own part
("This website") in scripts/privacy_template.md, and this script puts the
policy under "The app": its title and HTML comments are dropped and every
heading moves down one level. The deploy workflows run it with main's file
before `zola build`, so the live page can't drift from the policy.

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
COMMENT = re.compile(r"<!--.*?-->\n*", re.DOTALL)
FENCE = re.compile(r"^(`{3,}|~{3,})")
HEADING = re.compile(r"^(#{1,5}) ")


class PolicyError(ValueError):
    """The policy file isn't in the shape the page needs."""


def policy_body(policy: str) -> str:
    """The policy without its title and comments, with headings one level down."""
    text = COMMENT.sub("", policy.replace("\r\n", "\n"))
    lines = text.strip("\n").split("\n")
    if not lines or not lines[0].startswith("# "):
        raise PolicyError("docs/privacy.md must start with its '# ' title")
    out = []
    fence = None
    for line in lines[1:]:
        marker = FENCE.match(line)
        if marker:
            if fence is None:
                fence = marker.group(1)[0]
            elif line.startswith(fence * 3):
                fence = None
        elif fence is None:
            if line.startswith("# "):
                raise PolicyError("docs/privacy.md must have only one '# ' title")
            if line.startswith("######"):
                raise PolicyError("docs/privacy.md headings must stay above level 6")
            line = HEADING.sub(lambda m: "#" + m.group(1) + " ", line)
        out.append(line)
    body = "\n".join(out).strip("\n")
    if not body:
        raise PolicyError("docs/privacy.md has no policy text")
    return body


def build_page(policy: str, template: str) -> str:
    if template.count(PLACEHOLDER) != 1:
        raise PolicyError(f"the template must contain {PLACEHOLDER} exactly once")
    return template.replace("\r\n", "\n").replace(PLACEHOLDER, policy_body(policy)).rstrip("\n") + "\n"


def main(argv: list[str]) -> int:
    args = [a for a in argv if a != "--check"]
    if len(args) != 1:
        print("usage: build_privacy.py <path to docs/privacy.md> [--check]", file=sys.stderr)
        return 2
    try:
        page = build_page(
            Path(args[0]).read_text(encoding="utf-8"),
            TEMPLATE.read_text(encoding="utf-8"),
        )
    except (OSError, PolicyError) as error:
        print(f"build_privacy: {error}", file=sys.stderr)
        return 1
    if "--check" in argv:
        current = OUTPUT.read_text(encoding="utf-8") if OUTPUT.exists() else ""
        if current.replace("\r\n", "\n") != page:
            print(
                f"build_privacy: {OUTPUT.relative_to(ROOT)} differs from {args[0]}; "
                "run scripts/build_privacy.py to rebuild it",
                file=sys.stderr,
            )
            return 1
        return 0
    OUTPUT.write_text(page, encoding="utf-8", newline="\n")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
