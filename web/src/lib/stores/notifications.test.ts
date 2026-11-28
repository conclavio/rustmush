// NotificationStore (interface plan I11): parsing, dedup, the toast/tray
// split, and the MailNotice compatibility shim.

import { describe, expect, it } from "vitest";

import { NotificationStore, parseNotice } from "./notifications.svelte";

describe("parseNotice", () => {
  it("parses a well-formed notice and defaults an unknown severity", () => {
    const n = parseNotice(
      {
        id: 3,
        severity: "urgent",
        category: "page",
        text: [{ text: "hi" }],
        action: { label: "Reply", prefill: "page X=" },
      },
      100,
    );
    expect(n?.serverId).toBe(3);
    expect(n?.severity).toBe("urgent");
    expect(n?.category).toBe("page");
    expect(n?.action).toEqual({ label: "Reply", prefill: "page X=" });
    expect(n?.ts).toBe(100);
    expect(parseNotice({ id: 1, severity: "wat", category: "x", text: [] }, 1)?.severity).toBe(
      "info",
    );
  });

  it("rejects a malformed notice", () => {
    expect(parseNotice({ id: 1, category: "x" }, 1)).toBeNull(); // no text array
    expect(parseNotice({ severity: "info", category: "x", text: [] }, 1)).toBeNull(); // no id
    expect(parseNotice("no", 1)).toBeNull();
  });
});

describe("NotificationStore", () => {
  it("adds newest-first, dedups by (serverId, category), and counts unseen", () => {
    const s = new NotificationStore();
    s.push({ id: 1, severity: "info", category: "mail", text: [{ text: "a" }] }, 10);
    s.push({ id: 2, severity: "warn", category: "escalation", text: [{ text: "b" }] }, 20);
    s.push({ id: 1, severity: "info", category: "mail", text: [{ text: "dup" }] }, 30);
    expect(s.list.map((n) => n.serverId)).toEqual([2, 1]);
    expect(s.unseen).toBe(2);
  });

  it("toasts are the newest unseen; markSeen leaves the toasts but keeps the tray", () => {
    const s = new NotificationStore();
    const n = s.push({ id: 1, severity: "info", category: "mail", text: [{ text: "a" }] }, 10)!;
    expect(s.toasts).toHaveLength(1);
    s.markSeen(n.rowId);
    expect(s.toasts).toHaveLength(0);
    expect(s.list).toHaveLength(1);
    expect(s.unseen).toBe(0);
  });

  it("markAllSeen clears the unseen count", () => {
    const s = new NotificationStore();
    s.push({ id: 1, severity: "info", category: "x", text: [{ text: "a" }] }, 1);
    s.push({ id: 2, severity: "info", category: "x", text: [{ text: "b" }] }, 2);
    expect(s.unseen).toBe(2);
    s.markAllSeen();
    expect(s.unseen).toBe(0);
    expect(s.list).toHaveLength(2);
  });

  it("the mail shim synthesizes a Notice with an @mail action, disjoint from server ids", () => {
    const s = new NotificationStore();
    const n = s.fromMail("Zoe", "Docks", 3, 100)!;
    expect(n.category).toBe("mail");
    expect(n.severity).toBe("info");
    expect(n.action).toEqual({ label: "Open", prefill: "@mail 3" });
    // A server notice with id 3 does not dedup against the mail one.
    s.push({ id: 3, severity: "info", category: "mail", text: [{ text: "other" }] }, 200);
    expect(s.list).toHaveLength(2);
  });

  it("dismiss and clear", () => {
    const s = new NotificationStore();
    const n = s.push({ id: 1, severity: "info", category: "x", text: [{ text: "a" }] }, 1)!;
    s.dismiss(n.rowId);
    expect(s.list).toHaveLength(0);
    s.push({ id: 2, severity: "info", category: "x", text: [{ text: "b" }] }, 2);
    s.clear();
    expect(s.list).toHaveLength(0);
  });

  it("fromQuestMilestone produces distinct transient quest toasts with /quests prefill", () => {
    const s = new NotificationStore();
    const a = s.fromQuestMilestone("accepted", "First Papers", 100)!;
    const r = s.fromQuestMilestone("ready", "First Papers", 200)!;
    const c = s.fromQuestMilestone("completed", "First Papers", 300)!;
    expect(a.category).toBe("quest");
    expect(a.text[0]?.text).toBe("Quest Accepted: First Papers");
    expect(r.text[0]?.text).toBe("Ready to Turn In [?]: First Papers");
    expect(c.text[0]?.text).toBe("Quest Completed: First Papers");
    expect(c.action).toEqual({ label: "Quests", prefill: "/quests" });
    expect(s.toasts).toHaveLength(3);
  });

  it("fromQuestArrival emits objective, turn-in, and between-steps next-guided arrival toasts with /quests prefill", () => {
    const s = new NotificationStore();
    const obj = s.fromQuestArrival("Co-op Registry", false, 400)!;
    const ti = s.fromQuestArrival("Port Watch Arrivals", true, 500)!;
    const nextG = s.fromQuestArrival("Exchange Hall", false, 600, {
      step: 6,
      giverLabel: "Rosalind Qureshi-Mbeki",
    })!;
    expect(obj.category).toBe("quest");
    expect(obj.text[0]?.text).toBe("Quest Objective: Co-op Registry");
    expect(ti.text[0]?.text).toBe("Quest Turn-In Here: Port Watch Arrivals");
    expect(nextG.text[0]?.text).toBe(
      "Next Guided Step (6/7) [!]: Speak with Rosalind Qureshi-Mbeki in Exchange Hall",
    );
    const board = s.fromQuestArrival("Visiting Berth Gallery", false, 700, undefined, true)!;
    expect(board.text[0]?.text).toBe("Board Ship Here: Visiting Berth Gallery (+ship/board)");
    expect(board.action).toEqual({ label: "Board Ship", prefill: "+ship/board" });
    expect(ti.action).toEqual({ label: "Quests", prefill: "/quests" });
    expect(s.toasts).toHaveLength(3);
    expect(s.list).toHaveLength(4);
  });

  it("fromQuestMilestone formats checkpoint, restartable abandon, and 7/7 Corridor Captain mastery toasts", () => {
    const s = new NotificationStore();
    const cp = s.fromQuestMilestone("checkpoint", "Stock the Locker", 100, {
      detail: "Buy 10 provisions",
    })!;
    const ab = s.fromQuestMilestone("abandoned", "Stock the Locker", 200, {
      restartable: true,
    })!;
    const cap = s.fromQuestMilestone("completed", "Deliver Your Mined Ore to Market", 300, {
      tourAllDone: true,
    })!;
    expect(cp.text[0]?.text).toBe("Objective Complete: Buy 10 provisions");
    expect(ab.text[0]?.text).toBe("Quest Abandoned (restartable anytime): Stock the Locker");
    expect(cap.text[0]?.text).toContain("Corridor Captain (7/7)");
    expect(cap.text[0]?.text).toContain("Deliver Your Mined Ore to Market");
  });
});
