import { describe, expect, it } from "vitest";
import {
  DocumentHistory,
  historicalSnapshotPath,
  newPackRegistrationSnippet,
  parseCatalog,
} from "./store";

describe("Content Studio store & export helpers (CS.0a)", () => {
  it("parses a catalog snapshot with ports, stations, npcs, and packs", () => {
    const catalog = parseCatalog({
      ports: [
        {
          id: "ceres_port",
          venue: "ceres_exchange",
          location: "ceres_freeport",
          area: "neighborhood.ceres_freeport",
          rooms: { arrivals: "room.arrivals" },
          orgs: ["freeport_harbourmaster"],
          vendors: ["ceres_exchange"],
        },
      ],
      stations: [],
      npcs: [],
      lore_pools: [],
      goods: [],
      interfaces: ["hire", "quest"],
      command_refs: ["+crew", "+quest"],
      space_clusters: [
        {
          id: "pallas_inclined_belt",
          title: "Pallas Inclined Belt",
          jurisdiction: "FrontierUnpoliced",
          rock_count: 6,
        },
      ],
      outer_bodies: ["saturn", "neptune"],
      space_hulls: [
        {
          id: "tacsim.unit.dray",
          short_id: "dray",
          label: "Dray-class Bulk Freighter",
          refit_family: "dray_hull",
          pixel_model: "freighter",
          dry_mass_t: 400,
          delta_v_mps: 36000,
        },
      ],
      space_refit_packages: [
        {
          id: "refit.dray.prospector",
          label: "Dray-P Asteroid Mining & Drone Bay Package",
          refit_family: "dray_hull",
          target_hull_id: "tacsim.unit.dray_p",
          cost_cr: 18500,
          work_hours: 12,
        },
      ],
      packs: [],
    });
    expect(catalog.ports).toHaveLength(1);
    expect(catalog.ports[0]?.id).toBe("ceres_port");
    expect(catalog.interfaces).toEqual(["hire", "quest"]);
    expect(catalog.space_clusters).toHaveLength(1);
    expect(catalog.space_hulls[0]?.short_id).toBe("dray");
    expect(catalog.space_refit_packages[0]?.id).toBe("refit.dray.prospector");
  });

  it("maintains bounded undo/redo history for studio documents", () => {
    const history = new DocumentHistory<{ version: number }>(5);
    let doc = { version: 1 };
    history.record(doc);
    doc = { version: 2 };
    expect(history.canUndo).toBe(true);
    const undone = history.undo(doc);
    expect(undone).toEqual({ version: 1 });
    expect(history.canRedo).toBe(true);
    const redone = history.redo(undone!);
    expect(redone).toEqual({ version: 2 });
  });

  it("formats historical snapshot paths and new-pack Rust registration snippets (D-34-5, D-34-6)", () => {
    expect(historicalSnapshotPath("passengers/stickney.json", 1)).toBe(
      "passengers/stickney_v1.json",
    );
    expect(historicalSnapshotPath("relays/luna_ceres.json", 1)).toBe(
      "relays/luna_ceres-v1.json",
    );
    const snippet = newPackRegistrationSnippet("tacsim.quests.ceres_arc", "quests/ceres_arc.json");
    expect(snippet).toContain('pub const TACSIM_QUESTS_CERES_ARC_PACK: &str = "tacsim.quests.ceres_arc";');
    expect(snippet).toContain("TACSIM_WRITE_CONTENT=1 cargo test -p tacsim");
  });
});
