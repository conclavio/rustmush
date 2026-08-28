// MailStore: mail.list keyframe hydration, unread badge, mail_notice.

import { describe, expect, it } from "vitest";

import { MailStore } from "./mail.svelte";

const keyframe = {
  folder: "inbox",
  folders: [
    { name: "inbox", count: 3, unread: 2 },
    { name: "sent", count: 5, unread: 0 },
  ],
  entries: [
    {
      index: 1,
      id: 41,
      from: { id: "#5", name: "Wizard" },
      subject: "hi",
      status: "unread",
      sent_at: 1756300000000,
      reply_to: null,
    },
    {
      index: 2,
      id: 44,
      from: { id: "#9", name: "Guest1" },
      subject: "urgent!",
      status: "urgent",
      sent_at: 1756300001000,
      reply_to: 41,
    },
    {
      index: 3,
      id: 47,
      from: { id: "#5", name: "Wizard" },
      subject: "old",
      status: "read",
      sent_at: 1756300002000,
      reply_to: null,
    },
  ],
  quota: { used: 3, max: 100 },
};

describe("MailStore", () => {
  it("hydrates folders, entries, quota and the unread badge", () => {
    const mail = new MailStore();
    expect(mail.loaded).toBe(false);
    mail.applyList(keyframe);
    expect(mail.loaded).toBe(true);
    expect(mail.folder).toBe("inbox");
    expect(mail.folders.map((f) => f.name)).toEqual(["inbox", "sent"]);
    expect(mail.entries).toHaveLength(3);
    expect(mail.entries[1]).toMatchObject({
      index: 2,
      status: "urgent",
      replyTo: 41,
      from: { name: "Guest1" },
    });
    expect(mail.quota).toEqual({ used: 3, max: 100 });
    expect(mail.unread).toBe(2); // inbox unread
  });

  it("staff quota (null) round-trips as null", () => {
    const mail = new MailStore();
    mail.applyList({ ...keyframe, quota: null });
    expect(mail.quota).toBeNull();
  });

  it("mail_notice bumps the badge; the next keyframe stays authoritative", () => {
    const mail = new MailStore();
    mail.applyList(keyframe);
    mail.notice(3);
    expect(mail.unread).toBe(3);
    mail.applyList(keyframe);
    expect(mail.unread).toBe(2);
  });

  it("listing another folder keeps the inbox badge", () => {
    const mail = new MailStore();
    mail.applyList({ ...keyframe, folder: "sent", entries: [] });
    expect(mail.folder).toBe("sent");
    expect(mail.unread).toBe(2);
  });

  it("assigns unique rowIds even when the server `id` is missing/duplicate", () => {
    // `id` defaults to 0 (via `num`) for a missing/malformed entry — two
    // such entries in one keyframe used to give MailPanel's `#each e.id`
    // key a duplicate, throwing each_key_duplicate and crashing the app.
    // `rowId` is store-assigned instead, so it stays unique.
    const mail = new MailStore();
    mail.applyList({
      folder: "inbox",
      folders: [],
      entries: [
        { index: 1, from: { id: "#5", name: "Wizard" }, subject: "a", status: "read" },
        { index: 2, from: { id: "#5", name: "Wizard" }, subject: "b", status: "read" },
      ],
    });
    expect(mail.entries).toHaveLength(2);
    expect(mail.entries.map((e) => e.id)).toEqual([0, 0]); // both defaulted — the old crash bait
    const rowIds = mail.entries.map((e) => e.rowId);
    expect(new Set(rowIds).size).toBe(2); // no duplicate #each key
  });

  it("survives malformed payloads", () => {
    const mail = new MailStore();
    mail.applyList(null);
    mail.applyList({ folders: "x", entries: [null, 7], quota: "y" });
    expect(mail.folders).toHaveLength(0);
    expect(mail.entries).toHaveLength(0);
    expect(mail.quota).toBeNull();
  });
});
