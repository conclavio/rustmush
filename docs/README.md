# Documentation

Start with the [design overview](design.md) for the ideas behind rustmush and
the current TacSim game. Use the [crate map](../crates/README.md) to find the
code that implements them.

| I want to… | Start here |
| --- | --- |
| Understand the game and its architecture | [Design overview](design.md) → [full design index](design/README.md) |
| Find a subsystem in the code | [Workspace crate map](../crates/README.md) |
| Build, run, or test locally | [Development guide](development.md) and [example configuration](../mushd.toml.example) |
| Learn the commands | [Getting started](help/getting-started.md), [simulator](help/simulator.md), and [all help topics](help/) |
| Deploy or maintain a server | [Operations](ops.md) and [deployment guide](../deploy/README.md) |
| Run bots or evaluate play | [Bot overview](bot.md) and [detailed bot guides](bot/) |

## Design and implementation records

The [design collection](design/README.md) contains the project's architecture,
accepted decisions, implementation plans, cut notes, and reviews. The
[guided overview](design.md) groups those documents by subject and points to
useful starting places. Read a document's status and implementation record
together: a proposal is not a claim that all its behavior is running today.

## Specialist references

| Reference | Use it for |
| --- | --- |
| [Interface contracts](interface/) | Shared presentation, commands, and player-facing surfaces. |
| [Governed hardware composition](hardware-composition.md) | Composing the optional hardware owners into an actual deployment. |
| [Named mind launch](named-mind-launch.md) | Provisioning the optional mind-commerce launch with real world identities. |
| [Minds fact inventory](minds-facts.md) | Fact namespaces, visibility, and authorization for mind-related contracts. |
| [Testing at a higher world rate](testing-rate.md) | A dated experiment on IC clock rates and test duration. |

## Plans and history

- [Original master plan](PLAN.md): the project's initial architecture and
  milestones. Read its treaty rulings alongside the later
  [superseding decisions](design/README.md#superseded-decisions).
- [Archived ideas](archive/README.md): early brainstorming retained for context.
- [Review records](reviews/): dated assessments, probes, and reproduction
  material. These describe the revisions they reviewed.

[Back to the project](../README.md)
