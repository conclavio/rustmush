// DOM-free state engine, undo/redo history, and repository export helpers for
// the staff Content Studio (doc 34 cut CS.0a, D-34-1, D-34-2, D-34-5, D-34-6).
import type { JsonValue } from "../proto";

export type StudioSubTab = "npcs" | "quests" | "components" | "corridor";

export interface PortCatalogEntry {
  id: string;
  venue: string;
  location: string;
  area: string;
  rooms: Record<string, string>;
  orgs: string[];
  vendors: string[];
}

export interface RoomCatalogEntry {
  id: string;
  name: string;
  zone: string;
  tags: string[];
  details: string[];
}

export interface StationCatalogEntry {
  pack_id: string;
  version: number;
  latched_version: number | null;
  rooms: RoomCatalogEntry[];
}

export interface NpcCatalogEntry {
  pack_id: string;
  entry_id: string;
  name: string;
  room_pack: string;
  room: string;
  tier: string;
  role: "station" | "crew" | "passenger";
  interfaces: string[];
  command_refs: string[];
  tags: string[];
  quest_count: number;
  code_bound: boolean;
  code_bindings: string[];
  latched: boolean;
}

export interface LorePoolCatalogEntry {
  pack_id: string;
  entry_id: string;
  tag: string;
  entry_count: number;
}

export interface GoodCatalogEntry {
  id: string;
  label: string;
  family: string | null;
  class: string;
  ledger_unit: string;
  quote_unit: string;
  stowage: string;
  measures: Record<string, number>;
  reference_major: number;
}

export interface PackSummary {
  id: string;
  repo_path: string;
  family: StudioSubTab | string;
  kind: string;
  version: number;
  entry_count: number;
  latched_version: number | null;
  retired: boolean;
}

export interface CatalogData {
  ports: PortCatalogEntry[];
  stations: StationCatalogEntry[];
  npcs: NpcCatalogEntry[];
  lore_pools: LorePoolCatalogEntry[];
  goods: GoodCatalogEntry[];
  interfaces: string[];
  command_refs: string[];
  packs: PackSummary[];
}

export function emptyCatalog(): CatalogData {
  return {
    ports: [],
    stations: [],
    npcs: [],
    lore_pools: [],
    goods: [],
    interfaces: ["hire", "passenger", "quest", "help"],
    command_refs: ["+crew", "+passage", "+ship", "+quest", "+market", "+coop"],
    packs: [],
  };
}

export function parseCatalog(raw: JsonValue): CatalogData {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) {
    return emptyCatalog();
  }
  const obj = raw as Record<string, unknown>;
  return {
    ports: Array.isArray(obj.ports) ? (obj.ports as PortCatalogEntry[]) : [],
    stations: Array.isArray(obj.stations) ? (obj.stations as StationCatalogEntry[]) : [],
    npcs: Array.isArray(obj.npcs) ? (obj.npcs as NpcCatalogEntry[]) : [],
    lore_pools: Array.isArray(obj.lore_pools) ? (obj.lore_pools as LorePoolCatalogEntry[]) : [],
    goods: Array.isArray(obj.goods) ? (obj.goods as GoodCatalogEntry[]) : [],
    interfaces: Array.isArray(obj.interfaces) ? (obj.interfaces as string[]) : [],
    command_refs: Array.isArray(obj.command_refs) ? (obj.command_refs as string[]) : [],
    packs: Array.isArray(obj.packs) ? (obj.packs as PackSummary[]) : [],
  };
}

/** Bounded undo/redo stack for studio document edits. */
export class DocumentHistory<T> {
  #undo: string[] = [];
  #redo: string[] = [];
  readonly #limit: number;

  constructor(limit = 50) {
    this.#limit = limit;
  }

  record(snapshot: T): void {
    const encoded = JSON.stringify(snapshot);
    if (this.#undo[this.#undo.length - 1] === encoded) return;
    this.#undo.push(encoded);
    if (this.#undo.length > this.#limit) {
      this.#undo.shift();
    }
    this.#redo = [];
  }

  undo(current: T): T | null {
    const prev = this.#undo.pop();
    if (prev === undefined) return null;
    this.#redo.push(JSON.stringify(current));
    return JSON.parse(prev) as T;
  }

  redo(current: T): T | null {
    const next = this.#redo.pop();
    if (next === undefined) return null;
    this.#undo.push(JSON.stringify(current));
    return JSON.parse(next) as T;
  }

  get canUndo(): boolean {
    return this.#undo.length > 0;
  }

  get canRedo(): boolean {
    return this.#redo.length > 0;
  }

  clear(): void {
    this.#undo = [];
    this.#redo = [];
  }
}

/**
 * Compute the historical snapshot path when exporting a version-bumped pack for
 * a live world (D-34-6: historical `-vN.json` / `_vN.json` files are preserved
 * for boot-replay tests).
 */
export function historicalSnapshotPath(repoPath: string, oldVersion: number): string {
  const base = repoPath.endsWith(".json") ? repoPath.slice(0, -5) : repoPath;
  const separator = repoPath.startsWith("passengers/") ? "_v" : "-v";
  return `${base}${separator}${oldVersion}.json`;
}

/**
 * Generate the 3-line Rust registration snippet when a staffer authors a brand-new
 * pack file rather than adding entries to an existing compiled-in pack (D-34-5).
 */
export function newPackRegistrationSnippet(packId: string, repoPath: string): string {
  const constName = packId
    .replace(/[^a-zA-Z0-9]+/g, "_")
    .toUpperCase();
  return [
    `// D-34-5: Register new compiled-in pack \`${packId}\` (\`crates/tacsim/content/${repoPath}\`):`,
    `pub const ${constName}_PACK: &str = "${packId}";`,
    `pub const ${constName}_JSON: &str = include_str!("../../content/${repoPath}");`,
    `// And bless test digest pins with: TACSIM_WRITE_CONTENT=1 cargo test -p tacsim`,
  ].join("\n");
}
