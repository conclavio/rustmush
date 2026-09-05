// The room band's model (interface plan I4): `RoomView` plus its `ambient`
// map — weather, the IC clock, the scene glance — read defensively, since
// every ambient value is a module's own shape and may be absent. Pure.

import type { RoomView } from "./proto";

export interface RoomBandWeather {
  state: string;
  icon: string;
  temp: number | null;
  line: string | null;
}

export interface RoomBandClock {
  hour: number;
  minute: number;
  season: string | null;
  timeOfDay: string | null;
}

export interface RoomBandScene {
  title: string;
  status: string;
  round: number | null;
  turn: number | null;
}

export interface RoomBandExit {
  name: string;
  /** The one-letter nav key that walks it, when the room offers one. */
  key: string | null;
}

export interface RoomBandModel {
  name: string;
  id: string;
  zone: string | null;
  weather: RoomBandWeather | null;
  clock: RoomBandClock | null;
  scene: RoomBandScene | null;
  exits: RoomBandExit[];
}

function obj(v: unknown): Record<string, unknown> | null {
  return typeof v === "object" && v !== null && !Array.isArray(v)
    ? (v as Record<string, unknown>)
    : null;
}

function str(v: unknown): string | null {
  return typeof v === "string" && v.length > 0 ? v : null;
}

function num(v: unknown): number | null {
  return typeof v === "number" && Number.isFinite(v) ? v : null;
}

/** The band for `view`, or `null` before any room. */
export function roomBand(
  view: RoomView | null,
  exitKeys: ReadonlyMap<string, string>,
): RoomBandModel | null {
  if (!view) {
    return null;
  }
  const ambient = obj(view.ambient) ?? {};
  const wx = obj(ambient["weather"]);
  const weather: RoomBandWeather | null = wx
    ? {
        state: str(wx["state"]) ?? "",
        icon: str(wx["icon"]) ?? str(wx["state"]) ?? "",
        temp: num(wx["temp"]),
        line: str(wx["line"]),
      }
    : null;
  const ic = obj(ambient["ic_time"]);
  const hour = ic ? num(ic["hour"]) : null;
  const minute = ic ? num(ic["minute"]) : null;
  const clock: RoomBandClock | null =
    ic && hour !== null && minute !== null
      ? {
          hour,
          minute,
          season: str(ic["season"]),
          timeOfDay: str(ic["time_of_day"]),
        }
      : null;
  const sc = obj(ambient["scene"]);
  const scene: RoomBandScene | null = sc
    ? {
        title: str(sc["title"]) ?? "(untitled)",
        status: str(sc["status"]) ?? "",
        round: num(sc["round"]),
        turn: num(sc["turn"]),
      }
    : null;
  const exits: RoomBandExit[] = view.exits.map((e) => {
    let key: string | null = null;
    for (const [k, name] of exitKeys) {
      if (name.toLowerCase() === e.name.toLowerCase()) {
        key = k;
        break;
      }
    }
    return { name: e.name, key };
  });
  return {
    name: view.name,
    id: view.id,
    zone: view.zone?.name ?? null,
    weather,
    clock,
    scene,
    exits,
  };
}

/** `HH:MM` for a clock. */
export function clockText(clock: RoomBandClock): string {
  return `${String(clock.hour).padStart(2, "0")}:${String(clock.minute).padStart(2, "0")}`;
}
