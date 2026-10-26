# Named mind launch

The named launch is opt-in. The ordinary server leaves it absent. It composes the
existing Registry, scoped House commerce, Conduct service and governed hardware
owners; configuration does not create money, stock, certificates or licences.

## Prepare the real world

Create the organizations, scopes, characters, accounts, venue room, stock vaults,
agents, factory and output container through the normal world and owner commands.
Use the existing `tacsim.credit` instrument. Fund the actual accounts and grant
the intended scoped capabilities. The publisher needs `roll.mind.publish` in the
House's publication scope. The hardware parties retain their existing Origin,
Inventory, factory and account permissions. A staff flag does not replace those
owner checks.

The host supplies a UTF-8 JSON `tacsim::launch::LaunchDeployment` document, at most
64 KiB, with no unknown fields. Use the actual identifiers read from this world.
Its fields are:

| Field | Required actual identity |
| --- | --- |
| `format`, `mode` | `1`; initially `"initialize"`, later `"active"` |
| `registry_issuer` | Configured Registry title issuer |
| `registry` | Registry recipient's exact native `owner` and `account` |
| `venue` | Exact Market `name`, existing `room`, and native `recipient` |
| `houses.meridiani`, `houses.coprates`, `houses.tycho` | Each House's `organization`, `scope`, `treasury`, `publisher`, and owned `source_name` |
| `republic` | Existing Republic FactionId used for Coprates admission |
| `hardware` | Existing `importer`, `stock_scope`, `depot`, two `agents`, three `grades`, `supplier_account`, scoped `integrator`, `factory`, and `output_container` |

The hardware fields use the same types as `HardwareDeployment`. Prices, recipes,
stock quotas and catalogue roots come from the checked `PublishedHardware`
content, not caller-supplied values. House organizations and treasury accounts
must be distinct. Each publisher/source-name pair is unique. The counter must be
a real room. The host validates these joins before admitting the launch.

Set the explicit deployment path in the ordinary server configuration:

```toml
[tacsim.minds]
launch_path = "config/named-mind-launch.json"
```

Keep Tacsim enabled. Configure the ordinary proving worker/queue/daily budget
fields for the workload; each Registry or Conduct job still pays its actual
quota. No increased budget implies a completed measurement.
The normal `--check-config` preflight uses the same bounded document loader and
rejects an unreadable, empty, malformed or oversized launch document, including
launch configured while Tacsim is disabled. Actual world identity joins still
require booting the owner modules.

## Initialize, then activate

Boot with `mode = "initialize"`. Use the actual offered initialization actions
for Roll commerce, Core Origin, Core production and Core refit; wait for each
acknowledged persistence result. The launch imports no prebuilt owner rows.

After those acknowledgements, stop the host normally and change only `mode` to
`"active"`. Cold boot must pass every owner's native and typed-history audit.
The shared roster preserves port, moon-port, geology, Inventory and ordinary
market behavior. It captures the real Conduct carrier before constructing the
finite Tactics catalogue. `@conduct` is enabled through the actual owner service.

This configuration selects one-seat sealed House Pins. Existing two-seat products
remain separately explicit. IC region drivers and hull-coverage products are not
selected by this deployment. A host requiring coverage must separately compose
the actual withheld-departure service and, for qualified prices, the genuine
ConductReader-backed coverage configuration. An absent region producer cannot
charge a premium or supply a departure/loss proof.

## Publish the named sources

`@mind/launch` displays caller help without source output. From each configured
publisher, use its exact configured name:

```text
@mind/launch meridiani=Meridiani Field Series
@mind/launch coprates=Coprates Drill
@mind/launch safe_conduct=Safe Conduct
```

These commands import only the checked shipped draft into that publisher's
library. They require current scoped publication authorization. Existing drafts
are not overwritten. Ordinary authoring remains available afterward.

For each draft, run `@mind/check`, then `@mind/save <name>=<notes>`. Wait for the
actual save completion and inspect `@mind/info <name>` for the saved revision.
Use `@registry/rate <mind number>/<revision>` for the real off-loop certification
job. Follow its actual status and acknowledgement; pending or Unknown is not a
certificate and does not authorize another charge.

After certification, reserve the actual filing shell and use the offered House
filing action with that acknowledged shell, exact saved revision, current rating
reference and scoped House treasury. Use the filing result's actual owner licence
at the configured counter. The normal signed action vocabulary is:

| Verb | Public selector |
| --- | --- |
| `house-list-one-seat` | `{"owner_licence": <actual licence>}` |
| `house-quote-one-seat` | `{"listing": <acknowledged listing operation>}` |
| `house-fund-one-seat` | `{"quote": <acknowledged quote operation>}` |
| `house-grant-one-seat` | `{"operation": <acknowledged Fund operation>}` |
| `house-refund-one-seat` | `{"operation": <acknowledged Fund operation>}` |

These selector examples describe fields, not executable placeholder identities.
`+act <verb> with <JSON object>` resolves the current signed offer and repeats all
owner authority checks. Listing/quote results provide source-free terms; the
actual filing and purchase acknowledgements supply their historical identities.
Keep those results when retrying after uncertain delivery. Do not replace them
with reconstructed receipts or infer success from silence.

The filing fee is 100cr. Each initial one-seat sealed Pin is 400cr for 90 IC days
plus 3 days Grace, with 5% Registry and 5% venue deductions from gross proceeds.
Renewal is a separately accepted 400cr offer; no licence extends itself. Coprates
requires current Republic membership for a new purchase **and** a new renewal
term. Membership is checked again before prepared custody and consuming take.
Losing membership does not rewrite accepted terms and does not block refund or
historical cleanup. Meridiani and Safe Conduct have no Republic predicate.

`@mind/doctrine` retains the free Stickney teaching pages. For Safe Conduct,
`@conduct/run <filed> <profile> <escort|transit>` requests the actual trial from its
configured operator. Only a completed verified trial can satisfy a separately
configured qualified insurance product; the source import itself grants nothing.

## Acceptance evidence

The launch fixture prepares real Initialize acknowledgements, Active cold boot,
all three private imports and actual Save/Registry/House filing/listing/purchase
flows. It checks Coprates prepare/stage/take membership loss, genuine renewal,
and cold retained terms. It does not substitute synthetic certificates or balances
for the owner path. These tests require the shared runner; source preparation is
not a runtime pass. The long three-source Registry story is ignored in ordinary
test prefixes and must run through its exact named acceptance gate. Transport
loss and live region journeys remain separate acceptance obligations.
