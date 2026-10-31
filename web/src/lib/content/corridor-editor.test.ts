import { describe, expect, it } from "vitest";
import {
  extractCorridorEntries,
  updateCorridorEntryDef,
} from "./corridor-editor";

describe("corridor-editor (CS.4)", () => {
  const sampleLane = {
    manifest: { id: "tacsim.lane.ceres_phobos", version: 15 },
    entries: {
      ceres_port: {
        kind: "port",
        def: {
          venue: "Ceres Freeport",
          location: "ceres",
          area: "ceres_freeport",
          orgs: [{ id: "port_authority" }],
          vendors: [{ id: "assay_desk" }],
          site: { blocks: ["A", "B"] },
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
          nodes: { ceres: {}, luna: {} },
          pairs: { ceres_luna: {} },
        },
      },
    },
  };

  it("extracts port, route, ephemeris body (read-only), and relay network entries", () => {
    const entries = extractCorridorEntries(sampleLane);
    expect(entries).toHaveLength(4);

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
    }) as Record<string, Record<string, { def: Record<string, unknown> }>>;

    expect(updated.entries.ceres_to_stickney.def.duration_seconds).toBe(259200);
    expect(updated.entries.ceres_port.def.venue).toBe("Ceres Freeport");
  });
});
