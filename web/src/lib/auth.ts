// Auth — thin wrappers over /api/auth/* (mush-net-web/src/auth.rs) with
// friendly error mapping. The opaque token from login/register is the only
// credential the SPA keeps (localStorage, via $lib/storage).
//
// Server shapes:
//   POST /api/auth/login    {account, password}            → {token, account, characters:[{key,name}]}
//   POST /api/auth/register {account, password, char_name} → same
//   POST /api/auth/logout   Authorization: Bearer <token>  → 204
// Errors arrive as {"error": "..."} with 401/403/409/422/429.

export interface CharSummary {
  /** Opaque numeric character key — the `?char=` WS parameter. */
  key: number;
  name: string;
}

export interface LoginOk {
  token: string;
  account: string;
  characters: CharSummary[];
}

/** A refused auth call, with a player-friendly message. */
export class AuthError extends Error {
  constructor(
    message: string,
    public readonly status: number,
  ) {
    super(message);
    this.name = "AuthError";
  }
}

export type FetchLike = (
  url: string,
  init: {
    method: string;
    headers: Record<string, string>;
    body?: string;
  },
) => Promise<{ status: number; json(): Promise<unknown> }>;

const defaultFetch: FetchLike = (url, init) => fetch(url, init);

async function errorMessage(
  res: { status: number; json(): Promise<unknown> },
  fallback: string,
): Promise<string> {
  try {
    const body = (await res.json()) as { error?: unknown };
    if (typeof body?.error === "string" && body.error.length > 0) {
      return body.error;
    }
  } catch {
    // non-JSON body
  }
  return fallback;
}

/** Friendly rewording of the server's terse error strings. */
function friendly(status: number, serverMsg: string): string {
  switch (status) {
    case 401:
      return "Invalid account name or password.";
    case 403:
      return "You are not allowed to connect from this address.";
    case 409:
      return "That account name is already taken.";
    case 429:
      return "Too many attempts — wait a moment and try again.";
    case 422:
      // BadName carries a real explanation; "password too weak" gets help.
      // (Matches the server's actual minimum — mushd/src/accounts.rs.)
      return serverMsg === "password too weak"
        ? "That password is too weak — use at least 4 characters."
        : serverMsg;
    default:
      return serverMsg || "The server refused the request.";
  }
}

async function post(
  fetchImpl: FetchLike,
  path: string,
  body: unknown,
  headers: Record<string, string> = {},
): Promise<{ status: number; json(): Promise<unknown> }> {
  try {
    return await fetchImpl(path, {
      method: "POST",
      headers: { "content-type": "application/json", ...headers },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch {
    throw new AuthError("Can't reach the server — is it up?", 0);
  }
}

function parseLoginOk(v: unknown): LoginOk {
  const o = (typeof v === "object" && v !== null ? v : {}) as Record<string, unknown>;
  const chars = Array.isArray(o.characters) ? o.characters : [];
  return {
    token: typeof o.token === "string" ? o.token : "",
    account: typeof o.account === "string" ? o.account : "",
    characters: chars.flatMap((c) => {
      const e = (typeof c === "object" && c !== null ? c : {}) as Record<string, unknown>;
      return typeof e.key === "number" && typeof e.name === "string"
        ? [{ key: e.key, name: e.name }]
        : [];
    }),
  };
}

export async function login(
  account: string,
  password: string,
  fetchImpl: FetchLike = defaultFetch,
): Promise<LoginOk> {
  const res = await post(fetchImpl, "/api/auth/login", { account, password });
  if (res.status !== 200) {
    throw new AuthError(friendly(res.status, await errorMessage(res, "")), res.status);
  }
  return parseLoginOk(await res.json());
}

export async function register(
  account: string,
  password: string,
  charName: string,
  fetchImpl: FetchLike = defaultFetch,
): Promise<LoginOk> {
  const res = await post(fetchImpl, "/api/auth/register", {
    account,
    password,
    char_name: charName,
  });
  if (res.status !== 200) {
    throw new AuthError(friendly(res.status, await errorMessage(res, "")), res.status);
  }
  return parseLoginOk(await res.json());
}

/** Best-effort revoke; never throws (logout must always succeed locally). */
export async function logout(
  token: string,
  fetchImpl: FetchLike = defaultFetch,
): Promise<void> {
  try {
    await post(fetchImpl, "/api/auth/logout", undefined, {
      authorization: `Bearer ${token}`,
    });
  } catch {
    // token TTL handles it server-side
  }
}
