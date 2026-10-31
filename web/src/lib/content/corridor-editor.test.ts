import { describe, expect, it } from "vitest";
import {
  addPortOrg,
  addPortVendor,
  extractCorridorEntries,
  extractPortOrgs,
  extractPortSite,
  extractPortVendors,
  extractRelayNodes,
  extractRelayPairs,
  extractRelayRecipe,
  removePortOrg,
  removePortVendor,
  updateCorridorEntryDef,
  updatePortOrg,
  updatePortSiteBlock,
  updatePortSiteMeta,
  updatePortVendor,
  updateRelayNode,
  updateRelayPair,
  updateRelayRecipe,
} from "./corridor-editor";

describe("corridor-editor (CS.4 & CS.5b)", () => {
  const sampleLane = {
    manifest: { id: "tacsim.lane.ceres_phobos", version: 15 },
    entries: {
      ceres_port: {
        kind: "port",
        def: {
          venue: "Ceres Freeport",
          location: "ceres",
          area: "ceres_freeport",
          orgs: [
            {
              id: "org.ceres.authority",
              name: "Ceres Port Authority",
              capital_major: 250000,
              faction: "civic",
              raises: false,
            },
          ],
          vendors: [
            {
              org: "org.ceres.authority",
              storefront: "Room 08 — Chandlery",
              agent: "Chandler's Clerk",
              signatory: "npc.oskar_brenn",
              stock: [{ good: "metals", qty: 20, grade: "standard", issue: 100 }],
              asks: [{ good: "metals", price_major: 420, boards: ["public"] }],
              bids: [{ good: "volatiles", price_major: 180, intake: 50, boards: ["public"] }],
              mill: [
                {
                  ore: "silicate_ore",
                  into: "metals",
                  ratio: { ore: 4, product: 1 },
                  batch_t: 4,
                  per_ic_day_t: 12,
                },
              ],
            },
          ],
          site: {
            name: "Ceres Northern Bench",
            issuer: "org.ceres.authority",
            zone: "northern_bench",
            tenure: { default: "open" },
            blocks: {
              A: {
                tenure: "leased",
                holder: "org.ceres.authority",
                prospectivity: "proven",
                worked_permille: 250,
              },
            },
          },
        },
      },
      ceres_to_stickney: {
        kind: "route",
        def: {
          origin: "ceres",
          destination: "stickney",
          duration_seconds: 172800,
        },
      },
      ceres_body: {
        kind: "body",
        def: {
          elements: [{ epoch: 2460000.5 }],
        },
      },
      network: {
        kind: "relay_network",
        def: {
          nodes: {
            ceres: {
              label: "Ceres Freeport Relay",
              place: { pack: "tacsim.lane.ceres_phobos", entry: "ceres_freeport" },
              coverage: [{ pack: "tacsim.lane.ceres_phobos", entry: "ceres" }],
            },
            luna: {
              label: "Aitken Relay",
              place: { pack: "tacsim.lane.ceres_phobos", entry: "aitken_highport" },
              coverage: [],
            },
          },
          pairs: {
            ceres_luna: {
              capacity_units_per_ic_second: 12,
              class_policy: "public_tariff",
              halves: [
                { id: "ceres_luna.a", node: "ceres" },
                { id: "ceres_luna.b", node: "luna" },
              ],
            },
          },
        },
      },
      private_pair: {
        kind: "relay_manufacture",
        def: {
          label: "Entangled Relay Pair",
          work_units: 3600,
          requires: "facility.entanglement_bench",
          inputs: [{ commodity: "electronics", units: 4 }],
          outputs: [{ commodity: "relay_pair", units: 1 }],
        },
      },
    },
  };

  it("extracts port, route, ephemeris body (read-only), and relay network entries", () => {
    const entries = extractCorridorEntries(sampleLane);
    expect(entries).toHaveLength(5);

    const port = entries.find((e) => e.id === "ceres_port");
    expect(port?.kind).toBe("port");
    expect(port?.label).toBe("Ceres Freeport");
    expect(port?.subtitle).toContain("mining site");
    expect(port?.readOnly).toBe(false);

    const route = entries.find((e) => e.id === "ceres_to_stickney");
    expect(route?.kind).toBe("route");
    expect(route?.label).toBe("ceres → stickney");
    expect(route?.subtitle).toContain("2.0 IC days");

    const body = entries.find((e) => e.id === "ceres_body");
    expect(body?.kind).toBe("body");
    expect(body?.readOnly).toBe(true);

    const relay = entries.find((e) => e.id === "network");
    expect(relay?.kind).toBe("relay");
    expect(relay?.label).toContain("2 nodes, 1 pairs");
  });

  it("updates an entry def while preserving other corridor entries", () => {
    const updated = updateCorridorEntryDef(sampleLane, "ceres_to_stickney", {
      origin: "ceres",
      destination: "stickney",
      duration_seconds: 259200,
    });
    const entries = extractCorridorEntries(updated);
    const route = entries.find((e) => e.id === "ceres_to_stickney");
    const port = entries.find((e) => e.id === "ceres_port");

    expect(route?.def.duration_seconds).toBe(259200);
    expect(port?.def.venue).toBe("Ceres Freeport");
  });

  it("extracts and updates port orgs, vendors, and mining site blocks (CS.5b)", () => {
    const entries = extractCorridorEntries(sampleLane);
    const portDef = entries.find((e) => e.id === "ceres_port")!.def;

    const orgs = extractPortOrgs(portDef);
    expect(orgs).toHaveLength(1);
    expect(orgs[0]?.id).toBe("org.ceres.authority");

    const withAddedOrg = addPortOrg(portDef);
    expect(extractPortOrgs(withAddedOrg)).toHaveLength(2);

    const secondOrg = extractPortOrgs(withAddedOrg)[1]!;
    const withUpdatedOrg = updatePortOrg(withAddedOrg, 1, {
      ...secondOrg,
      id: "org.ceres.syndicate",
      capital_major: 95000,
    });
    expect(extractPortOrgs(withUpdatedOrg)[1]?.id).toBe("org.ceres.syndicate");
    expect(extractPortOrgs(removePortOrg(withUpdatedOrg, 1))).toHaveLength(1);

    const vendors = extractPortVendors(portDef);
    expect(vendors).toHaveLength(1);
    const firstVendor = vendors[0]!;
    expect(firstVendor.signatory).toBe("npc.oskar_brenn");
    expect(firstVendor.mill[0]?.ratio_ore).toBe(4);

    const withVendorAdded = addPortVendor(portDef, "ceres_port");
    expect(extractPortVendors(withVendorAdded)).toHaveLength(2);
    const withVendorUpdated = updatePortVendor(withVendorAdded, 0, {
      ...firstVendor,
      asks: [{ good: "metals", price_major: 455, boards: ["public"] }],
    });
    expect(extractPortVendors(withVendorUpdated)[0]?.asks[0]?.price_major).toBe(455);
    expect(extractPortVendors(removePortVendor(withVendorUpdated, 1))).toHaveLength(1);

    const site = extractPortSite(portDef);
    expect(site).not.toBeNull();
    expect(site?.blocks).toHaveLength(16);
    expect(site?.blocks.find((b) => b.id === "A")?.worked_permille).toBe(250);

    const withUpdatedBlock = updatePortSiteBlock(portDef, "B", {
      tenure: "leased",
      holder: "org.ceres.authority",
      prospectivity: 1,
      worked_permille: 120,
    });
    expect(extractPortSite(withUpdatedBlock)?.blocks.find((b) => b.id === "B")?.holder).toBe(
      "org.ceres.authority",
    );

    const withUpdatedSiteMeta = updatePortSiteMeta(withUpdatedBlock, {
      name: "Ceres Deep Bench",
    });
    expect(extractPortSite(withUpdatedSiteMeta)?.name).toBe("Ceres Deep Bench");
  });

  it("extracts and updates relay nodes, entangled pairs, and manufacture recipes (CS.5b)", () => {
    const entries = extractCorridorEntries(sampleLane);
    const netDef = entries.find((e) => e.id === "network")!.def;
    const mfgDef = entries.find((e) => e.id === "private_pair")!.def;

    const nodes = extractRelayNodes(netDef);
    expect(nodes).toHaveLength(2);
    const updatedNet = updateRelayNode(netDef, "ceres", {
      label: "Ceres Central Trunk",
    });
    expect(extractRelayNodes(updatedNet)[0]?.label).toBe("Ceres Central Trunk");

    const pairs = extractRelayPairs(netDef);
    expect(pairs).toHaveLength(1);
    expect(pairs[0]?.capacityUnitsPerIcSecond).toBe(12);
    const updatedPairs = updateRelayPair(netDef, "ceres_luna", {
      capacityUnitsPerIcSecond: 24,
    });
    expect(extractRelayPairs(updatedPairs)[0]?.capacityUnitsPerIcSecond).toBe(24);

    const recipe = extractRelayRecipe(mfgDef);
    expect(recipe).not.toBeNull();
    expect(recipe?.workUnits).toBe(3600);
    const updatedRecipe = updateRelayRecipe(mfgDef, {
      workUnits: 4800,
    });
    expect(extractRelayRecipe(updatedRecipe)?.workUnits).toBe(4800);
  });
});
