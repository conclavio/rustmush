// ChannelsStore: comsys.state hydration, live message flow, client-side
// unread math, tab activation, backlog merging, dedupe on resume replay.

import { describe, expect, it } from "vitest";

import { CHANNEL_MSG_CAP, ChannelsStore } from "./channels.svelte";

/** A comsys.state keyframe payload (see mush-comsys/src/query.rs). */
function stateData(
  channels: Array<Partial<Record<string, unknown>> & { name: string }>,
): unknown {
  return {
    channels: channels.map((c) => ({
      alias: "pub",
      title: null,
      gagged: false,
      color: { ansi: "yellow", web_hex: "#d1a458" },
      seq: 0,
      last_read: 0,
      unread: 0,
      loud: false,
      logged: true,
      ...c,
    })),
  };
}

function liveMsg(channel: string, seq: number): Parameters<ChannelsStore["live"]>[0] {
  return {
    channel,
    seq,
    actor: { id: "#5", name: "Wizard" },
    title: null,
    kind: "say",
    text: [{ text: `msg ${seq}` }],
    ts: 1000 + seq,
  };
}

describe("ChannelsStore", () => {
  it("hydrates from a keyframe (sorted, colors, unread from cursors)", () => {
    const st = new ChannelsStore();
    st.applyState(
      stateData([
        { name: "Public", seq: 10, last_read: 7 },
        { name: "Admin", seq: 3, last_read: 3, alias: "adm" },
      ]),
    );
    expect(st.list.map((c) => c.name)).toEqual(["Admin", "Public"]);
    expect(st.unreadOf("Public")).toBe(3);
    expect(st.unreadOf("Admin")).toBe(0);
    expect(st.byName("public")!.colorHex).toBe("#d1a458");
    expect(st.byName("Admin")!.alias).toBe("adm");
    expect(st.totalUnread).toBe(3);
  });

  it("accrues unread on live messages for inactive tabs only", () => {
    const st = new ChannelsStore();
    st.applyState(stateData([{ name: "Public", seq: 5, last_read: 5 }]));

    st.live(liveMsg("Public", 6));
    st.live(liveMsg("Public", 7));
    expect(st.unreadOf("Public")).toBe(2);

    st.activate("Public"); // opening the tab marks it read
    expect(st.unreadOf("Public")).toBe(0);

    st.live(liveMsg("Public", 8)); // active tab: read as it arrives
    expect(st.unreadOf("Public")).toBe(0);

    st.activate(null); // back to the main stream
    st.live(liveMsg("Public", 9));
    expect(st.unreadOf("Public")).toBe(1);
  });

  it("dedupes replayed live messages (returns false, no double unread)", () => {
    const st = new ChannelsStore();
    st.applyState(stateData([{ name: "Public", seq: 0, last_read: 0 }]));
    expect(st.live(liveMsg("Public", 1))).toBe(true);
    expect(st.live(liveMsg("Public", 1))).toBe(false);
    expect(st.messagesOf("Public")).toHaveLength(1);
    expect(st.unreadOf("Public")).toBe(1);
  });

  it("stubs a channel joined mid-session (message before any keyframe)", () => {
    const st = new ChannelsStore();
    st.live(liveMsg("Newbie", 4));
    expect(st.byName("Newbie")).toBeDefined();
    expect(st.unreadOf("Newbie")).toBe(1);
    expect(st.messagesOf("newbie")).toHaveLength(1);
  });

  it("a re-hydrate keeps local read cursors (no unread flash)", () => {
    const st = new ChannelsStore();
    st.applyState(stateData([{ name: "Public", seq: 5, last_read: 5 }]));
    st.live(liveMsg("Public", 6));
    st.activate("Public"); // read up to 6
    st.activate(null);
    // Server cursor lags (delivery-driven): keyframe says last_read 5.
    st.applyState(stateData([{ name: "Public", seq: 6, last_read: 5 }]));
    expect(st.unreadOf("Public")).toBe(0);
  });

  it("drops channels (and their messages) that vanish from the keyframe", () => {
    const st = new ChannelsStore();
    st.applyState(stateData([{ name: "Public" }, { name: "Gone" }]));
    st.live(liveMsg("Gone", 1));
    st.activate("Gone");
    st.applyState(stateData([{ name: "Public" }]));
    expect(st.byName("Gone")).toBeUndefined();
    expect(st.messagesOf("Gone")).toHaveLength(0);
    expect(st.active).toBeNull(); // active tab fell back to the main stream
  });

  it("merges backlog (newest-first wire order) ascending and dedupes", () => {
    const st = new ChannelsStore();
    st.applyState(stateData([{ name: "Public", seq: 5, last_read: 5 }]));
    st.live(liveMsg("Public", 5));
    const merged = st.applyBacklog({
      backlog: {
        channel: "Public",
        before_seq: null,
        messages: [4, 3, 2].map((seq) => ({
          seq,
          actor: { id: "#5", name: "Wizard" },
          title: null,
          kind: "say",
          text: [{ text: `old ${seq}` }],
          ts: 1000 + seq,
        })),
      },
    });
    expect(merged).toBe(3);
    expect(st.messagesOf("Public").map((m) => m.seq)).toEqual([2, 3, 4, 5]);
    expect(st.backlogFetched.public).toBe(true);
    // Unread untouched by history reads.
    expect(st.unreadOf("Public")).toBe(0);
  });

  it("caps per-channel messages", () => {
    const st = new ChannelsStore();
    for (let i = 1; i <= CHANNEL_MSG_CAP + 20; i++) {
      st.live(liveMsg("Public", i));
    }
    const msgs = st.messagesOf("Public");
    expect(msgs).toHaveLength(CHANNEL_MSG_CAP);
    expect(msgs[0]!.seq).toBe(21);
  });

  it("assigns unique message ids even when seq is 0 (live's own dedup skips seq<=0)", () => {
    // `live()` only dedupes when `msg.seq > 0`, so two seq-0 messages (a
    // malformed/not-yet-assigned wire value) both get pushed. Before this
    // fix, ChannelView keyed its #each by `msg.seq` — two entries sharing
    // seq 0 would give Svelte a duplicate key and crash the whole app
    // (each_key_duplicate). The store-assigned `id` must stay unique.
    const st = new ChannelsStore();
    st.applyState(stateData([{ name: "Public" }]));
    expect(st.live(liveMsg("Public", 0))).toBe(true);
    expect(st.live(liveMsg("Public", 0))).toBe(true); // not deduped (seq <= 0)
    const ids = st.messagesOf("Public").map((m) => m.id);
    expect(ids).toHaveLength(2);
    expect(new Set(ids).size).toBe(2); // no duplicate #each key
  });

  it("clearing backlogFetched after a full rehydrate lets a refetch merge safely", () => {
    // Mirrors GameApp's onFullRehydrate: the resume gap outgrew the ring, so
    // backlogFetched is cleared and the active tab's backlog is
    // re-requested. applyBacklog's seq-based dedup must make that refetch a
    // no-op for messages already held, while still picking up new ones.
    const st = new ChannelsStore();
    st.applyState(stateData([{ name: "Public", seq: 5, last_read: 5 }]));
    st.applyBacklog({
      backlog: {
        channel: "Public",
        messages: [5, 4, 3].map((seq) => ({
          seq,
          actor: { id: "#5", name: "Wizard" },
          kind: "say",
          text: [{ text: `m${seq}` }],
        })),
      },
    });
    expect(st.backlogFetched.public).toBe(true);

    // Full rehydrate: GameApp clears the flags…
    st.backlogFetched = {};
    // …then refetches; the server now also has seq 6 (arrived while gapped).
    const merged = st.applyBacklog({
      backlog: {
        channel: "Public",
        messages: [6, 5, 4, 3].map((seq) => ({
          seq,
          actor: { id: "#5", name: "Wizard" },
          kind: "say",
          text: [{ text: `m${seq}` }],
        })),
      },
    });
    expect(merged).toBe(1); // only seq 6 is new
    expect(st.messagesOf("Public").map((m) => m.seq)).toEqual([3, 4, 5, 6]);
    // Re-merged messages keep their message identity distinct too.
    const ids = st.messagesOf("Public").map((m) => m.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("survives malformed payloads", () => {
    const st = new ChannelsStore();
    st.applyState(null);
    st.applyState({ channels: "nope" });
    st.applyState({ channels: [null, 42, { alias: "x" }] });
    expect(st.list).toHaveLength(0);
    expect(st.applyBacklog({ backlog: { channel: "x" } })).toBe(0);
    expect(st.applyBacklog("garbage")).toBe(0);
  });
});
