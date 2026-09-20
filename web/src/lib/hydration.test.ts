// The hydration seam (interface plan I4): a hello — and a full rehydrate
// after a resume gap — issues the command table once, subscribes every
// `.vitals` feed and the scenes feed, and keeps the older requests, each
// gated on the manifest.

import { describe, expect, it } from "vitest";

import { hydrationQueries } from "./game.svelte";
import { SessionStore } from "./stores/session.svelte";

function session(panels: string[], features: string[] = []): SessionStore {
  const s = new SessionStore();
  s.applyHello({ server: "test", features, panels });
  return s;
}

describe("hydrationQueries", () => {
  it("subscribes vitals and scenes and asks for the command table once", () => {
    const qs = hydrationQueries(
      session(["comsys", "mail", "wod.sheet", "wod.vitals", "sr.vitals", "scenes", "wod.roll"]),
    );
    const kinds = qs.map((q) => q.q.t);
    expect(kinds.filter((t) => t === "commands")).toHaveLength(1);
    expect(kinds.filter((t) => t === "who")).toHaveLength(1);
    const feeds = qs
      .map((q) => q.q)
      .flatMap((q) => (q.t === "panel_subscribe" ? [q.d.feed] : []));
    expect(feeds).toEqual(["comsys", "wod.vitals", "sr.vitals", "scenes"]);
    expect(kinds).toContain("mail_list");
    expect(kinds).toContain("sheet");
    expect(qs.find((q) => q.pending.kind === "vitals")?.pending).toEqual({
      kind: "vitals",
      feed: "wod.vitals",
    });
  });

  it("asks only for what the manifest carries", () => {
    const qs = hydrationQueries(session([]));
    expect(qs.map((q) => q.q.t)).toEqual(["who", "commands"]);
    const byFeature = hydrationQueries(session([], ["mail", "channels"]));
    expect(byFeature.map((q) => q.q.t)).toEqual([
      "who",
      "panel_subscribe",
      "mail_list",
      "commands",
    ]);
  });

  it("subscribes the unit feed and the contacts list once each, after the vitals loop", () => {
    // Interface cut I8: two DISCRETE arms, deliberately outside the suffix
    // loop above (whose order the first test pins byte-for-byte) and ahead
    // of `scenes`.
    const qs = hydrationQueries(
      session(["comsys", "tactics.vitals", "tactics.unit", "tactics.contacts", "scenes"]),
    );
    const feeds = qs
      .map((q) => q.q)
      .flatMap((q) => (q.t === "panel_subscribe" ? [q.d.feed] : []));
    expect(feeds).toEqual([
      "comsys",
      "tactics.vitals",
      "tactics.unit",
      "tactics.contacts",
      "scenes",
    ]);
    expect(qs.map((q) => q.pending.kind)).toEqual([
      "who",
      "comsys.state",
      "commands",
      "vitals",
      "unit",
      "contacts",
      "scenes",
    ]);
    expect(qs.find((q) => q.pending.kind === "unit")?.pending).toEqual({
      kind: "unit",
      feed: "tactics.unit",
    });
    // A manifest without them asks for neither.
    const bare = hydrationQueries(session(["comsys"]));
    expect(bare.map((q) => q.pending.kind)).toEqual(["who", "comsys.state", "commands"]);
  });

  it("subscribes both arena feeds, after the contacts list and ahead of scenes", () => {
    // 16b cut Q.5: two more discrete arms. The cockpit's map is asked for on
    // the hello pass because it is PULLED and never pushed, so it is not in
    // the resume ring and a re-attach holds no terrain at all.
    const qs = hydrationQueries(
      session(["comsys", "tactics.contacts", "arena.cockpit", "arena.map", "scenes"]),
    );
    expect(qs.map((q) => q.pending.kind)).toEqual([
      "who",
      "comsys.state",
      "commands",
      "contacts",
      "cockpit",
      "arena.map",
      "scenes",
    ]);
    // A server with the cockpit but no map feed asks only for the cockpit.
    const one = hydrationQueries(session(["arena.cockpit"]));
    expect(one.map((q) => q.pending.kind)).toEqual(["who", "commands", "cockpit"]);
    // …and a server with neither asks for neither.
    const none = hydrationQueries(session(["comsys"]));
    expect(none.map((q) => q.pending.kind)).not.toContain("cockpit");
    expect(none.map((q) => q.pending.kind)).not.toContain("arena.map");
  });
});
