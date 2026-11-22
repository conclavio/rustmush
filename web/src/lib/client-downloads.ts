// Standalone 3D client discovery (`/api/client-downloads`).
// The Rust gateway inspects its configured client downloads directory and returns
// the build matching the running server's commit (or newest build per OS), while
// Caddy serves the actual `.zip` bytes directly under `/downloads/clients/*`.

export type ClientTargetSlug = "windows-x64" | "linux-x64" | "macos-universal";

export interface ClientDownloadEntry {
  target: ClientTargetSlug;
  label: string;
  filename: string;
  url: string;
  commit?: string;
  exact_match: boolean;
  size_bytes: number;
  sha256?: string;
  unity_version?: string;
  built_at_utc?: string;
}

export interface ClientDownloadsResponse {
  server_commit?: string;
  clients: ClientDownloadEntry[];
}

export type SimpleFetch = (
  url: string,
  init?: { method?: string; headers?: Record<string, string> },
) => Promise<{ ok: boolean; json(): Promise<unknown> }>;

function isValidTarget(raw: unknown): raw is ClientTargetSlug {
  return raw === "windows-x64" || raw === "linux-x64" || raw === "macos-universal";
}

function isSafeDownloadUrl(raw: unknown): raw is string {
  if (typeof raw !== "string" || raw.length === 0) return false;
  return raw.startsWith("/") || raw.startsWith("https://") || raw.startsWith("http://");
}

export function parseClientDownloadsResponse(raw: unknown): ClientDownloadsResponse {
  if (!raw || typeof raw !== "object") {
    return { clients: [] };
  }
  const obj = raw as Record<string, unknown>;
  const serverCommit =
    typeof obj.server_commit === "string" && obj.server_commit.trim().length > 0
      ? obj.server_commit.trim()
      : undefined;
  const list = Array.isArray(obj.clients) ? obj.clients : [];
  const clients: ClientDownloadEntry[] = [];
  for (const item of list) {
    if (!item || typeof item !== "object") continue;
    const rec = item as Record<string, unknown>;
    if (
      !isValidTarget(rec.target) ||
      typeof rec.label !== "string" ||
      typeof rec.filename !== "string" ||
      !isSafeDownloadUrl(rec.url) ||
      typeof rec.exact_match !== "boolean" ||
      typeof rec.size_bytes !== "number" ||
      !Number.isFinite(rec.size_bytes) ||
      rec.size_bytes < 0
    ) {
      continue;
    }
    clients.push({
      target: rec.target,
      label: rec.label,
      filename: rec.filename,
      url: rec.url,
      exact_match: rec.exact_match,
      size_bytes: rec.size_bytes,
      commit: typeof rec.commit === "string" ? rec.commit : undefined,
      sha256: typeof rec.sha256 === "string" ? rec.sha256 : undefined,
      unity_version: typeof rec.unity_version === "string" ? rec.unity_version : undefined,
      built_at_utc: typeof rec.built_at_utc === "string" ? rec.built_at_utc : undefined,
    });
  }
  return { server_commit: serverCommit, clients };
}

export async function fetchClientDownloads(
  fetchImpl: SimpleFetch = (url, init) => fetch(url, init),
): Promise<ClientDownloadsResponse> {
  try {
    const res = await fetchImpl("/api/client-downloads", {
      method: "GET",
      headers: { Accept: "application/json" },
    });
    if (!res.ok) return { clients: [] };
    return parseClientDownloadsResponse(await res.json());
  } catch {
    return { clients: [] };
  }
}

export function detectClientTarget(userAgent: string): ClientTargetSlug | null {
  const ua = userAgent.toLowerCase();
  if (ua.includes("windows") || ua.includes("win64") || ua.includes("win32")) {
    return "windows-x64";
  }
  if (ua.includes("mac os") || ua.includes("macintosh")) {
    return "macos-universal";
  }
  if (ua.includes("linux") && !ua.includes("android")) {
    return "linux-x64";
  }
  return null;
}

export function orderClientDownloads(
  clients: readonly ClientDownloadEntry[],
  preferred: ClientTargetSlug | null,
): ClientDownloadEntry[] {
  if (!preferred || clients.length <= 1) {
    return [...clients];
  }
  const primary = clients.filter((c) => c.target === preferred);
  const rest = clients.filter((c) => c.target !== preferred);
  return [...primary, ...rest];
}

export function formatClientBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const mib = bytes / (1024 * 1024);
  if (mib >= 1024) {
    return `${(mib / 1024).toFixed(2)} GB`;
  }
  if (mib >= 1) {
    return `${Math.round(mib)} MB`;
  }
  const kib = bytes / 1024;
  if (kib >= 1) {
    return `${Math.round(kib)} KB`;
  }
  return `${Math.round(bytes)} B`;
}
