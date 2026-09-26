# Triage Labels

The skills speak in terms of five canonical triage roles. This file maps those roles to the `Status:` values used in this repo's local Markdown issues.

| Label in mattpocock/skills | Label in our tracker | Meaning                                  |
| -------------------------- | -------------------- | ---------------------------------------- |
| `needs-triage`             | `needs-triage`       | Maintainer needs to evaluate this issue  |
| `needs-info`               | `needs-info`         | Waiting on reporter for more information |
| `ready-for-agent`          | `ready-for-agent`    | Fully specified, ready for an AFK agent  |
| `ready-for-human`          | `ready-for-human`    | Requires human implementation            |
| `wontfix`                  | `wontfix`            | Will not be actioned                     |

When a skill mentions a role (for example, "apply the AFK-ready triage label"), set the issue's `Status:` line to the corresponding value from this table. The lifecycle states `claimed` and `resolved`, and the initial wayfinding state `open`, are separate from these triage roles.

Edit the right-hand column to match whatever vocabulary you actually use.
