import { describe, expect, it } from "vitest";

import {
  detectClientTarget,
  fetchClientDownloads,
  formatClientBytes,
  orderClientDownloads,
  parseClientDownloadsResponse,
  type ClientDownloadEntry,
} from "./client-downloads";

describe("client-downloads helper", () => {
  it("detects platform targets from browser user agents", () => {
    expect(detectClientTarget("Mozilla/5.0 (Windows NT 10.0; Win64; x64)")).toBe("windows-x64");
    expect(detectClientTarget("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)")).toBe(
      "macos-universal",
    );
    expect(detectClientTarget("Mozilla/5.0 (X11; Linux x86_64)")).toBe("linux-x64");
    expect(detectClientTarget("Mozilla/5.0 (Linux; Android 14)")).toBeNull();
  });

  it("orders preferred platform first and formats archive sizes cleanly", () => {
    const entries: ClientDownloadEntry[] = [
      {
        target: "windows-x64",
        label: "Windows (x64)",
        filename: "RustMUSH_TacSim-windows-x64-1922318d8.zip",
        url: "/downloads/clients/RustMUSH_TacSim-windows-x64-1922318d8.zip",
        commit: "1922318d8",
        exact_match: true,
        size_bytes: 155 * 1024 * 1024,
      },
      {
        target: "linux-x64",
        label: "Linux (x64)",
        filename: "RustMUSH_TacSim-linux-x64-1922318d8.zip",
        url: "/downloads/clients/RustMUSH_TacSim-linux-x64-1922318d8.zip",
        commit: "1922318d8",
        exact_match: false,
        size_bytes: 140 * 1024 * 1024,
      },
    ];

    const ordered = orderClientDownloads(entries, "linux-x64");
    expect(ordered.map((e) => e.target)).toEqual(["linux-x64", "windows-x64"]);
    expect(formatClientBytes(155 * 1024 * 1024)).toBe("155 MB");
    expect(formatClientBytes(1.5 * 1024 * 1024 * 1024)).toBe("1.50 GB");
    expect(formatClientBytes(64 * 1024)).toBe("64 KB");
  });

  it("parses valid payloads, strips unsafe URLs, and degrades cleanly on HTTP failure", async () => {
    const parsed = parseClientDownloadsResponse({
      server_commit: "1922318d8",
      clients: [
        {
          target: "windows-x64",
          label: "Windows (x64)",
          filename: "RustMUSH_TacSim-windows-x64-1922318d8.zip",
          url: "/downloads/clients/RustMUSH_TacSim-windows-x64-1922318d8.zip",
          commit: "1922318d8",
          exact_match: true,
          size_bytes: 1024 * 1024 * 120,
        },
        {
          target: "linux-x64",
          label: "Evil",
          filename: "bad.zip",
          url: "javascript:alert(1)",
          exact_match: false,
          size_bytes: 10,
        },
      ],
    });
    expect(parsed.server_commit).toBe("1922318d8");
    expect(parsed.clients).toHaveLength(1);
    expect(parsed.clients[0]?.target).toBe("windows-x64");

    const okRes = await fetchClientDownloads(async () => ({
      ok: true,
      json: async () => ({
        server_commit: "1922318d8",
        clients: [parsed.clients[0]],
      }),
    }));
    expect(okRes.clients).toHaveLength(1);

    const errRes = await fetchClientDownloads(async () => ({
      ok: false,
      json: async () => ({}),
    }));
    expect(errRes.clients).toEqual([]);
  });
});
