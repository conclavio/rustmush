# K8 compatibility rules for modules and scripts

Status: K8.5 contract, version 1. These are source and ownership rules for the in-tree Rust modules
and MUSH scripts. They are not a stable dynamic-plugin ABI promise.

## 1. What is stable

Persisted and cross-process data is compatible only through its declared versioned form. Canonical
IDs, exact `DefinitionRef`s, state-family owner/family/key tuples, authoritative bundle versions,
action ingress, and projected DTO schema versions retain the strict decode and migration rules in
doc 09. A Rust type name, trait layout, registration call, or module implementation detail is not a
wire contract. Readers may retain an explicitly documented older version; writers emit only the
current canonical form.

`GameModule` remains the composition seam. A module name is its canonical mechanical owner, and
mechanics registration is accepted only while `Engine::compose_modules` invokes that module. A
third-party stable ABI is deliberately not promised yet: module crates are rebuilt with the server,
and additive source changes may still be required while the first two setting rulesets and tactics
finish migrating production mechanics.

## 2. Every command declares its lane

A composed module must call `Registrar::classify_commands` before it registers a command, alias
stage, or command fallback. The selected value is copied into each registry entry; a module may
change the selection between command groups. Ordinary commands are inspectable with
`Registry::command_registrations`/`compatibility_of`, while alias and fallback declarations have
their corresponding registry iterators. Omitting the declaration fails module composition before a
world or persisted record is loaded.

The four classes describe the maximum persistence authority of that ingress:

- `ReadOnly` is inspection or presentation. Durable mutation is not part of its contract.
- `LegacyPostImage` may emit only a bounded, strictly decoded and validated whole-document
  post-image under a deterministic owner key. Its design must name the future typed family, key
  codec, migration slot, and authoritative replacement. It may not mutate a K-owned typed family,
  reservation, contact, lease, action window, resolution, or production record.
- `Authoritative` is only an adapter to the shared offer/coordinator path. The handler does not
  implement the mechanic, draw ambient randomness, construct a second authorization decision, or
  persist the outcome on the legacy lane.
- `EconomyCarveOut` is the narrow legacy-post-image exception for existing E-owned domain records.
  It inherits every `LegacyPostImage` constraint and expires record-by-record when that owner moves
  onto a typed authoritative bundle.

The metadata is an audit boundary for trusted compiled code, not a runtime sandbox. A class that
grants more persistence authority than before is a design change and must update its migration
record and contract tests. CI may reject an unclassified registration or a class outside the
checked-in owner policy; source review and domain tests still verify that the implementation obeys
the declaration.

Bare `Registry::register` and its host-only alias/fallback helpers remain source-compatible for core
assembly and tests. They are conservatively recorded as `LegacyPostImage`. Shippable game modules
use `Registrar`, where the declaration is mandatory.

## 3. One writer while records migrate

A record has one mutation authority at a time. The first installed typed-family record is the
authority fence: legacy setters for that record must refuse before mutation, and presentation must
overlay/read the acknowledged typed post-image. A module may keep unrelated fields in a legacy
whole-document record only when the ownership split is explicit and round-trip tests prove neither
writer clobbers the other. Deleting a legacy alias means deleting its mutation path, not merely
hiding its help entry.

An authoritative adapter submits the authenticated actor, exact subject, bounded parameters, and
request identity to the ordinary coordinator. It never accepts client-supplied guards, revisions,
definitions, RNG results, or internal target truth. Success, external effects, and semantic receipts
remain acknowledgement-gated. Recovery uses the durable token plus `action/status` and current
keyframes; no interface invents a second replay protocol.

## 4. Script boundary

MUSH scripts remain a compatibility-language feature, not mechanical providers. Their `$`-command
fallback is explicitly registered as `LegacyPostImage`. Scripts may use the bounded, allowlisted
object and narrative operations supplied by `mush-script`; they cannot register definitions,
providers, validators, state families, schedules, decision sources, bridges, or durable outbox
effects, and they never receive offer secrets, guard witnesses, resolution keys, or mutable typed
state. A future script primitive that invokes a mechanic must select a projected public action and
re-enter the same coordinator as line, TUI/web, NPC, and LLM callers.

Script persistence remains ordinary object/document compatibility state. It must not become a
shadow store for authoritative resources or outcomes. Limits on source, instructions, timers,
recursion, emitted events, and payloads are part of compatibility and may be tightened to preserve
safety; loosening one requires an explicit resource review.

## 5. Change and test policy

Every new mutating mechanic starts as a provider plus typed family (or extends an existing owner),
not as a new legacy handler. Every compatibility-record exception must have strict decode,
canonical re-encoding, bounded size/counts, a deterministic key, boot validation, a named migration
slot, and a test proving failure emits no success. Backend-specific behavior requires the shared
store conformance assertion.

Provider and validator traits may gain only backward-compatible default methods during this K8
contract. Removing a method, changing guard/RNG/write semantics, reinterpreting an existing version,
or broadening a command class requires a new contract version and migration review. Definition,
bundle, ingress, receipt, and view golden fixtures pin wire behavior; the Gate-5 matrix pins
acknowledgement, restart, and replay behavior.

K8.5 replaces source scanning and a growing verb allowlist with these registrar declarations. K8.6
turns the declarations, direct-randomness policy, truth-serialization policy, wire fixtures, contract
tests, and Redis fault suites into named CI jobs.
