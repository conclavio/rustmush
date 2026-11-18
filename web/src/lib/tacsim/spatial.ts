// Spatial models, pack parsers, hysteresis containment, and shortest-path BFS
// exit resolvers for the Web 3D Station & Ship Interior viewport.
// Mirrors Unity's `ContentModels.cs`, `StationWorldController.cs`, and `ShipInteriorController.cs`.

import type { ObjRef, RoomView } from "../proto";
import { plainText } from "../render";

export interface Vec2 {
  x: number;
  y: number;
}

export interface Vec3 {
  x: number;
  y: number;
  z: number;
}

export interface RectXZ {
  minX: number;
  maxX: number;
  minZ: number;
  maxZ: number;
}

export interface AtlasFloorDef {
  index: number;
  name: string;
  zOffsetM: number;
  heightM: number;
}

export interface AtlasRoomDef {
  entryId: string;
  name: string;
  shortDesc: string;
  desc: string;
  floorIndex: number;
  minX: number;
  minY: number;
  maxX: number;
  maxY: number;
  lighting: string;
  details: Record<string, string>;
}

export interface AtlasExitDef {
  entryId: string;
  fromRoom: string;
  toRoom: string;
  forwardLabel: string;
  forwardCommand: string;
  reverseLabel: string;
  reverseCommand: string;
  mode: string;
  locked: boolean;
  hidden: boolean;
  dzM: number;
  portalA: Vec3 | null;
  portalB: Vec3 | null;
}

export interface AtlasFeatureDef {
  entryId: string;
  featureKind: string;
  roomId: string;
  floorIndex: number;
  pointA: Vec2;
  pointB: Vec2;
  heightM: number;
  blocksMovement: boolean;
}

export interface AtlasNpcSeedDef {
  entryId: string;
  name: string;
  roomId: string;
  shortDesc: string;
  enabled: boolean;
  isMale: boolean;
}

export interface AtlasItemSeedDef {
  entryId: string;
  name: string;
  roomId: string;
  desc: string;
  isInterior: boolean;
}

export interface AtlasNeighborhoodPack {
  packId: string;
  name: string;
  floors: Map<number, AtlasFloorDef>;
  rooms: Map<string, AtlasRoomDef>;
  exits: AtlasExitDef[];
  features: AtlasFeatureDef[];
  seedNpcs: AtlasNpcSeedDef[];
  seedItems: AtlasItemSeedDef[];
}

export interface HorizontalDoorDef {
  floor: number;
  yWall: number;
  centerX: number;
  exit: AtlasExitDef;
}

export interface VerticalDoorDef {
  floor: number;
  xWall: number;
  centerY: number;
  exit: AtlasExitDef;
}

export interface ShipCompartmentDef {
  id: string;
  name: string;
  shortDesc: string;
  desc: string;
  localCenter: Vec3;
  localSize: Vec3;
  details: Record<string, string>;
}

export interface ShipExitPairDef {
  from: string;
  to: string;
  forwardLabel: string;
  forwardCommand: string;
  reverseLabel: string;
  reverseCommand: string;
}

export interface ShipUnitTemplate {
  unitId: string;
  label: string;
  compartments: Map<string, ShipCompartmentDef>;
  exits: ShipExitPairDef[];
}

export interface DockedHullSummary {
  id: string;
  name: string;
  mark: string;
  unit: string;
  captain: string;
  yours: boolean;
}

export interface StowageSnapshot {
  cargoKg: number;
  cargoMaxKg: number;
  cargoL: number;
  cargoMaxL: number;
  propellantKg: number;
  propellantMaxKg: number;
  provisionsCrewDays: number;
  provisionsMaxCrewDays: number;
  cargoLines: Array<{ good: string; label: string; qty: number; unit: string; massKg: number }>;
}

export interface SimulatorSlotInfo {
  index: number;
  team: string;
  pilotName: string;
  unit: string;
  seated: boolean;
}

export interface DecodedRoomAmbient {
  atlasPack: string | null;
  atlasEntry: string | null;
  atlasSurface: string | null;
  playerIds: Set<string>;
  npcIds: Set<string>;
  thingIds: Set<string>;
  dormantNpcIds: Set<string>;
  tacsimMode: string | null;
  unit: string | null;
  roomTemplateId: string | null;
  hullName: string | null;
  mark: string | null;
  classWord: string | null;
  port: string | null;
  portEntry: string | null;
  docked: boolean;
  settledPad: boolean;
  ownHull: string | null;
  ownHullName: string | null;
  ownHullDockedHere: boolean;
  dockedCount: number;
  dockedHulls: DockedHullSummary[];
  stowage: StowageSnapshot | null;
  ownStowage: StowageSnapshot | null;
  simMatchState: string | null;
  simAiFill: number;
  simMyTeam: string | null;
  simMyUnit: string | null;
  simInPod: boolean;
  simSlots: SimulatorSlotInfo[];
}

export interface VisibleRoomActor {
  id: string;
  name: string;
  isNpc: boolean;
  isMale: boolean;
  shortDesc: string;
}

// Companionway stairwell cutouts in ship local X/Z so climbing between Passage (Y=0),
// Bridge (Y=+3.4m), and Cargo Hold (Y=-3.6m) has full head & capsule clearance.
export const BRIDGE_STAIRWELL_XZ: RectXZ = { minX: -2.45, maxX: -0.75, minZ: 2.15, maxZ: 6.20 };
export const HOLD_STAIRWELL_XZ: RectXZ = { minX: 0.75, maxX: 2.45, minZ: -0.10, maxZ: 3.50 };

function asObj(v: unknown): Record<string, unknown> | null {
  return typeof v === "object" && v !== null && !Array.isArray(v)
    ? (v as Record<string, unknown>)
    : null;
}

function asStr(v: unknown, fallback = ""): string {
  return typeof v === "string" ? v : fallback;
}

function asNum(v: unknown, fallback = 0): number {
  return typeof v === "number" && Number.isFinite(v) ? v : fallback;
}

function asBool(v: unknown, fallback = false): boolean {
  return typeof v === "boolean" ? v : fallback;
}

function asArr(v: unknown): unknown[] | null {
  return Array.isArray(v) ? v : null;
}

export function getFloorElevationY(pack: AtlasNeighborhoodPack, floorIdx: number): number {
  const f = pack.floors.get(floorIdx);
  return f ? f.zOffsetM : floorIdx * 5.0;
}

export function getFloorHeight(pack: AtlasNeighborhoodPack, floorIdx: number): number {
  const f = pack.floors.get(floorIdx);
  return f ? f.heightM : 4.5;
}

export function getPackBoundsCenter2D(pack: AtlasNeighborhoodPack): Vec2 {
  if (pack.rooms.size === 0) return { x: 160, y: 150 };
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const r of pack.rooms.values()) {
    if (r.minX < minX) minX = r.minX;
    if (r.minY < minY) minY = r.minY;
    if (r.maxX > maxX) maxX = r.maxX;
    if (r.maxY > maxY) maxY = r.maxY;
  }
  return { x: (minX + maxX) * 0.5, y: (minY + maxY) * 0.5 };
}

export function getDefaultSpawnRoomEntry(pack: AtlasNeighborhoodPack): string {
  if (pack.rooms.has("room.c03_arrivals")) return "room.c03_arrivals";
  if (pack.rooms.has("room.simulator_lobby")) return "room.simulator_lobby";
  if (pack.rooms.has("room.welcome_hall")) return "room.welcome_hall";
  for (const r of pack.rooms.values()) {
    const low = `${r.entryId} ${r.name}`.toLowerCase();
    if (low.includes("arrival") || low.includes("concourse")) return r.entryId;
  }
  const first = pack.rooms.keys().next();
  return first.done ? "room.c03_arrivals" : first.value;
}

export function computePackGeometrySignature(pack: AtlasNeighborhoodPack): string {
  const parts: string[] = [pack.packId, `r=${pack.rooms.size}`, `e=${pack.exits.length}`];
  const keys = [...pack.rooms.keys()].sort();
  for (const k of keys) {
    const r = pack.rooms.get(k);
    if (r) parts.push(`${r.entryId}:${r.floorIndex}:${r.minX},${r.minY},${r.maxX},${r.maxY}`);
  }
  return parts.join("|");
}

function parseExitSpec(raw: string): { label: string; cmd: string } {
  const trimmed = raw.trim();
  if (!trimmed) return { label: "Exit", cmd: "n" };
  const parts = trimmed.split(";");
  const label = (parts[0] ?? "Exit").trim() || "Exit";
  const cmd = (parts[1] ?? "").trim() || label;
  return { label, cmd };
}

export function inferIsMale(actorName: string, bioOrPersona?: string | null): boolean {
  const n = actorName.toLowerCase();
  const knownMales = [
    "dov akintola",
    "nkem halvorsen",
    "corin mbatha",
    "matthias kalu",
    "halvard ndiaye",
    "eskil varga",
    "tomasz wen",
    "watch officer kade",
    "marcus",
    "anton",
    "viktor",
    "kade",
  ];
  for (const m of knownMales) {
    if (n.includes(m)) return true;
  }
  if (bioOrPersona) {
    const padded = ` ${bioOrPersona.toLowerCase()} `;
    const maleHits =
      (padded.includes(" he ") ? 1 : 0) +
      (padded.includes(" his ") ? 1 : 0) +
      (padded.includes(" him ") ? 1 : 0);
    const femaleHits =
      (padded.includes(" she ") ? 1 : 0) + (padded.includes(" her ") ? 1 : 0);
    if (maleHits > femaleHits) return true;
  }
  return false;
}

export function parseNeighborhoodPack(
  rawJson: string,
  fallbackId = "neighborhood.ceres_freeport",
): AtlasNeighborhoodPack {
  let parsed: unknown;
  try {
    parsed = JSON.parse(rawJson);
  } catch {
    return fallbackId.includes("ooc") ? buildFallbackOocArea() : buildFallbackFreeport();
  }
  const root = asObj(parsed);
  const entries = root ? asObj(root["entries"]) : null;
  if (!entries) {
    return fallbackId.includes("ooc") ? buildFallbackOocArea() : buildFallbackFreeport();
  }

  const pack: AtlasNeighborhoodPack = {
    packId: fallbackId,
    name: "Station",
    floors: new Map(),
    rooms: new Map(),
    exits: [],
    features: [],
    seedNpcs: [],
    seedItems: [],
  };

  for (const [id, entryVal] of Object.entries(entries)) {
    const entry = asObj(entryVal);
    if (!entry) continue;
    const kind = asStr(entry["kind"]);
    const def = asObj(entry["def"]);
    if (!def) continue;

    if (kind === "neighborhood") {
      pack.packId = id;
      pack.name = asStr(def["name"], pack.name);
    } else if (kind === "structure") {
      const floors = asArr(def["floors"]);
      if (floors) {
        for (const flVal of floors) {
          const fl = asObj(flVal);
          if (!fl) continue;
          const idx = Math.trunc(asNum(fl["index"], 0));
          pack.floors.set(idx, {
            index: idx,
            name: asStr(fl["name"], `Deck ${idx}`),
            zOffsetM: asNum(fl["z_offset_m"], idx * 5),
            heightM: asNum(fl["height_m"], 4.5),
          });
        }
      }
    } else if (kind === "room") {
      const surface = asObj(def["surface"]);
      const floorTuple = surface ? asArr(surface["floor"]) : null;
      const floorIdx =
        floorTuple && floorTuple.length >= 2 ? Math.trunc(asNum(floorTuple[1], 1)) : 1;

      const footprint = asObj(def["footprint"]);
      const rect = footprint ? asArr(footprint["rect"]) : asArr(def["rect"]);
      const poly =
        (footprint ? asArr(footprint["polygon"]) : null) ?? asArr(def["polygon"]);

      let minX = Infinity;
      let minY = Infinity;
      let maxX = -Infinity;
      let maxY = -Infinity;

      if (rect && rect.length >= 2) {
        const p0 = asArr(rect[0]);
        const p1 = asArr(rect[1]);
        if (p0 && p0.length >= 2 && p1 && p1.length >= 2) {
          const x0 = asNum(p0[0], 0);
          const y0 = asNum(p0[1], 0);
          const x1 = asNum(p1[0], 0);
          const y1 = asNum(p1[1], 0);
          minX = Math.min(x0, x1);
          maxX = Math.max(x0, x1);
          minY = Math.min(y0, y1);
          maxY = Math.max(y0, y1);
        }
      } else if (poly && poly.length >= 3) {
        for (const pt of poly) {
          const pair = asArr(pt);
          if (!pair || pair.length < 2) continue;
          const px = asNum(pair[0], 0);
          const py = asNum(pair[1], 0);
          if (px < minX) minX = px;
          if (py < minY) minY = py;
          if (px > maxX) maxX = px;
          if (py > maxY) maxY = py;
        }
      }
      if (!Number.isFinite(minX)) continue;

      const details: Record<string, string> = {};
      const detObj = asObj(def["details"]);
      if (detObj) {
        for (const [dk, dv] of Object.entries(detObj)) {
          if (typeof dv === "string") details[dk] = dv;
        }
      }
      const envObj = asObj(def["environment"]);
      pack.rooms.set(id, {
        entryId: id,
        name: asStr(def["name"], id),
        shortDesc: asStr(def["short_desc"]),
        desc: asStr(def["desc"]),
        floorIndex: floorIdx,
        minX,
        minY,
        maxX,
        maxY,
        lighting: envObj ? asStr(envObj["lighting"], "steady") : "steady",
        details,
      });
    } else if (kind === "exit") {
      const fromRoom = asStr(def["from"]);
      const toRoom = asStr(def["to"]);
      const names = asArr(def["names"]);
      const fwd = parseExitSpec(names && names.length > 0 ? asStr(names[0], "North;n") : "North;n");
      const rev = parseExitSpec(names && names.length > 1 ? asStr(names[1], "South;s") : "South;s");
      const portal = asObj(def["portal"]);
      const hidden =
        asBool(def["hidden"], false) ||
        (portal
          ? asBool(portal["hidden"], false) ||
            asStr(portal["visibility"], "public").toLowerCase() === "hidden"
          : false);

      let portalA: Vec3 | null = null;
      let portalB: Vec3 | null = null;
      const at = portal ? asArr(portal["at"]) : null;
      if (at && at.length >= 2) {
        const p0 = asArr(at[0]);
        const p1 = asArr(at[1]);
        if (p0 && p0.length >= 3 && p1 && p1.length >= 3) {
          portalA = { x: asNum(p0[0]), y: asNum(p0[2]), z: asNum(p0[1]) };
          portalB = { x: asNum(p1[0]), y: asNum(p1[2]), z: asNum(p1[1]) };
        }
      }

      pack.exits.push({
        entryId: id,
        fromRoom,
        toRoom,
        forwardLabel: fwd.label,
        forwardCommand: fwd.cmd,
        reverseLabel: rev.label,
        reverseCommand: rev.cmd,
        mode: portal ? asStr(portal["mode"], "door") : "door",
        locked: Boolean(asStr(def["lock"])),
        hidden,
        dzM: portal ? asNum(portal["dz_m"], 0) : 0,
        portalA,
        portalB,
      });
    } else if (kind === "feature") {
      const poly = asArr(def["polyline"]);
      if (!poly || poly.length < 2) continue;
      const p0 = asArr(poly[0]);
      const p1 = asArr(poly[1]);
      if (!p0 || p0.length < 2 || !p1 || p1.length < 2) continue;

      const surface = asObj(def["surface"]);
      const floorTuple = surface ? asArr(surface["floor"]) : null;
      const floorIdx =
        floorTuple && floorTuple.length >= 2 ? Math.trunc(asNum(floorTuple[1], 1)) : 1;
      const zRange = asArr(def["z_range"]);
      const h =
        zRange && zRange.length >= 2
          ? Math.max(0.5, asNum(zRange[1], 1.1) - asNum(zRange[0], 0))
          : 1.1;
      const blocks = asObj(def["blocks"]);

      pack.features.push({
        entryId: id,
        featureKind: asStr(def["feature_kind"], "wall"),
        roomId: asStr(def["room"]),
        floorIndex: floorIdx,
        pointA: { x: asNum(p0[0]), y: asNum(p0[1]) },
        pointB: { x: asNum(p1[0]), y: asNum(p1[1]) },
        heightM: h,
        blocksMovement: blocks ? asBool(blocks["movement"], false) : false,
      });
    } else if (kind === "npc" || kind === "person") {
      const profile = asObj(def["profile"]);
      const enabled =
        def["enabled"] !== undefined
          ? asBool(def["enabled"], true)
          : profile
            ? asBool(profile["enabled"], true)
            : true;
      const room = asStr(def["room"], asStr(def["home"]));
      if (room) {
        const npcName = asStr(def["name"], id);
        const npcBio = asStr(
          def["short_desc"],
          profile ? asStr(profile["background"]) : asStr(def["persona"]),
        );
        pack.seedNpcs.push({
          entryId: id,
          name: npcName,
          roomId: room,
          shortDesc: npcBio,
          enabled,
          isMale: inferIsMale(npcName, npcBio),
        });
      }
    } else if (kind === "item") {
      const room = asStr(def["room"]);
      if (room) {
        pack.seedItems.push({
          entryId: id,
          name: asStr(def["name"], id),
          roomId: room,
          desc: asStr(def["desc"]),
          isInterior: asBool(def["interior"], false),
        });
      }
    }
  }

  if (pack.rooms.size === 0) {
    return fallbackId.includes("ooc") ? buildFallbackOocArea() : buildFallbackFreeport();
  }
  return pack;
}

export function mergeCastPackIntoNeighborhood(
  pack: AtlasNeighborhoodPack,
  castRawJson: string,
): void {
  let parsed: unknown;
  try {
    parsed = JSON.parse(castRawJson);
  } catch {
    return;
  }
  const root = asObj(parsed);
  const entries = root ? asObj(root["entries"]) : null;
  if (!entries) return;

  const existingIds = new Set(pack.seedNpcs.map((n) => n.entryId));
  for (const [id, entryVal] of Object.entries(entries)) {
    if (existingIds.has(id)) continue;
    const entry = asObj(entryVal);
    if (!entry) continue;
    const kind = asStr(entry["kind"]);
    if (kind !== "npc" && kind !== "person") continue;
    const def = asObj(entry["def"]);
    if (!def) continue;
    const profile = asObj(def["profile"]);
    const enabled =
      def["enabled"] !== undefined
        ? asBool(def["enabled"], true)
        : profile
          ? asBool(profile["enabled"], true)
          : true;
    const room = asStr(def["room"], asStr(def["home"]));
    if (!room) continue;
    const npcName = asStr(def["name"], id);
    const npcBio = asStr(
      def["short_desc"],
      profile ? asStr(profile["background"]) : asStr(def["persona"]),
    );
    pack.seedNpcs.push({
      entryId: id,
      name: npcName,
      roomId: room,
      shortDesc: npcBio,
      enabled,
      isMale: inferIsMale(npcName, npcBio),
    });
  }
}

export function computeDoorwaysAndStairs(pack: AtlasNeighborhoodPack): {
  horizontalDoors: HorizontalDoorDef[];
  verticalDoors: VerticalDoorDef[];
  interFloorExits: AtlasExitDef[];
} {
  const horizontalDoors: HorizontalDoorDef[] = [];
  const verticalDoors: VerticalDoorDef[] = [];
  const interFloorExits: AtlasExitDef[] = [];

  for (const ex of pack.exits) {
    if (ex.hidden) continue;
    const rA = pack.rooms.get(ex.fromRoom);
    const rB = pack.rooms.get(ex.toRoom);
    if (!rA || !rB) continue;

    if (rA.floorIndex !== rB.floorIndex) {
      if (!ex.locked) interFloorExits.push(ex);
      continue;
    }

    if (Math.abs(rA.maxY - rB.minY) < 0.25 || Math.abs(rB.maxY - rA.minY) < 0.25) {
      const yShared = Math.abs(rA.maxY - rB.minY) < 0.25 ? rA.maxY : rA.minY;
      const xOverlapMin = Math.max(rA.minX, rB.minX);
      const xOverlapMax = Math.min(rA.maxX, rB.maxX);
      if (xOverlapMax - xOverlapMin >= 2.2) {
        horizontalDoors.push({
          floor: rA.floorIndex,
          yWall: yShared,
          centerX: (xOverlapMin + xOverlapMax) * 0.5,
          exit: ex,
        });
      }
    } else if (Math.abs(rA.maxX - rB.minX) < 0.25 || Math.abs(rB.maxX - rA.minX) < 0.25) {
      const xShared = Math.abs(rA.maxX - rB.minX) < 0.25 ? rA.maxX : rA.minX;
      const yOverlapMin = Math.max(rA.minY, rB.minY);
      const yOverlapMax = Math.min(rA.maxY, rB.maxY);
      if (yOverlapMax - yOverlapMin >= 2.2) {
        verticalDoors.push({
          floor: rA.floorIndex,
          xWall: xShared,
          centerY: (yOverlapMin + yOverlapMax) * 0.5,
          exit: ex,
        });
      }
    }
  }

  return { horizontalDoors, verticalDoors, interFloorExits };
}

export function findStationExitPath(
  pack: AtlasNeighborhoodPack,
  fromRoom: string,
  toRoom: string,
): Array<{ command: string; targetRoom: string }> {
  if (!fromRoom || !toRoom || fromRoom === toRoom) return [];
  const queue: string[] = [fromRoom];
  const visited = new Set<string>([fromRoom]);
  const cameFrom = new Map<string, { prev: string; cmd: string }>();

  while (queue.length > 0) {
    const cur = queue.shift()!;
    if (cur === toRoom) break;

    for (const ex of pack.exits) {
      if (ex.hidden || ex.locked) continue;
      if (ex.fromRoom === cur && !visited.has(ex.toRoom)) {
        visited.add(ex.toRoom);
        cameFrom.set(ex.toRoom, { prev: cur, cmd: ex.forwardCommand });
        queue.push(ex.toRoom);
      } else if (ex.toRoom === cur && !visited.has(ex.fromRoom)) {
        visited.add(ex.fromRoom);
        cameFrom.set(ex.fromRoom, { prev: cur, cmd: ex.reverseCommand });
        queue.push(ex.fromRoom);
      }
    }
  }

  if (!cameFrom.has(toRoom)) return [];
  const path: Array<{ command: string; targetRoom: string }> = [];
  let walk = toRoom;
  while (walk !== fromRoom) {
    const step = cameFrom.get(walk)!;
    path.push({ command: step.cmd, targetRoom: walk });
    walk = step.prev;
  }
  path.reverse();
  return path;
}

export function areRoomsConnectedWithinHops(
  pack: AtlasNeighborhoodPack,
  roomA: string,
  roomB: string,
  maxHops = 3,
): boolean {
  if (!roomA || !roomB) return false;
  if (roomA === roomB) return true;
  const p = findStationExitPath(pack, roomA, roomB);
  return p.length > 0 && p.length <= maxHops;
}

export function tryFindContainingRoomWithHysteresis(
  pack: AtlasNeighborhoodPack,
  worldPos: Vec3,
  originOffsetX: number,
  originOffsetZ: number,
  currentRoomEntryId: string | null,
  hysteresisM = 0.85,
): string | null {
  const ax = worldPos.x - originOffsetX;
  const ay = worldPos.z - originOffsetZ;

  if (currentRoomEntryId && hysteresisM > 0) {
    const cur = pack.rooms.get(currentRoomEntryId);
    if (cur) {
      const curElev = getFloorElevationY(pack, cur.floorIndex);
      if (
        ax >= cur.minX - hysteresisM &&
        ax <= cur.maxX + hysteresisM &&
        ay >= cur.minY - hysteresisM &&
        ay <= cur.maxY + hysteresisM &&
        Math.abs(worldPos.y - curElev) < 3.6
      ) {
        return currentRoomEntryId;
      }
    }
  }

  let best: string | null = null;
  let bestVert = Infinity;
  const vertCaptureLimit = currentRoomEntryId && hysteresisM > 0 ? 2.6 : 3.4;
  for (const r of pack.rooms.values()) {
    if (ax >= r.minX && ax <= r.maxX && ay >= r.minY && ay <= r.maxY) {
      const elev = getFloorElevationY(pack, r.floorIndex);
      const dy = Math.abs(worldPos.y - elev);
      if (dy < vertCaptureLimit && dy < bestVert) {
        bestVert = dy;
        best = r.entryId;
      }
    }
  }
  return best ?? currentRoomEntryId;
}

export function normalizeUnitId(unitId: string | null | undefined): string {
  const trimmed = (unitId ?? "").trim().toLowerCase();
  if (!trimmed) return "tacsim.unit.dray";
  return trimmed.startsWith("tacsim.unit.") ? trimmed : `tacsim.unit.${trimmed}`;
}

export function getStandardDrayCompartmentBounds(): Record<
  string,
  { center: Vec3; size: Vec3 }
> {
  return {
    airlock: { center: { x: 0, y: 0, z: 13 }, size: { x: 5.5, y: 3.2, z: 6 } },
    passage: { center: { x: 0, y: 0, z: 4 }, size: { x: 5.5, y: 3.2, z: 12 } },
    crew: { center: { x: 0, y: 0, z: -6 }, size: { x: 7.5, y: 3.2, z: 8 } },
    bridge: { center: { x: 0, y: 3.4, z: 9 }, size: { x: 7.5, y: 3.2, z: 9 } },
    hold: { center: { x: 0, y: -3.6, z: 2 }, size: { x: 9.0, y: 3.4, z: 12 } },
    engineering: { center: { x: 0, y: -3.6, z: -9 }, size: { x: 7.5, y: 3.4, z: 10 } },
    bay: { center: { x: 0, y: -3.6, z: -20 }, size: { x: 9.0, y: 3.6, z: 11 } },
  };
}

export function parseShipUnitJson(
  rawJson: string,
  expectedUnitId = "tacsim.unit.dray",
): ShipUnitTemplate {
  const normId = normalizeUnitId(expectedUnitId);
  let parsed: unknown;
  try {
    parsed = JSON.parse(rawJson);
  } catch {
    return buildFallbackDray(normId);
  }
  const root = asObj(parsed);
  const entries = root ? asObj(root["entries"]) : null;
  if (!entries) return buildFallbackDray(normId);

  let unitId = normId;
  let unitEntry = asObj(entries[normId]);
  if (!unitEntry) {
    for (const [k, v] of Object.entries(entries)) {
      const cand = asObj(v);
      if (cand && asStr(cand["kind"]) === "unit") {
        unitId = k;
        unitEntry = cand;
        break;
      }
    }
  }
  if (!unitEntry) return buildFallbackDray(normId);

  const def = asObj(unitEntry["def"]);
  const interior = def ? asObj(def["interior"]) : null;
  const rooms = interior ? asArr(interior["rooms"]) : null;
  if (!def || !interior || !rooms) return buildFallbackDray(normId);

  const layout = getStandardDrayCompartmentBounds();
  const tpl: ShipUnitTemplate = {
    unitId,
    label: asStr(def["label"], "Dray-class bulk hauler"),
    compartments: new Map(),
    exits: [],
  };

  rooms.forEach((rVal, i) => {
    const r = asObj(rVal);
    if (!r) return;
    const rKind = asStr(r["kind"]);
    if (rKind !== "walkable" && rKind !== "bay") return;
    const id = asStr(r["id"]);
    if (!id) return;
    const bounds = layout[id];
    const details: Record<string, string> = {};
    const detObj = asObj(r["details"]);
    if (detObj) {
      for (const [dk, dv] of Object.entries(detObj)) {
        if (typeof dv === "string") details[dk] = dv;
      }
    }
    tpl.compartments.set(id, {
      id,
      name: asStr(r["name"], id),
      shortDesc: asStr(
        r["short_desc"],
        rKind === "bay" ? "Two-rig stern vehicle bay (RIG 1 & RIG 2)." : "",
      ),
      desc: asStr(r["desc"]),
      localCenter: bounds ? { ...bounds.center } : { x: 0, y: 0, z: -i * 6 },
      localSize: bounds ? { ...bounds.size } : { x: 6, y: 3.2, z: 6 },
      details,
    });
  });

  const exits = asArr(interior["exits"]);
  if (exits) {
    for (const exVal of exits) {
      const ex = asObj(exVal);
      if (!ex) continue;
      const names = asArr(ex["names"]);
      const fwd = parseExitSpec(names && names.length > 0 ? asStr(names[0], "In;in") : "In;in");
      const rev = parseExitSpec(names && names.length > 1 ? asStr(names[1], "Out;out") : "Out;out");
      tpl.exits.push({
        from: asStr(ex["from"]),
        to: asStr(ex["to"]),
        forwardLabel: fwd.label,
        forwardCommand: fwd.cmd,
        reverseLabel: rev.label,
        reverseCommand: rev.cmd,
      });
    }
  }

  if (tpl.compartments.has("hold") && tpl.compartments.has("bay")) {
    tpl.exits.push({
      from: "hold",
      to: "bay",
      forwardLabel: "Vehicle Bay",
      forwardCommand: "+tv/bay in",
      reverseLabel: "Cargo Hold",
      reverseCommand: "+tv/bay out",
    });
  }

  return tpl.compartments.size > 0 ? tpl : buildFallbackDray(normId);
}

export function resolveCompartmentDef(
  tpl: ShipUnitTemplate,
  compartmentId: string | null | undefined,
): ShipCompartmentDef | null {
  const key = (compartmentId ?? "airlock").trim();
  const direct = tpl.compartments.get(key);
  if (direct) return direct;
  if (key.toLowerCase() === "corridor") return tpl.compartments.get("passage") ?? null;
  if (key.toLowerCase() === "passage") return tpl.compartments.get("corridor") ?? null;
  return null;
}

export function tryFindContainingCompartment(
  tpl: ShipUnitTemplate,
  localPos: Vec3,
  currentCompId: string | null,
  hysteresisM = 0.55,
): string | null {
  if (currentCompId && hysteresisM > 0) {
    const cur = resolveCompartmentDef(tpl, currentCompId);
    if (cur) {
      const hx = cur.localSize.x * 0.5;
      const hy = cur.localSize.y * 0.5;
      const hz = cur.localSize.z * 0.5;
      if (
        Math.abs(localPos.x - cur.localCenter.x) <= hx + hysteresisM &&
        Math.abs(localPos.y - cur.localCenter.y) <= hy + 0.9 &&
        Math.abs(localPos.z - cur.localCenter.z) <= hz + hysteresisM
      ) {
        return currentCompId;
      }
    }
  }

  for (const [k, c] of tpl.compartments) {
    const hx = c.localSize.x * 0.5;
    const hy = c.localSize.y * 0.5;
    const hz = c.localSize.z * 0.5;
    if (
      Math.abs(localPos.x - c.localCenter.x) <= hx &&
      Math.abs(localPos.y - c.localCenter.y) <= hy + 0.9 &&
      Math.abs(localPos.z - c.localCenter.z) <= hz
    ) {
      return k;
    }
  }
  return currentCompId;
}

export function findShipExitPath(
  tpl: ShipUnitTemplate,
  fromComp: string,
  toComp: string,
): Array<{ command: string; targetCompartment: string }> {
  const cFrom = resolveCompartmentDef(tpl, fromComp)?.id ?? fromComp;
  const cTo = resolveCompartmentDef(tpl, toComp)?.id ?? toComp;
  if (!cFrom || !cTo || cFrom === cTo) return [];

  const queue: string[] = [cFrom];
  const visited = new Set<string>([cFrom]);
  const cameFrom = new Map<string, { prev: string; cmd: string }>();

  while (queue.length > 0) {
    const cur = queue.shift()!;
    if (cur === cTo) break;
    for (const ex of tpl.exits) {
      if (ex.from === cur && !visited.has(ex.to)) {
        visited.add(ex.to);
        cameFrom.set(ex.to, { prev: cur, cmd: ex.forwardCommand });
        queue.push(ex.to);
      } else if (ex.to === cur && !visited.has(ex.from)) {
        visited.add(ex.from);
        cameFrom.set(ex.from, { prev: cur, cmd: ex.reverseCommand });
        queue.push(ex.from);
      }
    }
  }

  if (!cameFrom.has(cTo)) return [];
  const path: Array<{ command: string; targetCompartment: string }> = [];
  let walk = cTo;
  while (walk !== cFrom) {
    const step = cameFrom.get(walk)!;
    path.push({ command: step.cmd, targetCompartment: walk });
    walk = step.prev;
  }
  path.reverse();
  return path;
}

function parseIdSet(arrVal: unknown): Set<string> {
  const out = new Set<string>();
  const arr = asArr(arrVal);
  if (!arr) return out;
  for (const item of arr) {
    if (typeof item === "string" && item.length > 0) out.add(item);
    else {
      const obj = asObj(item);
      const id = obj ? asStr(obj["id"]) : "";
      if (id) out.add(id);
    }
  }
  return out;
}

export function parseStowageNode(nodeVal: unknown): StowageSnapshot | null {
  const node = asObj(nodeVal);
  if (!node) return null;
  const cargoLines: StowageSnapshot["cargoLines"] = [];
  const linesArr = asArr(node["cargo"]) ?? asArr(node["lines"]);
  if (linesArr) {
    for (const lnVal of linesArr) {
      const ln = asObj(lnVal);
      if (!ln) continue;
      cargoLines.push({
        good: asStr(ln["good"], asStr(ln["id"])),
        label: asStr(ln["label"], asStr(ln["good"])),
        qty: asNum(ln["qty"], asNum(ln["quantity"], 0)),
        unit: asStr(ln["unit"], "t"),
        massKg: asNum(ln["mass_kg"], 0),
      });
    }
  }
  return {
    cargoKg: asNum(node["cargo_kg"], 0),
    cargoMaxKg: Math.max(1, asNum(node["cargo_max_kg"], 100_000)),
    cargoL: asNum(node["cargo_l"], 0),
    cargoMaxL: Math.max(1, asNum(node["cargo_max_l"], 120_000)),
    propellantKg: asNum(node["propellant_kg"], asNum(node["tank_kg"], 0)),
    propellantMaxKg: Math.max(1, asNum(node["propellant_max_kg"], asNum(node["tank_max_kg"], 80_000))),
    provisionsCrewDays: asNum(node["provisions_crew_days"], asNum(node["stores_days"], 0)),
    provisionsMaxCrewDays: Math.max(
      1,
      asNum(node["provisions_max_crew_days"], asNum(node["stores_max_days"], 120)),
    ),
    cargoLines,
  };
}

export function decodeTacsimRoomAmbient(view: RoomView | null): DecodedRoomAmbient {
  const amb = view?.ambient ? asObj(view.ambient) : null;
  const atlas = amb ? asObj(amb["atlas"]) : null;
  const tac = amb ? asObj(amb["tacsim"]) : null;

  const dockedHulls: DockedHullSummary[] = [];
  const dhArr = tac ? asArr(tac["docked_hulls"]) : null;
  if (dhArr) {
    for (const hVal of dhArr) {
      const h = asObj(hVal);
      if (!h) continue;
      dockedHulls.push({
        id: asStr(h["id"]),
        name: asStr(h["name"]),
        mark: asStr(h["mark"]),
        unit: asStr(h["unit"], "dray"),
        captain: asStr(h["captain"]),
        yours: asBool(h["yours"], false),
      });
    }
  }

  const simSlots: SimulatorSlotInfo[] = [];
  const slotsArr = tac ? asArr(tac["slots"]) : null;
  if (slotsArr) {
    for (const sVal of slotsArr) {
      const s = asObj(sVal);
      if (!s) continue;
      simSlots.push({
        index: Math.trunc(asNum(s["index"], 0)),
        team: asStr(s["team"], "red"),
        pilotName: asStr(s["pilot_name"], asStr(s["name"])),
        unit: asStr(s["unit"], "line_tank"),
        seated: asBool(s["seated"], false),
      });
    }
  }

  return {
    atlasPack: atlas
      ? asStr(atlas["pack"]) || asStr(atlas["neighborhood_pack"]) || null
      : null,
    atlasEntry: atlas ? asStr(atlas["entry"]) || null : null,
    atlasSurface: atlas ? asStr(atlas["surface"]) || null : null,
    playerIds: amb ? parseIdSet(amb["players"]) : new Set(),
    npcIds: amb ? parseIdSet(amb["npcs"]) : new Set(),
    thingIds: amb ? parseIdSet(amb["things"]) : new Set(),
    dormantNpcIds: amb ? parseIdSet(amb["dormant_npcs"]) : new Set(),
    tacsimMode: tac ? asStr(tac["mode"]) || null : null,
    unit: tac ? asStr(tac["unit"]) || null : null,
    roomTemplateId: tac
      ? asStr(tac["room"]) || asStr(tac["room_template_id"]) || null
      : null,
    hullName: tac ? asStr(tac["hull_name"]) || null : null,
    mark: tac ? asStr(tac["mark"]) || null : null,
    classWord: tac ? asStr(tac["class"], "Dray") || null : null,
    port: tac ? asStr(tac["port"]) || null : null,
    portEntry: tac ? asStr(tac["port_entry"]) || null : null,
    docked: tac ? asBool(tac["docked"], true) : true,
    settledPad: tac ? asBool(tac["settled_pad"], false) : false,
    ownHull: tac ? asStr(tac["own_hull"]) || null : null,
    ownHullName: tac ? asStr(tac["own_hull_name"]) || null : null,
    ownHullDockedHere: tac ? asBool(tac["own_hull_docked_here"], false) : false,
    dockedCount: tac ? Math.trunc(asNum(tac["docked_count"], dockedHulls.length)) : 0,
    dockedHulls,
    stowage: tac ? parseStowageNode(tac["stowage"]) : null,
    ownStowage: tac ? parseStowageNode(tac["own_stowage"]) : null,
    simMatchState: tac ? asStr(tac["match_state"]) || null : null,
    simAiFill: tac ? Math.trunc(asNum(tac["ai_fill"], 0)) : 0,
    simMyTeam: tac ? asStr(tac["my_team"]) || null : null,
    simMyUnit: tac ? asStr(tac["my_unit"]) || null : null,
    simInPod: tac ? asBool(tac["in_pod"], false) : false,
    simSlots,
  };
}

export function resolveShipCompartmentFromRoom(
  view: RoomView | null,
  decoded: DecodedRoomAmbient,
  tpl: ShipUnitTemplate,
): string | null {
  if (!view) return null;
  if (decoded.tacsimMode === "interior" && decoded.roomTemplateId) {
    return decoded.roomTemplateId;
  }
  const trimmed = view.name.trim().toLowerCase();
  if (!trimmed) return null;
  for (const [key, comp] of tpl.compartments) {
    const cname = comp.name.toLowerCase();
    if (trimmed.endsWith(` - ${cname}`) || trimmed.endsWith(` — ${cname}`)) {
      return key;
    }
  }
  return null;
}

export function resolveStationPackAndEntry(
  view: RoomView | null,
  decoded: DecodedRoomAmbient,
  loadedPack: AtlasNeighborhoodPack,
): { desiredPackId: string; entryId: string | null } {
  let desiredPackId = decoded.atlasPack ?? loadedPack.packId;
  if (!decoded.atlasPack && view) {
    const rn = view.name.trim().toLowerCase();
    if (
      rn === "simulator lobby" ||
      rn === "red room" ||
      rn === "blue room" ||
      rn === "welcome hall"
    ) {
      desiredPackId = "neighborhood.ooc_area";
    }
  }

  if (decoded.atlasEntry) {
    return { desiredPackId, entryId: decoded.atlasEntry };
  }
  if (view) {
    const trimmed = view.name.trim().toLowerCase();
    for (const [entryId, room] of loadedPack.rooms) {
      if (room.name.toLowerCase() === trimmed) {
        return { desiredPackId, entryId };
      }
    }
  }
  return { desiredPackId, entryId: null };
}

function looksLikeInanimateFixture(name: string): boolean {
  const low = name.toLowerCase();
  return (
    low.includes("pod ") ||
    low.includes("selector") ||
    low.includes("board") ||
    low.includes("screen") ||
    low.includes("slate") ||
    low.includes("locker") ||
    low.includes("bunk") ||
    low.includes("couch") ||
    low.includes("console") ||
    low.includes("hatch") ||
    low.includes("door") ||
    low.startsWith("rig ")
  );
}

export function filterVisibleRoomActors(
  view: RoomView | null,
  decoded: DecodedRoomAmbient,
  selfActorId: string | null,
  pack: AtlasNeighborhoodPack | null,
): VisibleRoomActor[] {
  if (!view) return [];
  const dockedHullIds = new Set<string>();
  const dockedHullNames = new Set<string>();
  if (decoded.ownHull) dockedHullIds.add(decoded.ownHull);
  if (decoded.ownHullName) dockedHullNames.add(decoded.ownHullName.toLowerCase());
  for (const dh of decoded.dockedHulls) {
    if (dh.id) dockedHullIds.add(dh.id);
    if (dh.name) dockedHullNames.add(dh.name.toLowerCase());
  }

  const hasAuthoritativeTyping =
    decoded.thingIds.size > 0 || decoded.playerIds.size > 0 || decoded.npcIds.size > 0;
  const isBerthGallery =
    decoded.tacsimMode === "berth_gallery" ||
    (decoded.atlasEntry?.toLowerCase().includes("berth_gallery") ?? false);

  const out: VisibleRoomActor[] = [];
  for (const item of view.contents) {
    if (!item.id) continue;
    if (selfActorId && item.id === selfActorId) continue;
    if (decoded.dormantNpcIds.has(item.id) || decoded.thingIds.has(item.id)) continue;

    const seedNpc = pack?.seedNpcs.find(
      (n) => n.name.toLowerCase() === item.name.toLowerCase(),
    );
    if (seedNpc && !seedNpc.enabled) continue;

    const isNpc = decoded.npcIds.has(item.id) || Boolean(seedNpc?.enabled);
    const isPlayer = decoded.playerIds.has(item.id);

    if (!isNpc && !isPlayer) {
      if (
        dockedHullIds.has(item.id) ||
        dockedHullNames.has(item.name.toLowerCase()) ||
        hasAuthoritativeTyping ||
        isBerthGallery ||
        looksLikeInanimateFixture(item.name)
      ) {
        continue;
      }
    }

    out.push({
      id: item.id,
      name: item.name,
      isNpc,
      isMale: seedNpc ? seedNpc.isMale : inferIsMale(item.name, null),
      shortDesc: seedNpc?.shortDesc ?? "",
    });
  }
  return out;
}

export function extractRoomDescriptionPlain(view: RoomView | null): string {
  if (!view) return "";
  return plainText(view.desc);
}

export function findSeedNpcByName(
  pack: AtlasNeighborhoodPack,
  name: string,
): AtlasNpcSeedDef | undefined {
  const low = name.toLowerCase();
  return pack.seedNpcs.find((n) => n.name.toLowerCase() === low);
}

export function buildFallbackFreeport(): AtlasNeighborhoodPack {
  const pack: AtlasNeighborhoodPack = {
    packId: "neighborhood.ceres_freeport",
    name: "Ceres Freeport",
    floors: new Map([
      [0, { index: 0, name: "Freight Deck", zOffsetM: 0, heightM: 6 }],
      [1, { index: 1, name: "Public Deck", zOffsetM: 6, heightM: 4.5 }],
      [2, { index: 2, name: "Upper Offices", zOffsetM: 10.5, heightM: 4.2 }],
    ]),
    rooms: new Map(),
    exits: [],
    features: [],
    seedNpcs: [
      {
        entryId: "npc.ilse",
        name: "Ilse Oduya-Brandt",
        roomId: "room.c05_registry",
        shortDesc: "Co-op First Captain registrar",
        enabled: true,
        isMale: false,
      },
      {
        entryId: "npc.margit",
        name: "Margit Søndergaard",
        roomId: "room.c07_exchange",
        shortDesc: "Freeport Commodity Exchange factor",
        enabled: true,
        isMale: false,
      },
      {
        entryId: "npc.dov",
        name: "Dov Akintola",
        roomId: "room.c06_chandlery",
        shortDesc: "Co-op chandler & bunkering clerk",
        enabled: true,
        isMale: true,
      },
    ],
    seedItems: [],
  };

  const addRoom = (
    entryId: string,
    name: string,
    floorIndex: number,
    minX: number,
    minY: number,
    maxX: number,
    maxY: number,
    shortDesc: string,
  ): void => {
    pack.rooms.set(entryId, {
      entryId,
      name,
      shortDesc,
      desc: shortDesc,
      floorIndex,
      minX,
      minY,
      maxX,
      maxY,
      lighting: "steady",
      details: {},
    });
  };

  addRoom("room.c01_berth_gallery", "Visiting Berth Gallery", 1, 110, 110, 210, 125, "Armourglass gallery above the floodlit ship cradles.");
  addRoom("room.c02_pressure", "Pressure Lobby", 1, 110, 125, 140, 140, "Inner airlock lobby leading from the Berth Gallery.");
  addRoom("room.c03_arrivals", "Port Watch Arrivals", 1, 110, 140, 140, 165, "Customs counter, Council notices, and stair to the freight deck.");
  addRoom("room.c04_concourse", "Freeport Concourse", 1, 140, 140, 175, 165, "Wide public arcade connecting Registry, Chandlery, and Exchange.");
  addRoom("room.c05_registry", "Registry & Co-op Desk", 1, 110, 165, 140, 190, "Joint counter for the Belt Registry and First Captain charters.");
  addRoom("room.c06_chandlery", "Co-op Chandlery", 1, 140, 165, 175, 190, "Voyage provisions cases and Occator reaction-mass bunkering.");
  addRoom("room.c07_exchange", "Exchange Hall", 1, 175, 140, 210, 165, "Live commodity asks, standing bids, and clearing house boards.");
  addRoom("room.c08_dockmaster", "Dockmaster's Office", 0, 110, 140, 140, 165, "Sailing plan clearances and berth assignments.");

  const addExit = (
    entryId: string,
    fromRoom: string,
    toRoom: string,
    fwdLbl: string,
    fwdCmd: string,
    revLbl: string,
    revCmd: string,
    mode = "door",
  ): void => {
    pack.exits.push({
      entryId,
      fromRoom,
      toRoom,
      forwardLabel: fwdLbl,
      forwardCommand: fwdCmd,
      reverseLabel: revLbl,
      reverseCommand: revCmd,
      mode,
      locked: false,
      hidden: false,
      dzM: 0,
      portalA: null,
      portalB: null,
    });
  };

  addExit("exit.c01_c02", "room.c01_berth_gallery", "room.c02_pressure", "Pressure Lobby", "n", "Berth Gallery", "s");
  addExit("exit.c02_c03", "room.c02_pressure", "room.c03_arrivals", "Arrivals", "n", "Pressure Lobby", "s");
  addExit("exit.c03_c04", "room.c03_arrivals", "room.c04_concourse", "Concourse", "e", "Arrivals", "w");
  addExit("exit.c03_c05", "room.c03_arrivals", "room.c05_registry", "Registry", "n", "Arrivals", "s");
  addExit("exit.c04_c06", "room.c04_concourse", "room.c06_chandlery", "Chandlery", "n", "Concourse", "s");
  addExit("exit.c04_c07", "room.c04_concourse", "room.c07_exchange", "Exchange", "e", "Concourse", "w");
  addExit("exit.c03_c08", "room.c03_arrivals", "room.c08_dockmaster", "Freight Stair", "d", "Arrivals", "u", "stair");

  return pack;
}

export function buildFallbackOocArea(): AtlasNeighborhoodPack {
  const pack: AtlasNeighborhoodPack = {
    packId: "neighborhood.ooc_area",
    name: "OOC Simulator Centre",
    floors: new Map([[1, { index: 1, name: "Main Floor", zOffsetM: 0, heightM: 4.5 }]]),
    rooms: new Map(),
    exits: [],
    features: [],
    seedNpcs: [],
    seedItems: [],
  };

  const addRoom = (
    entryId: string,
    name: string,
    minX: number,
    minY: number,
    maxX: number,
    maxY: number,
    shortDesc: string,
  ): void => {
    pack.rooms.set(entryId, {
      entryId,
      name,
      shortDesc,
      desc: shortDesc,
      floorIndex: 1,
      minX,
      minY,
      maxX,
      maxY,
      lighting: "steady",
      details: {},
    });
  };

  addRoom("room.welcome_hall", "Welcome Hall", 95, 65, 117, 85, "OOC arrival hall with transit gate to Ceres Freeport.");
  addRoom("room.simulator_lobby", "Simulator Lobby", 117, 65, 145, 85, "6v6 tactical arena roster console and team ready room.");
  addRoom("room.sim_red", "Red Room", 117, 85, 145, 105, "Six Red Team armoured cockpit simulator pods.");
  addRoom("room.sim_blue", "Blue Room", 117, 45, 145, 65, "Six Blue Team armoured cockpit simulator pods.");

  pack.exits.push(
    {
      entryId: "exit.hall_sim",
      fromRoom: "room.welcome_hall",
      toRoom: "room.simulator_lobby",
      forwardLabel: "Simulator Lobby",
      forwardCommand: "simulator",
      reverseLabel: "Welcome Hall",
      reverseCommand: "w",
      mode: "door",
      locked: false,
      hidden: false,
      dzM: 0,
      portalA: null,
      portalB: null,
    },
    {
      entryId: "exit.sim_red",
      fromRoom: "room.simulator_lobby",
      toRoom: "room.sim_red",
      forwardLabel: "Red Room",
      forwardCommand: "red",
      reverseLabel: "Simulator Lobby",
      reverseCommand: "lobby",
      mode: "door",
      locked: false,
      hidden: false,
      dzM: 0,
      portalA: null,
      portalB: null,
    },
    {
      entryId: "exit.sim_blue",
      fromRoom: "room.simulator_lobby",
      toRoom: "room.sim_blue",
      forwardLabel: "Blue Room",
      forwardCommand: "blue",
      reverseLabel: "Simulator Lobby",
      reverseCommand: "lobby",
      mode: "door",
      locked: false,
      hidden: false,
      dzM: 0,
      portalA: null,
      portalB: null,
    },
  );

  pack.seedItems.push({
    entryId: "item.simulator_selector",
    name: "Simulator Selector",
    roomId: "room.simulator_lobby",
    desc: "Team & unit chassis selector console.",
    isInterior: false,
  });
  for (let i = 1; i <= 6; i++) {
    pack.seedItems.push({
      entryId: `item.red_pod_${i}`,
      name: `Red Pod ${i}`,
      roomId: "room.sim_red",
      desc: "Simulator cockpit pod",
      isInterior: true,
    });
    pack.seedItems.push({
      entryId: `item.blue_pod_${i}`,
      name: `Blue Pod ${i}`,
      roomId: "room.sim_blue",
      desc: "Simulator cockpit pod",
      isInterior: true,
    });
  }
  return pack;
}

export function buildFallbackDray(unitId = "tacsim.unit.dray"): ShipUnitTemplate {
  const norm = normalizeUnitId(unitId);
  const tpl: ShipUnitTemplate = {
    unitId: norm,
    label: norm.includes("dray_p") ? "Dray-P Prospector" : "Dray-class bulk hauler",
    compartments: new Map(),
    exits: [
      { from: "airlock", to: "passage", forwardLabel: "Central Passage", forwardCommand: "aft", reverseLabel: "Airlock", reverseCommand: "fore" },
      { from: "passage", to: "crew", forwardLabel: "Crew Compartment", forwardCommand: "aft", reverseLabel: "Central Passage", reverseCommand: "fore" },
      { from: "passage", to: "bridge", forwardLabel: "Bridge", forwardCommand: "u", reverseLabel: "Central Passage", reverseCommand: "d" },
      { from: "passage", to: "hold", forwardLabel: "Cargo Hold", forwardCommand: "d", reverseLabel: "Central Passage", reverseCommand: "u" },
      { from: "hold", to: "engineering", forwardLabel: "Engineering Access", forwardCommand: "aft", reverseLabel: "Cargo Hold", reverseCommand: "fore" },
    ],
  };

  const bounds = getStandardDrayCompartmentBounds();
  const labels: Record<string, { name: string; shortDesc: string }> = {
    airlock: { name: "Airlock", shortDesc: "Two-person lock, scuffed white, suit locker & outer seal." },
    passage: { name: "Central Passage", shortDesc: "Pressurised spine of grab-rails, cable trays & manifest slate." },
    crew: { name: "Crew Compartment", shortDesc: "Two curtained bunks, fold-down galley & water still." },
    bridge: { name: "Bridge", shortDesc: "Two crash couches before flat displays, holo-tank & Class 0 ROM." },
    hold: { name: "Cargo Hold", shortDesc: "100 t / 120 m³ bulk bay under blue work lights." },
    engineering: { name: "Engineering Access", shortDesc: "Reactor buses, coolant manifold & drive bulkhead." },
    bay: { name: "Vehicle Bay", shortDesc: "Stern ramp & cradles for Prospector Rigs 1 & 2." },
  };

  for (const [key, b] of Object.entries(bounds)) {
    if (key === "bay" && !norm.includes("dray_p")) continue;
    const meta = labels[key] ?? { name: key, shortDesc: "" };
    tpl.compartments.set(key, {
      id: key,
      name: meta.name,
      shortDesc: meta.shortDesc,
      desc: meta.shortDesc,
      localCenter: { ...b.center },
      localSize: { ...b.size },
      details: {},
    });
  }

  if (tpl.compartments.has("bay")) {
    tpl.exits.push({
      from: "hold",
      to: "bay",
      forwardLabel: "Vehicle Bay",
      forwardCommand: "+tv/bay in",
      reverseLabel: "Cargo Hold",
      reverseCommand: "+tv/bay out",
    });
  }
  return tpl;
}

export type QuickActionItem = {
  label: string;
  action: string;
  accent?: boolean;
};

export function contextualQuickActions(
  roomEntryOrComp: string | null,
  isAboardShip: boolean,
  isSeatedAtHelm: boolean,
): QuickActionItem[] {
  if (isSeatedAtHelm) {
    return [
      { label: "Stand From Helm", action: "ui:stand_helm", accent: true },
      { label: "Undock", action: "ui:open_sailing" },
      { label: "Dock Collar", action: "+helm/dock ceres" },
      { label: "Mine Ping", action: "+mine/ping" },
      { label: "Ship Status", action: "+ship" },
    ];
  }
  if (isAboardShip) {
    const comp = (roomEntryOrComp ?? "").toLowerCase();
    if (comp.includes("bridge")) {
      return [
        { label: "Take Bridge Helm", action: "ui:sit_helm", accent: true },
        { label: "Sailing Plan", action: "ui:open_sailing" },
        { label: "Ship Status", action: "+ship" },
        { label: "Hold Manifest", action: "+ship/hold" },
      ];
    }
    if (comp.includes("airlock")) {
      return [
        { label: "Disembark Ashore", action: "+ship/leave", accent: true },
        { label: "Ship Status", action: "+ship" },
        { label: "Sailing Plan", action: "ui:open_sailing" },
      ];
    }
    if (comp.includes("bay")) {
      return [
        { label: "Launch Rig 1", action: "+tv/1 launch go", accent: true },
        { label: "Lease Rig", action: "+tv/lease go" },
        { label: "Back to Hold", action: "+tv/bay out" },
      ];
    }
    return [
      { label: "Ship Status", action: "+ship" },
      { label: "Hold Manifest", action: "+ship/hold" },
      { label: "Sailing Plan", action: "ui:open_sailing" },
    ];
  }

  const entry = (roomEntryOrComp ?? "").toLowerCase();
  if (entry.includes("registry")) {
    return [
      { label: "Enrol Charter Hull", action: "ui:open_coop", accent: true },
      { label: "Co-op Terms", action: "+coop" },
      { label: "Credits", action: "+credit" },
    ];
  }
  if (entry.includes("exchange") || entry.includes("chandlery")) {
    return [
      { label: "Port Exchange", action: "ui:open_trade", accent: true },
      { label: "Sailing Plan", action: "ui:open_sailing" },
      { label: "Credits", action: "+credit" },
    ];
  }
  if (entry.includes("dockmaster")) {
    return [
      { label: "Sailing & Route Plan", action: "ui:open_sailing", accent: true },
      { label: "Port Exchange", action: "ui:open_trade" },
      { label: "Ship Status", action: "+ship" },
    ];
  }
  if (entry.includes("berth_gallery")) {
    return [
      { label: "Board Ship", action: "+ship/board", accent: true },
      { label: "Ship Status", action: "+ship" },
      { label: "Sailing Plan", action: "ui:open_sailing" },
    ];
  }
  if (entry.includes("simulator_lobby") || entry.includes("sim_red") || entry.includes("sim_blue")) {
    return [
      { label: "Simulator Console", action: "ui:open_sim", accent: true },
      { label: "Board Pod", action: "board" },
      { label: "Match Status", action: "+sim" },
    ];
  }
  return [
    { label: "Port Exchange", action: "ui:open_trade" },
    { label: "Co-op Charter", action: "ui:open_coop" },
    { label: "Ship Status", action: "+ship" },
    { label: "Credits", action: "+credit" },
  ];
}
