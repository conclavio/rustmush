// Pure data model, validation bounds, port-cascade helpers, and JSON patchers
// for Cut CS.1 — NPC Studio (Station Cast, Crew, Passengers, and Lore Pools)
// (doc 34 §3, D-34-3, D-34-4, D-34-8, D-34-9).
import type { JsonValue } from "../proto";
import type { CatalogData } from "./store";

export type NpcRoleFilter = "all" | "station" | "crew" | "passenger" | "lore";

export const CREW_DUTIES = ["mining", "combat", "recovery"] as const;
export type CrewDuty = (typeof CREW_DUTIES)[number];

export const NPC_BOUNDS = {
  maxNameBytes: 64,
  maxPersonaBytes: 8 * 1024,
  maxBackgroundBytes: 32 * 1024,
  maxSpeechBytes: 4 * 1024,
  maxListItems: 64,
  maxFallbackItems: 32,
  maxWatchItems: 128,
  maxItemBytes: 2 * 1024,
  maxPassengerTextBytes: 500,
} as const;

export interface LoreFactItem {
  tier: "public" | "guarded" | "secret";
  subject: string;
  fact: string;
  source: string;
}

export interface CrewTermsModel {
  schema: 1;
  npc: string;
  port: string;
  duties: CrewDuty[];
  fee_minor: number;
  daily_wage_minor: number;
}

export interface PassengerTermsModel {
  npc: string;
  origin: string;
  destination: string;
  fare_minor: number;
  deadline_days: number;
  opening: string;
  settled: string;
}

export interface NpcEditorModel {
  slug: string;
  entryId: string;
  name: string;
  tier: "major" | "minor" | "extra";
  roomPack: string;
  room: string;
  enabled: boolean;
  cooldownS: number | null;
  dailyTokens: number | null;
  persona: string;
  background: string;
  speechStyle: string;
  goals: string[];
  knows: string[];
  doesNotKnow: string[];
  fallback: string[];
  watch: string[];
  interfaces: string[];
  commandRefs: string[];
  tags: string[];
  lore: LoreFactItem[];
  role: "station" | "crew" | "passenger";
  crewTerms: CrewTermsModel | null;
  passengerTerms: PassengerTermsModel | null;
}

export interface LorePoolEditorModel {
  entryId: string;
  tag: string;
  entries: LoreFactItem[];
}

export function utf8ByteLength(text: string): number {
  return new TextEncoder().encode(text).length;
}

export function requiredNeighborhoodVersion(roomPack: string): number {
  if (
    roomPack === "neighborhood.ceres_freeport" ||
    roomPack === "neighborhood.stickney_yards"
  ) {
    return 4;
  }
  return 1;
}

function asRecord(val: unknown): Record<string, unknown> {
  return val && typeof val === "object" && !Array.isArray(val)
    ? (val as Record<string, unknown>)
    : {};
}

function asStringArray(val: unknown): string[] {
  return Array.isArray(val) ? val.filter((v): v is string => typeof v === "string") : [];
}

function parseLoreArray(val: unknown): LoreFactItem[] {
  if (!Array.isArray(val)) return [];
  return val.map((item) => {
    const obj = asRecord(item);
    const tier =
      obj.tier === "guarded" || obj.tier === "secret" ? obj.tier : "public";
    return {
      tier,
      subject: typeof obj.subject === "string" ? obj.subject : "",
      fact: typeof obj.fact === "string" ? obj.fact : "",
      source: typeof obj.source === "string" ? obj.source : "",
    };
  });
}

/** Extract all NPC models (paired with crew/passenger companion entries) from a pack JSON document. */
export function extractNpcModels(doc: JsonValue | null): NpcEditorModel[] {
  const root = asRecord(doc);
  const entries = asRecord(root.entries);
  const manifest = asRecord(root.manifest);
  const packId = typeof manifest.id === "string" ? manifest.id : "";

  const crewByNpc = new Map<string, CrewTermsModel>();
  const passengerByNpc = new Map<string, PassengerTermsModel>();

  for (const [, rawEntry] of Object.entries(entries)) {
    const entry = asRecord(rawEntry);
    const kind = entry.kind;
    const def = asRecord(entry.def);
    if (kind === "crew_for_hire" && typeof def.npc === "string") {
      const rawDuties = asStringArray(def.duties).filter((d): d is CrewDuty =>
        (CREW_DUTIES as readonly string[]).includes(d),
      );
      crewByNpc.set(def.npc, {
        schema: 1,
        npc: def.npc,
        port: typeof def.port === "string" ? def.port : "ceres_port",
        duties: rawDuties.length > 0 ? rawDuties : ["recovery"],
        fee_minor: typeof def.fee_minor === "number" ? def.fee_minor : 50_000,
        daily_wage_minor:
          typeof def.daily_wage_minor === "number" ? def.daily_wage_minor : 2_500,
      });
    } else if (kind === "passenger" && typeof def.npc === "string") {
      passengerByNpc.set(def.npc, {
        npc: def.npc,
        origin: typeof def.origin === "string" ? def.origin : "ceres_port",
        destination:
          typeof def.destination === "string" ? def.destination : "stickney_port",
        fare_minor: typeof def.fare_minor === "number" ? def.fare_minor : 40_000,
        deadline_days:
          typeof def.deadline_days === "number" ? def.deadline_days : 14,
        opening: typeof def.opening === "string" ? def.opening : "",
        settled: typeof def.settled === "string" ? def.settled : "",
      });
    }
  }

  const out: NpcEditorModel[] = [];
  for (const [entryId, rawEntry] of Object.entries(entries)) {
    const entry = asRecord(rawEntry);
    if (entry.kind !== "npc") continue;
    const def = asRecord(entry.def);
    const profile = asRecord(def.profile);
    const slug = entryId.startsWith("npc.") ? entryId.slice(4) : entryId;
    const crewTerms = crewByNpc.get(entryId) ?? null;
    const passengerTerms = passengerByNpc.get(entryId) ?? null;
    const role: "station" | "crew" | "passenger" = crewTerms
      ? "crew"
      : passengerTerms
        ? "passenger"
        : "station";
    const defaultRoomPack = packId.startsWith("tacsim.cast.")
      ? `neighborhood.${packId.slice("tacsim.cast.".length)}`
      : packId.startsWith("neighborhood.")
        ? packId
        : "neighborhood.ceres_freeport";

    const tierRaw = def.tier;
    const tier: "major" | "minor" | "extra" =
      tierRaw === "major" || tierRaw === "extra" ? tierRaw : "minor";

    out.push({
      slug,
      entryId,
      name: typeof def.name === "string" ? def.name : slug,
      tier,
      roomPack: typeof def.room_pack === "string" ? def.room_pack : defaultRoomPack,
      room: typeof def.room === "string" ? def.room : "room.c04_concourse",
      enabled: profile.enabled !== false,
      cooldownS: typeof profile.cooldown_s === "number" ? profile.cooldown_s : null,
      dailyTokens:
        typeof profile.daily_tokens === "number" ? profile.daily_tokens : null,
      persona: typeof profile.persona === "string" ? profile.persona : "",
      background: typeof profile.background === "string" ? profile.background : "",
      speechStyle:
        typeof profile.speech_style === "string" ? profile.speech_style : "",
      goals: asStringArray(profile.goals),
      knows: asStringArray(profile.knows),
      doesNotKnow: asStringArray(profile.does_not_know),
      fallback: asStringArray(profile.fallback),
      watch: asStringArray(profile.watch),
      interfaces: asStringArray(profile.interfaces),
      commandRefs: asStringArray(profile.command_refs),
      tags: asStringArray(def.tags),
      lore: parseLoreArray(def.lore),
      role,
      crewTerms,
      passengerTerms,
    });
  }
  return out;
}

/** Extract all shared `lore.*` pool entries from a pack JSON document. */
export function extractLorePools(doc: JsonValue | null): LorePoolEditorModel[] {
  const root = asRecord(doc);
  const entries = asRecord(root.entries);
  const out: LorePoolEditorModel[] = [];
  for (const [entryId, rawEntry] of Object.entries(entries)) {
    const entry = asRecord(rawEntry);
    if (entry.kind !== "lore") continue;
    const def = asRecord(entry.def);
    out.push({
      entryId,
      tag:
        typeof def.tag === "string"
          ? def.tag
          : entryId.startsWith("lore.")
            ? entryId.slice(5)
            : entryId,
      entries: parseLoreArray(def.entries),
    });
  }
  return out;
}

/**
 * Relocate an NPC model to a target corridor port (`portId`) using the live catalog:
 * updates `roomPack`, picks the first room in that station neighborhood (or keeps
 * the current room if already in that neighborhood), and updates `crewTerms.port`
 * or `passengerTerms.origin` (D-34-9).
 */
export function relocateNpcToPort(
  model: NpcEditorModel,
  portId: string,
  catalog: CatalogData,
): NpcEditorModel {
  const port = catalog.ports.find((p) => p.id === portId);
  const area = port?.area ?? "neighborhood.ceres_freeport";
  const station = catalog.stations.find((s) => s.pack_id === area);
  const firstRoom =
    station?.rooms[0]?.id ??
    Object.values(port?.rooms ?? {})[0] ??
    model.room;
  const nextRoom = model.roomPack === area ? model.room : firstRoom;

  const next: NpcEditorModel = {
    ...model,
    roomPack: area,
    room: nextRoom,
  };
  if (next.crewTerms) {
    next.crewTerms = {
      ...next.crewTerms,
      port: portId,
    };
  }
  if (next.passengerTerms) {
    const nextDest =
      next.passengerTerms.destination === portId
        ? portId === "ceres_port"
          ? "stickney_port"
          : "ceres_port"
        : next.passengerTerms.destination;
    next.passengerTerms = {
      ...next.passengerTerms,
      origin: portId,
      destination: nextDest,
    };
  }
  return next;
}

/** Write an updated `NpcEditorModel` back into the pack JSON document and reconcile `manifest.requires`. */
export function applyNpcModelToDoc(
  doc: JsonValue,
  model: NpcEditorModel,
): JsonValue {
  const clone = JSON.parse(JSON.stringify(doc)) as Record<string, unknown>;
  const entries = asRecord(clone.entries);
  clone.entries = entries;

  const profile: Record<string, unknown> = {};
  if (model.background.trim()) profile.background = model.background;
  if (model.commandRefs.length > 0) profile.command_refs = model.commandRefs;
  if (model.cooldownS !== null) profile.cooldown_s = model.cooldownS;
  if (model.dailyTokens !== null) profile.daily_tokens = model.dailyTokens;
  if (model.doesNotKnow.length > 0) profile.does_not_know = model.doesNotKnow;
  if (!model.enabled) profile.enabled = false;
  if (model.fallback.length > 0) profile.fallback = model.fallback;
  if (model.goals.length > 0) profile.goals = model.goals;
  if (model.interfaces.length > 0) profile.interfaces = model.interfaces;
  if (model.knows.length > 0) profile.knows = model.knows;
  if (model.persona.trim()) profile.persona = model.persona;
  if (model.speechStyle.trim()) profile.speech_style = model.speechStyle;
  if (model.watch.length > 0) profile.watch = model.watch;

  const npcDef: Record<string, unknown> = {
    name: model.name,
    profile,
    room: model.room,
    room_pack: model.roomPack,
    tier: model.tier,
  };
  if (model.lore.length > 0) {
    npcDef.lore = model.lore.map((l) => {
      const item: Record<string, unknown> = {
        fact: l.fact,
        subject: l.subject,
        tier: l.tier,
      };
      if (l.source.trim()) item.source = l.source;
      return item;
    });
  }
  if (model.tags.length > 0) {
    npcDef.tags = model.tags;
  }

  entries[model.entryId] = {
    def: npcDef,
    kind: "npc",
  };

  const crewKey = `crew_for_hire.${model.slug}`;
  const passengerKey = `passenger.${model.slug}`;
  if (model.role === "crew" && model.crewTerms) {
    delete entries[passengerKey];
    entries[crewKey] = {
      def: {
        daily_wage_minor: model.crewTerms.daily_wage_minor,
        duties: model.crewTerms.duties,
        fee_minor: model.crewTerms.fee_minor,
        npc: model.entryId,
        port: model.crewTerms.port,
        schema: 1,
      },
      kind: "crew_for_hire",
    };
  } else if (model.role === "passenger" && model.passengerTerms) {
    delete entries[crewKey];
    entries[passengerKey] = {
      def: {
        deadline_days: model.passengerTerms.deadline_days,
        destination: model.passengerTerms.destination,
        fare_minor: model.passengerTerms.fare_minor,
        npc: model.entryId,
        opening: model.passengerTerms.opening,
        origin: model.passengerTerms.origin,
        settled: model.passengerTerms.settled,
      },
      kind: "passenger",
    };
  } else {
    delete entries[crewKey];
    delete entries[passengerKey];
  }

  // Reconcile manifest.requires for every npc's room_pack in the pack
  const manifest = asRecord(clone.manifest);
  clone.manifest = manifest;
  const reqMap = new Map<string, number>();
  if (Array.isArray(manifest.requires)) {
    for (const r of manifest.requires) {
      const ro = asRecord(r);
      if (
        typeof ro.id === "string" &&
        typeof ro.version === "number" &&
        !ro.id.startsWith("neighborhood.")
      ) {
        reqMap.set(ro.id, ro.version);
      }
    }
  }
  for (const raw of Object.values(entries)) {
    const e = asRecord(raw);
    if (e.kind === "npc") {
      const d = asRecord(e.def);
      if (typeof d.room_pack === "string" && d.room_pack.trim()) {
        reqMap.set(d.room_pack, requiredNeighborhoodVersion(d.room_pack));
      }
    }
  }
  manifest.requires = Array.from(reqMap.entries())
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([id, version]) => ({ id, version }));

  return clone as JsonValue;
}

/** Write an updated `LorePoolEditorModel` back into the pack JSON document. */
export function applyLorePoolToDoc(
  doc: JsonValue,
  pool: LorePoolEditorModel,
): JsonValue {
  const clone = JSON.parse(JSON.stringify(doc)) as Record<string, unknown>;
  const entries = asRecord(clone.entries);
  clone.entries = entries;
  entries[pool.entryId] = {
    def: {
      entries: pool.entries.map((l) => {
        const item: Record<string, unknown> = {
          fact: l.fact,
          subject: l.subject,
          tier: l.tier,
        };
        if (l.source.trim()) item.source = l.source;
        return item;
      }),
      tag: pool.tag,
    },
    kind: "lore",
  };
  return clone as JsonValue;
}

/** Merge an LLM Assist `patch` response into an `NpcEditorModel` (`scope: "full"` or single field). */
export function applyAssistPatchToNpc(
  model: NpcEditorModel,
  scope: string,
  patch: JsonValue,
): NpcEditorModel {
  const p = asRecord(patch);
  const next: NpcEditorModel = { ...model };

  if (scope === "persona" && typeof p.persona === "string") {
    next.persona = p.persona;
    return next;
  }
  if (scope === "background" && typeof p.background === "string") {
    next.background = p.background;
    return next;
  }
  if (scope === "speech_style" && typeof p.speech_style === "string") {
    next.speechStyle = p.speech_style;
    return next;
  }
  if (scope === "goals" && Array.isArray(p.goals)) {
    next.goals = asStringArray(p.goals);
    return next;
  }
  if (scope === "knows" && Array.isArray(p.knows)) {
    next.knows = asStringArray(p.knows);
    return next;
  }
  if (scope === "does_not_know" && Array.isArray(p.does_not_know)) {
    next.doesNotKnow = asStringArray(p.does_not_know);
    return next;
  }
  if (scope === "fallback" && Array.isArray(p.fallback)) {
    next.fallback = asStringArray(p.fallback);
    return next;
  }

  // Full NPC draft synthesis (`scope: "full"`)
  const npcObj = asRecord(p.npc);
  const profileObj = asRecord(npcObj.profile);
  if (typeof npcObj.name === "string") next.name = npcObj.name;
  if (typeof npcObj.room_pack === "string") next.roomPack = npcObj.room_pack;
  if (typeof npcObj.room === "string") next.room = npcObj.room;
  if (
    npcObj.tier === "major" ||
    npcObj.tier === "minor" ||
    npcObj.tier === "extra"
  ) {
    next.tier = npcObj.tier;
  }
  if (Array.isArray(npcObj.tags)) next.tags = asStringArray(npcObj.tags);
  if (typeof profileObj.persona === "string") next.persona = profileObj.persona;
  if (typeof profileObj.background === "string")
    next.background = profileObj.background;
  if (typeof profileObj.speech_style === "string")
    next.speechStyle = profileObj.speech_style;
  if (Array.isArray(profileObj.goals))
    next.goals = asStringArray(profileObj.goals);
  if (Array.isArray(profileObj.knows))
    next.knows = asStringArray(profileObj.knows);
  if (Array.isArray(profileObj.does_not_know))
    next.doesNotKnow = asStringArray(profileObj.does_not_know);
  if (Array.isArray(profileObj.fallback))
    next.fallback = asStringArray(profileObj.fallback);
  if (Array.isArray(profileObj.interfaces))
    next.interfaces = asStringArray(profileObj.interfaces);
  if (Array.isArray(profileObj.command_refs))
    next.commandRefs = asStringArray(profileObj.command_refs);

  const termsObj = asRecord(p.terms);
  if (p.role === "crew" && Object.keys(termsObj).length > 0) {
    next.role = "crew";
    const duties = asStringArray(termsObj.duties).filter((d): d is CrewDuty =>
      (CREW_DUTIES as readonly string[]).includes(d),
    );
    next.crewTerms = {
      schema: 1,
      npc: next.entryId,
      port: typeof termsObj.port === "string" ? termsObj.port : "ceres_port",
      duties: duties.length > 0 ? duties : ["recovery"],
      fee_minor:
        typeof termsObj.fee_minor === "number" ? termsObj.fee_minor : 50_000,
      daily_wage_minor:
        typeof termsObj.daily_wage_minor === "number"
          ? termsObj.daily_wage_minor
          : 2_500,
    };
  } else if (p.role === "passenger" && Object.keys(termsObj).length > 0) {
    next.role = "passenger";
    next.passengerTerms = {
      npc: next.entryId,
      origin:
        typeof termsObj.origin === "string" ? termsObj.origin : "ceres_port",
      destination:
        typeof termsObj.destination === "string"
          ? termsObj.destination
          : "stickney_port",
      fare_minor:
        typeof termsObj.fare_minor === "number" ? termsObj.fare_minor : 40_000,
      deadline_days:
        typeof termsObj.deadline_days === "number"
          ? termsObj.deadline_days
          : 14,
      opening: typeof termsObj.opening === "string" ? termsObj.opening : "",
      settled: typeof termsObj.settled === "string" ? termsObj.settled : "",
    };
  }
  return next;
}
