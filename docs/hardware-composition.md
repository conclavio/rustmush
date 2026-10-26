# Explicit governed hardware host composition

`hardware::GovernedHardware` is an opt-in executable composition entry. Ordinary
`TacsimRuleset` and server rosters remain unchanged. A launch host must supply
existing, persisted deployment identities and pass the resulting owners into its
actual Engine/server roster. This package does not select or create a live site.

## Deployment

Create the actual importer Org and separate stock/factory scopes; a depot and two
agent stock vaults; scoped supplier/integrator accounts in `cr`; a factory with
`core_integration` and `core_refit` capabilities and the correct scoped Org title;
an output container; and two real named stock agents. Configure actual scope grants
for `industry.core_die.issue`, `vault.deposit`, `vault.withdraw`,
`industry.core.integrate`, and `industry.core.refit` as applicable. Use normal
Org/Economy/Industry commands and durable acknowledgement, not fabricated owner
rows. Fund buyer accounts through the real Economy path.

`HardwareDeployment` carries only those identities plus three distinct inventory
grade identities and the actual filing issuer. All prices, quotas, recipes,
work durations, delivery terms and finite fitted roots come from shipped content.
The same real `InitialFilingPolicy` must configure Economy and the actual Roll
filing action. No stub participant or caller-provided authority is accepted.

## Host entry and order

1. Create an enabled `TacsimRuleset` with the host's loaded content. Prepare
   `GovernedHardware::prepare(&layer, mode, deployment, &filing_policy)`.
2. Call `GovernedHardware::compose` with that layer, the existing Economy,
   Inventory and Industry modules and one shared `InventoryHandleCell`.
   Preserve the host's title-issuer list. Inventory must retain its actual quantity
   delegates, including Industry and Tacsim.
3. Prepare `MindCommerce` against the returned configured Tacsim layer, then apply
   its existing Tacsim/Economy/Roll configuration. This order gives Registry,
   Tacsim and the finite Tactics catalogue exactly the same I/II/III roots.
4. Put the returned Tacsim, Tactics, Inventory, Industry and Economy owners in the
   host roster alongside its actual Org, Sim and configured Roll owners. Existing
   sensor and IC options can wrap the returned Tactics module through their normal
   APIs. Do not also add a second owner instance.
5. In Initialize mode, durably acknowledge the actual signed commerce initializer
   if needed, then initialize-core-die-stock, initialize-core-factory and
   initialize-core-refit. Persist these empty owner deployment records. Rebuild
   fresh configuration objects with the same identities and restart in Active mode.
   Reusing already-registered OnceLock-backed module configurations is not a reboot.

The normal Tacsim boot path publishes the exact shipped hardware pack. Merely
constructing `PublishedHardware` or `GovernedHardware` does not publish it, mint
stock, grant permission or create money. Recipe callbacks require the actual
current published pack and matching owner state. All standard defaults remain
absent. Operator departure after acceptance does not become buyer impersonation;
existing owner rules decide whether accepted work can drain.

The player surface is the existing signed self-action route documented in
`docs/help/hardware.md`. Its generic schedule driver is the production work clock;
a caller cannot submit elapsed hours or an output image. Named-site server wiring
and deployment are still the launch host's explicit orchestration step.

## Prepared evidence

Three actual composed-Engine tests live under `hardware::flow::`. The full Class I
story creates real Org/accounts/vaults/facility through commands, acknowledges
Initialize and restarts Active, issues a real die, resolves supplier and buyer
signed offers, holds money and input, advances actual discovered schedules through
a cold restart, manufactures and delivers the core, then pays for and installs a
real finite-family refit. It checks the 90/10 payment split, fitting fee, actual
Core exchange receipt, preserved hull state and cold owner reconstruction.
Additional cases exercise real Class III refusal without entitlement and facility
changes at stage/consuming take. These are prepared tests, not a claim that a live
site or Linux test run has succeeded. No synthetic certificate, receipt, readiness,
stock lot, worker result or schedule postimage is used.
