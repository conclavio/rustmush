// Authoritative Doc 49 (`CC.8` / `CC.9`) `space.cockpit` fire-picture, mount magazine,
// dual-mode laser posture, and 5-band hull aspect decoder for the Web Bridge Flight Visor.

export type SpaceMountFamily = "laser" | "tubes" | "spinal" | "point_defence";

export type SpaceMountState =
  | "ready"
  | "cycling"
  | "empty"
  | "too_hot"
  | "past_reach"
  | "down";

export type SpaceLaserMode = "offense" | "defense" | null;

export interface SpaceMountRow {
  id: string;
  family: SpaceMountFamily;
  state: SpaceMountState;
  readyInMs: number;
  mode: SpaceLaserMode;
  rounds: number | null;
  roundsMax: number | null;
  damage: number | null;
  flightMs: number | null;
}

export interface SpaceHeldLock {
  id: string;
  label: string;
  rangeM: number;
  facet: string; // "front" | "left" | "right" | "rear" | "top"
}

export interface SpaceMiningState {
  matchedRock: string | null;
  approachingRock: string | null;
  activeDrones: number;
  laserRock: string | null;
}

export interface SpaceFireSnapshot {
  mounts: SpaceMountRow[];
  lockId: string | null;
  lock: SpaceHeldLock | null;
  emcon: string;
  thrustPermille: number;
  mining: SpaceMiningState | null;
}

function asObj(v: unknown): Record<string, unknown> | null {
  return typeof v === "object" && v !== null && !Array.isArray(v)
    ? (v as Record<string, unknown>)
    : null;
}

export function emptySpaceFireSnapshot(): SpaceFireSnapshot {
  return {
    mounts: [],
    lockId: null,
    lock: null,
    emcon: "standard",
    thrustPermille: 0,
    mining: null,
  };
}

export function normalizeSpaceLaserMode(raw: unknown): SpaceLaserMode {
  if (typeof raw !== "string") return null;
  const w = raw.trim().toLowerCase();
  if (w === "offence" || w === "offense") return "offense";
  if (w === "defence" || w === "defense" || w === "point") return "defense";
  return null;
}

function normalizeFamily(raw: unknown): SpaceMountFamily {
  if (typeof raw !== "string") return "laser";
  const w = raw.trim().toLowerCase();
  if (w === "tubes" || w === "spinal" || w === "point_defence") return w;
  return "laser";
}

function normalizeState(raw: unknown): SpaceMountState {
  if (typeof raw !== "string") return "ready";
  const w = raw.trim().toLowerCase();
  if (
    w === "cycling" ||
    w === "empty" ||
    w === "too_hot" ||
    w === "past_reach" ||
    w === "down"
  ) {
    return w;
  }
  return "ready";
}

export function decodeSpaceFireFeed(cockpitData: unknown): SpaceFireSnapshot {
  const root = asObj(cockpitData);
  if (!root || root["active"] === false || root["seated"] === false) {
    return emptySpaceFireSnapshot();
  }

  const emcon =
    typeof root["emcon"] === "string" && root["emcon"].trim().length > 0
      ? root["emcon"].trim()
      : "standard";
  const thrustPermille =
    typeof root["thrust_permille"] === "number"
      ? root["thrust_permille"]
      : typeof root["throttle_permille"] === "number"
        ? root["throttle_permille"]
        : 0;

  let mining: SpaceMiningState | null = null;
  const miningObj = asObj(root["mining"]);
  if (miningObj) {
    const laserObj = asObj(miningObj["laser"]);
    const matchedRock =
      typeof miningObj["matched_rock"] === "string" &&
      miningObj["matched_rock"].trim().length > 0
        ? miningObj["matched_rock"].trim().toUpperCase()
        : null;
    const approachingRock =
      typeof miningObj["approaching_rock"] === "string" &&
      miningObj["approaching_rock"].trim().length > 0
        ? miningObj["approaching_rock"].trim().toUpperCase()
        : null;
    const activeDrones =
      typeof miningObj["active_drones"] === "number"
        ? Math.max(0, Math.trunc(miningObj["active_drones"]))
        : 0;
    const laserRock =
      laserObj && typeof laserObj["rock"] === "string" && laserObj["rock"].trim().length > 0
        ? laserObj["rock"].trim().toUpperCase()
        : null;
    mining = { matchedRock, approachingRock, activeDrones, laserRock };
  }

  const fireObj = asObj(root["fire"]);
  if (!fireObj) {
    return {
      mounts: [],
      lockId: null,
      lock: null,
      emcon,
      thrustPermille,
      mining,
    };
  }

  const mounts: SpaceMountRow[] = [];
  if (Array.isArray(fireObj["mounts"])) {
    for (const item of fireObj["mounts"]) {
      const m = asObj(item);
      if (!m) continue;
      const id = typeof m["id"] === "string" ? m["id"].trim() : "";
      if (!id) continue;
      mounts.push({
        id,
        family: normalizeFamily(m["family"]),
        state: normalizeState(m["state"]),
        readyInMs:
          typeof m["ready_in_ms"] === "number"
            ? Math.max(0, Math.trunc(m["ready_in_ms"]))
            : 0,
        mode: normalizeSpaceLaserMode(m["mode"]),
        rounds: typeof m["rounds"] === "number" ? Math.trunc(m["rounds"]) : null,
        roundsMax:
          typeof m["rounds_max"] === "number" ? Math.trunc(m["rounds_max"]) : null,
        damage: typeof m["damage"] === "number" ? Math.trunc(m["damage"]) : null,
        flightMs:
          typeof m["flight_ms"] === "number" ? Math.trunc(m["flight_ms"]) : null,
      });
    }
  }

  let lockId =
    typeof fireObj["lock_id"] === "string" && fireObj["lock_id"].trim().length > 0
      ? fireObj["lock_id"].trim()
      : null;

  let lock: SpaceHeldLock | null = null;
  const lockObj = asObj(fireObj["lock"]);
  if (lockObj) {
    const id = typeof lockObj["id"] === "string" ? lockObj["id"].trim() : "";
    if (id) {
      lockId = id;
      const label =
        typeof lockObj["label"] === "string" && lockObj["label"].trim().length > 0
          ? lockObj["label"].trim()
          : id;
      const rangeM =
        typeof lockObj["range_m"] === "number"
          ? Math.max(0, Math.trunc(lockObj["range_m"]))
          : 0;
      const facet =
        typeof lockObj["facet"] === "string" && lockObj["facet"].trim().length > 0
          ? lockObj["facet"].trim().toLowerCase()
          : "front";
      lock = { id, label, rangeM, facet };
    }
  }

  return {
    mounts,
    lockId,
    lock,
    emcon,
    thrustPermille,
    mining,
  };
}

export function formatMountMeta(m: SpaceMountRow): string {
  const familyWord =
    m.family === "tubes"
      ? "Tubes"
      : m.family === "spinal"
        ? "Spinal"
        : m.family === "point_defence"
          ? "Point Defense"
          : "Laser";

  if (m.mode === "offense") return `${familyWord} · Offense`;
  if (m.mode === "defense") return `${familyWord} · Defense`;
  if (m.rounds !== null && m.roundsMax !== null && m.roundsMax > 0) {
    return `${familyWord} · ${m.rounds}/${m.roundsMax}`;
  }
  if (m.rounds !== null) {
    return `${familyWord} · ${m.rounds} left`;
  }
  return familyWord;
}

export function formatMountStateLabel(m: SpaceMountRow): string {
  switch (m.state) {
    case "cycling":
      return `Cycling ${(m.readyInMs / 1000).toFixed(1)}s`;
    case "empty":
      return "Empty";
    case "too_hot":
      return "Too Hot";
    case "past_reach":
      return "Past Reach";
    case "down":
      return "Down";
    case "ready":
      return m.mode === "defense" ? "Defense Screen" : "Ready";
  }
}

export function formatMountAtLockEffect(m: SpaceMountRow): string | null {
  if (m.damage !== null && m.damage >= 0) {
    return `${m.damage} dmg`;
  }
  if (m.flightMs !== null && m.flightMs >= 0) {
    return `${(m.flightMs / 1000).toFixed(1)}s flight`;
  }
  return null;
}

export function formatHeldLockSummary(snap: SpaceFireSnapshot): string {
  if (snap.lock) {
    const dist =
      snap.lock.rangeM >= 1000
        ? `${(snap.lock.rangeM / 1000).toFixed(1)} km`
        : `${snap.lock.rangeM} m`;
    const band =
      snap.lock.facet.charAt(0).toUpperCase() + snap.lock.facet.slice(1);
    return `Lock: ${snap.lock.label} (${snap.lock.id}) · ${dist} · Strike Band: ${band}`;
  }
  if (snap.lockId) {
    return `Lock Designated: ${snap.lockId}`;
  }
  return "No Lock Held";
}
