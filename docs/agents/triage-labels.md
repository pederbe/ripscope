# Triage labels

Use these label strings when a skill names a triage role.

| Role | Tracker label | Meaning |
| --- | --- | --- |
| `needs-triage` | `needs-triage` | Awaiting maintainer assessment |
| `needs-info` | `needs-info` | Waiting for more information |
| `ready-for-agent` | `ready-for-agent` | Fully specified; blockers, priorities and execution authority still apply |
| `ready-for-human` | `ready-for-human` | Requires the maintainer's implementation |
| `wontfix` | `wontfix` | Will not be worked on |

The existing `bug` and `enhancement` labels describe the issue category, separately
from its triage state. Preserve unrelated labels.

`ready-for-agent` does not select a ticket for implementation. Before starting
work, state who benefits and how often, and follow the owner's scope and priorities.

This file records the vocabulary. It does not create labels or change existing
issues. When an authorised workflow needs a missing label, create that label
without changing the definitions of existing labels.
