# Communication relay availability and privacy

Status: accepted on dev1 on 2026-10-24; closes retrospective RR-1 and RR-4.
This is follow-up gate 2 of [the R1–R10 retrospective](15c-comms-relay-retrospective.md).
It adds no new gameplay producer and leaves per-link timing, payload/rate policy,
market streams and generic mobile installation work to later gates.

## Contract and implementation

The physical pending limit remains 128. Admission counts both active envelopes and
source-owned mail/channel intents whose stable key has not yet been accepted.
A source reservation remains charged until its terminal receipt is consumed. Mail
retains the original sender and organization charge across outbound delivery and the
returning receipt; changing direction or envelope key cannot refill its allowance.
The active envelope and matching source reservation count once across handoff and
restart. Terminal envelopes whose source has withdrawn no longer count. New data
has a 32-copy actor allowance; explicitly
represented organization traffic additionally shares 32 copies across officials.
Mail and channels each have 48-copy ceilings and jointly may reserve 64 copies.
All data together is capped at 112; the remaining 16 slots are available to the
trusted acknowledgement/return-receipt paths. Existing retention-table and link
limits still apply. This prevents one issuer or one external producer from filling
the global queue; it does not guarantee latency under arbitrary multi-actor load.
The limits are conservative initial constants, not a new mutable policy product.

The owner-neutral `RelayRepresentation` and `TransportRefusal` types live in
mush-core. Mail and comsys ask their composition adapter for admission before
creating durable source state. The adapter returns typed unavailable, authority,
principal, producer, control-reserve or global refusals; existing text interfaces
render these without exposing other principals' counts. Direct tactical admission
uses the same accounting. Existing accepted work is grandfathered through upgrades;
it is not discarded to meet a new quota. There is one scheduler and one source
handshake, not a second reservation database.

Private policy stores independent discover/use grants by object or organization,
an explicit public-carriage set, and actor-selected representation preferences in
the existing durable relay service record. Missing policy in older records defaults
to an empty table: private pairs become closed. Exchange charter pairs remain public.
Positive use authority is required at both endpoints. Discovery gates the board,
direct pair commands and private receipt stamps; undiscovered and nonexistent IDs
produce identical replies. Service class, payment and inspect/manage/publish rights
do not imply use. Opening public carriage requires manage plus publish; policy
changes are audited and durable before their acknowledgement.

Organization and scope IDs retain their existing zero-based semantics; `0` is a
valid identity, not an absent-principal sentinel. An actor selects an organization
through `+relay/represent org:id`, requiring live
`relay.speak` authority at its root scope. A child-only grant cannot assert root
identity. The source freezes actor, organization and authorizing scope separately
from the payer. Selection changes affect new traffic only. Orders, reports, mail and
IC channel lines carry the snapshot through handoff, reboot, routing, subscription
lookup and service receipts. Acknowledgements are generated personally. Existing
legally titled non-actor ownership remains supported while its organization is
active; membership, custody and paying accounts never confer speaking authority.

Planning and each next relay transmission revalidate speaking and use authority.
Revoked authority falls back to conventional routing when physical endpoints are
known. A leg already in flight retains its timing. A revoked selected identity
refuses new source admission until changed or authorized again. Recipient identity
remains its own character or titled owner; selecting an outgoing organization does
not create a general organization inbox or a receiving delegation API.

Current commands and migration guidance are in [relay help](../help/relay.md).

## Acceptance gate

Production regression coverage exercises source atomicity, unresolved-mail isolation,
producer headroom, the 112/16 split and unchanged 128 hard cap, shared organization
quotas, identity switching, scoped agency, queued revocation, private discovery/use,
public carriage, and restart persistence. Existing R9a/R9b fixtures explicitly grant
private access; full-link saturation uses independent issuers instead of violating
the new per-actor allowance. The legacy-ticket test still fills the entire link and
checks migration without readmission.

The real-server gate uses `scripts/relay-availability-live.mjs` on conclav.io dev1.
`scripts/relay-availability-world-fixture.py` saves the current full Redis snapshot
and config, loads the genuine six-payment R9b checkpoint through a private portless
Redis reader, and later restores the saved baseline through Redis RESTORE. Only dev1
database 0 is replaced; its append-only log stays enabled and consistent. It verifies
the checkpoint digest and dev2 PID and never fabricates
payments, industrial output, clock anchors or installations. Test commands sail the
existing fitted hull to distinct coverage, exercise private policy and explicit
agency across SIGKILL, verify arrived inbox/receipt knowledge, and saturate one
ordinary sender while unrelated orders, acknowledgements and channel lines proceed.
Private snapshots and credentials remain root-only on the VPS.

Linux validation passed 2,490 tests with eight pre-existing ignored tests: 57 relay
integration, 205 mail/comsys/propagation, 31 boot-parity/IC integration, 1,480 TacSim
library and 717 isolated bot tests. The relay suite includes eight new regressions.
Strict all-targets Clippy and full formatting passed. The bot suite ran after its
fixture repair; the other suites reran after the final zero-ID correction.

The acceptance-repair push's normal CI finished with all required jobs passing except
the isolated bot arena fixture's mismatched content/runtime flag. This unit repairs
that fixture using the same `cfg.enabled` for both registrations, preserving the
production Fleet/Postmaster guard. The focused arena target (19 tests, included in
the full 717) and full bot suite pass. CodeQL passed. The next push's CI remains a
separate result.

The final real-server gate passed 150 checked commands and four SIGKILL/restart
boundaries on binary `f4e40f4af9a869c745dd669398c8f49e0a1a0e589357a2e330ec5a6a703000aa`.
It proved both-endpoint private use, queued revocation, explicit frozen agency,
zero-based organization/scope persistence, arrived private receipt filtering, and
actual unrelated order/acknowledgement/channel arrival during one sender's 32-copy
unresolved-mail saturation. The saved baseline was restored and verified healthy;
dev2 remained unchanged. All 1,492 build/help/harness inputs match the local source.

[The acceptance report](evidence/relay-availability-privacy-2026-10-24/README.md)
contains exact identities, logs, prior failed harness attempts and the earlier
successful nonzero-identity pass. This closes gate 2; policy/bandwidth and additional
consumer work remain open.
