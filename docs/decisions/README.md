# Decisions

Each important decision gets a short numbered file: `0001-tech-stack.md`, `0002-licence.md` and so on.

Write one when you add a dependency, change a public API or the database schema, or make any choice in CONTRIBUTING.md's "Decisions that only maintainers make" list. Open the PR with the `needs-decision` label.

## Index

| Number | Decision | Status |
|---|---|---|
| [0000](0000-app-name-and-domain-acquisition.md) | App name and domain name acquisition |
| [0001](0001-tech-stack.md) | Shared Rust core with native UIs, bound through UniFFI | proposed |

## Template

```
# NNNN: Title

Status: proposed | accepted | replaced by NNNN
Date: YYYY-MM-DD

## Context
What problem are we solving, and what limits us?

## Decision
What we're doing.

## Why
The reasons, and the options we didn't pick.

## Consequences
What gets easier, what gets harder.
```

Accepted decisions can't be quietly worked around. To change one, open a new decision that replaces it.
