// Auth wrappers: request shapes, success parsing, friendly error mapping.

import { describe, expect, it } from "vitest";

import { AuthError, login, logout, register, type FetchLike } from "./auth";

interface Call {
  url: string;
  init: { method: string; headers: Record<string, string>; body?: string };
}

function fakeFetch(status: number, body: unknown): { fetch: FetchLike; calls: Call[] } {
  const calls: Call[] = [];
  const fetch: FetchLike = (url, init) => {
    calls.push({ url, init });
    return Promise.resolve({
      status,
      json: () => Promise.resolve(body),
    });
  };
  return { fetch, calls };
}

describe("auth", () => {
  it("login posts credentials and parses the payload", async () => {
    const { fetch, calls } = fakeFetch(200, {
      token: "tok",
      account: "nick",
      characters: [
        { key: 7, name: "Kal" },
        { key: 9, name: "Vex" },
        { bad: true },
      ],
    });
    const ok = await login("Nick", "hunter22", fetch);
    expect(calls[0]!.url).toBe("/api/auth/login");
    expect(JSON.parse(calls[0]!.init.body!)).toEqual({ account: "Nick", password: "hunter22" });
    expect(ok.token).toBe("tok");
    expect(ok.characters).toEqual([
      { key: 7, name: "Kal" },
      { key: 9, name: "Vex" },
    ]);
  });

  it("maps 401 to a friendly uniform message", async () => {
    const { fetch } = fakeFetch(401, { error: "invalid account or password" });
    await expect(login("a", "b", fetch)).rejects.toMatchObject({
      name: "AuthError",
      status: 401,
      message: "Invalid account name or password.",
    });
  });

  it("maps register errors: 409 taken, 422 weak/bad-name, 403 banned", async () => {
    const taken = fakeFetch(409, { error: "account name taken" });
    await expect(register("a", "b", "c", taken.fetch)).rejects.toMatchObject({
      message: "That account name is already taken.",
    });

    const weak = fakeFetch(422, { error: "password too weak" });
    await expect(register("a", "b", "c", weak.fetch)).rejects.toMatchObject({
      message: expect.stringContaining("too weak"),
    });

    const badName = fakeFetch(422, { error: "Names must start with a letter." });
    await expect(register("a", "b", "c", badName.fetch)).rejects.toMatchObject({
      message: "Names must start with a letter.", // server explains, keep it
    });

    const banned = fakeFetch(403, { error: "forbidden" });
    await expect(register("a", "b", "c", banned.fetch)).rejects.toMatchObject({ status: 403 });
  });

  it("maps network failure to a reachability message", async () => {
    const fetch: FetchLike = () => Promise.reject(new Error("ECONNREFUSED"));
    await expect(login("a", "b", fetch)).rejects.toMatchObject({
      status: 0,
      message: expect.stringContaining("reach the server"),
    });
    expect(new AuthError("x", 0)).toBeInstanceOf(Error);
  });

  it("register posts char_name; logout sends the bearer and never throws", async () => {
    const reg = fakeFetch(200, { token: "t", account: "a", characters: [] });
    await register("a", "pw", "Kal", reg.fetch);
    expect(JSON.parse(reg.calls[0]!.init.body!)).toEqual({
      account: "a",
      password: "pw",
      char_name: "Kal",
    });

    const out = fakeFetch(204, null);
    await logout("tok", out.fetch);
    expect(out.calls[0]!.url).toBe("/api/auth/logout");
    expect(out.calls[0]!.init.headers.authorization).toBe("Bearer tok");

    const boom: FetchLike = () => Promise.reject(new Error("net down"));
    await expect(logout("tok", boom)).resolves.toBeUndefined();
  });
});
