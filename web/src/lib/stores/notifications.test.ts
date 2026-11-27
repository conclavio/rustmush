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

  it("fromQuestArrival emits objective and turn-in arrival toasts with /quests prefill", () => {
    const s = new NotificationStore();
    const obj = s.fromQuestArrival("Co-op Registry", false, 400)!;
    const ti = s.fromQuestArrival("Port Watch Arrivals", true, 500)!;
    expect(obj.category).toBe("quest");
    expect(obj.text[0]?.text).toBe("Quest Objective: Co-op Registry");
    expect(ti.text[0]?.text).toBe("Quest Turn-In Here: Port Watch Arrivals");
    expect(ti.action).toEqual({ label: "Quests", prefill: "/quests" });
    expect(s.toasts).toHaveLength(2);
  });
});
