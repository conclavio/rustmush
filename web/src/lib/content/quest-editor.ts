// Pure data model, validation bounds, DAG builder, and JSON patchers for
// Cut CS.2 — Quest Studio (doc 34 §4, D-34-8, D-34-9).
import type { JsonValue } from "../proto";
import { requiredNeighborhoodVersion, utf8ByteLength } from "./npc-editor";

export { utf8ByteLength };

export const QUEST_BOUNDS = {
  maxQuestsPerNpc: 8,
  maxCheckpoints: 8,
  maxTitleBytes: 80,
  maxDestinationBytes: 80,
  maxIntroductionBytes: 600,
  maxCompletionBytes: 600,
  maxCheckpointIdBytes: 32,
  maxCheckpointTitleBytes: 80,
  maxInstructionsBytes: 500,
} as const;

export const OBJECTIVE_KINDS = [
  "observation",
  "coop_enrolment",
  "survey_commission",
  "trade",
  "passenger",
  "contact",
  "paid_passage",
  "survey",
] as const;

export type ObjectiveKind = (typeof OBJECTIVE_KINDS)[number];

export interface QuestObjectiveModel {
  kind: ObjectiveKind;
  origin: string;
  destination: string;
  commodity: string;
  quantity: number;
  npcPack: string;
  npc: string;
  mode: "scan" | "sound";
}

export interface QuestCheckpointModel {
  id: string;
  title: string;
  roomPack: string;
  roomEntry: string;
  detail: string;
  instructions: string;
  objective: QuestObjectiveModel;
}

export interface QuestEditorModel {
  entryId: string;
  npc: string;
  npcPack: string;
  prerequisiteKind: "open" | "completed";
  prerequisiteQuest: string;
  choiceGroup: string;
  choicePath: string;
  title: string;
  destination: string;
  introduction: string;
  completion: string;
  checkpoints: QuestCheckpointModel[];
}

export interface QuestDagEdge {
  from: string;
  to: string;
  crossPack: boolean;
  choiceGroup: string | null;
}

function asRecord(val: unknown): Record<string, unknown> {
  return val && typeof val === "object" && !Array.isArray(val)
    ? (val as Record<string, unknown>)
    : {};
}

function parseObjective(raw: unknown): QuestObjectiveModel {
  const obj = asRecord(raw);
  const kindRaw = typeof obj.kind === "string" ? obj.kind : "observation";
  const kind: ObjectiveKind = (OBJECTIVE_KINDS as readonly string[]).includes(
    kindRaw,
  )
    ? (kindRaw as ObjectiveKind)
    : "observation";
  return {
    kind,
    origin: typeof obj.origin === "string" ? obj.origin : "stickney",
    destination: typeof obj.destination === "string" ? obj.destination : "ceres",
    commodity:
      typeof obj.commodity === "string" ? obj.commodity : "industrial_parts",
    quantity: typeof obj.quantity === "number" ? obj.quantity : 1,
    npcPack:
      typeof obj.npc_pack === "string"
        ? obj.npc_pack
        : "neighborhood.ceres_freeport",
    npc: typeof obj.npc === "string" ? obj.npc : "npc.akintola",
    mode: obj.mode === "sound" ? "sound" : "scan",
  };
}

function serializeObjective(
  obj: QuestObjectiveModel,
): Record<string, unknown> | undefined {
  switch (obj.kind) {
    case "observation":
      return undefined; // default when `detail` is set
    case "coop_enrolment":
      return { kind: "coop_enrolment" };
    case "survey_commission":
      return { kind: "survey_commission" };
    case "trade":
      return {
        commodity: obj.commodity,
        destination: obj.destination,
        kind: "trade",
        origin: obj.origin,
        quantity: obj.quantity,
      };
    case "passenger":
      return {
        kind: "passenger",
        npc: obj.npc,
        npc_pack: obj.npcPack,
      };
    case "contact":
      return {
        kind: "contact",
        npc: obj.npc,
        npc_pack: obj.npcPack,
      };
    case "paid_passage":
      return {
        destination: obj.destination,
        kind: "paid_passage",
        npc: obj.npc,
        npc_pack: obj.npcPack,
        origin: obj.origin,
      };
    case "survey":
      return {
        kind: "survey",
        mode: obj.mode,
      };
  }
}

/** Extract all `quest.*` entries from a pack JSON document. */
export function extractQuestModels(doc: JsonValue | null): QuestEditorModel[] {
  const root = asRecord(doc);
  const entries = asRecord(root.entries);
  const out: QuestEditorModel[] = [];

  for (const [entryId, rawEntry] of Object.entries(entries)) {
    const entry = asRecord(rawEntry);
    if (entry.kind !== "quest") continue;
    const def = asRecord(entry.def);

    let prerequisiteKind: "open" | "completed" = "open";
    let prerequisiteQuest = "";
    if (typeof def.prerequisite === "object" && def.prerequisite !== null) {
      const preObj = asRecord(def.prerequisite);
      const completed = asRecord(preObj.completed);
      if (typeof completed.quest === "string") {
        prerequisiteKind = "completed";
        prerequisiteQuest = completed.quest;
      }
    }

    const choiceObj = asRecord(def.choice);
    const rawCheckpoints = Array.isArray(def.checkpoints) ? def.checkpoints : [];
    const checkpoints: QuestCheckpointModel[] = rawCheckpoints.map((cp, idx) => {
      const c = asRecord(cp);
      return {
        id: typeof c.id === "string" ? c.id : `step_${idx + 1}`,
        title: typeof c.title === "string" ? c.title : "",
        roomPack:
          typeof c.room_pack === "string"
            ? c.room_pack
            : "neighborhood.ceres_freeport",
        roomEntry:
          typeof c.room_entry === "string" ? c.room_entry : "room.c05_registry",
        detail: typeof c.detail === "string" ? c.detail : "",
        instructions:
          typeof c.instructions === "string" ? c.instructions : "",
        objective: parseObjective(c.objective),
      };
    });

    out.push({
      entryId,
      npc: typeof def.npc === "string" ? def.npc : "npc.oduya",
      npcPack:
        typeof def.npc_pack === "string"
          ? def.npc_pack
          : "neighborhood.ceres_freeport",
      prerequisiteKind,
      prerequisiteQuest,
      choiceGroup: typeof choiceObj.group === "string" ? choiceObj.group : "",
      choicePath: typeof choiceObj.path === "string" ? choiceObj.path : "",
      title: typeof def.title === "string" ? def.title : entryId,
      destination:
        typeof def.destination === "string" ? def.destination : "ceres",
      introduction:
        typeof def.introduction === "string" ? def.introduction : "",
      completion: typeof def.completion === "string" ? def.completion : "",
      checkpoints,
    });
  }
  return out;
}

/** Build the intra-pack and cross-pack prerequisite/choice DAG edges for visualization. */
export function buildQuestDagEdges(
  packId: string,
  quests: QuestEditorModel[],
): QuestDagEdge[] {
  const edges: QuestDagEdge[] = [];
  for (const q of quests) {
    if (q.prerequisiteKind === "completed" && q.prerequisiteQuest.trim()) {
      const ref = q.prerequisiteQuest.trim();
      const crossPack = ref.includes("/") && !ref.startsWith(`${packId}/`);
      edges.push({
        from: ref,
        to: `${packId}/${q.entryId}`,
        crossPack,
        choiceGroup: q.choiceGroup.trim() || null,
      });
    }
  }
  return edges;
}

function inferPackVersion(packId: string): number {
  if (packId.startsWith("neighborhood.")) {
    return requiredNeighborhoodVersion(packId);
  }
  if (packId === "tacsim.passengers.stickney") {
    return 2;
  }
  return 1;
}

/** Write an updated `QuestEditorModel` back into the pack JSON document and reconcile `manifest.requires`. */
export function applyQuestModelToDoc(
  doc: JsonValue,
  model: QuestEditorModel,
): JsonValue {
  const clone = JSON.parse(JSON.stringify(doc)) as Record<string, unknown>;
  const entries = asRecord(clone.entries);
  clone.entries = entries;
  const manifest = asRecord(clone.manifest);
  clone.manifest = manifest;
  const selfPackId = typeof manifest.id === "string" ? manifest.id : "";

  const questDef: Record<string, unknown> = {
    checkpoints: model.checkpoints.map((cp) => {
      const outCp: Record<string, unknown> = {
        id: cp.id,
        instructions: cp.instructions,
        room_entry: cp.roomEntry,
        room_pack: cp.roomPack,
        title: cp.title,
      };
      if (cp.objective.kind === "observation") {
        outCp.detail = cp.detail;
      } else {
        const obj = serializeObjective(cp.objective);
        if (obj) outCp.objective = obj;
      }
      return outCp;
    }),
    completion: model.completion,
    destination: model.destination,
    introduction: model.introduction,
    npc: model.npc,
    prerequisite:
      model.prerequisiteKind === "completed" && model.prerequisiteQuest.trim()
        ? { completed: { quest: model.prerequisiteQuest.trim() } }
        : "open",
    title: model.title,
  };
  if (model.npcPack.trim() && model.npcPack.trim() !== selfPackId) {
    questDef.npc_pack = model.npcPack.trim();
  }
  if (model.choiceGroup.trim() && model.choicePath.trim()) {
    questDef.choice = {
      group: model.choiceGroup.trim(),
      path: model.choicePath.trim(),
    };
  }

  entries[model.entryId] = {
    def: questDef,
    kind: "quest",
  };

  // Reconcile `manifest.requires` across all quest entries in the pack
  const reqMap = new Map<string, number>();
  for (const raw of Object.values(entries)) {
    const e = asRecord(raw);
    const d = asRecord(e.def);
    if (e.kind === "npc" && typeof d.room_pack === "string" && d.room_pack !== selfPackId) {
      reqMap.set(d.room_pack, inferPackVersion(d.room_pack));
    }
    if (e.kind !== "quest") continue;
    if (typeof d.npc_pack === "string" && d.npc_pack && d.npc_pack !== selfPackId) {
      reqMap.set(d.npc_pack, inferPackVersion(d.npc_pack));
    }
    const pre = asRecord(d.prerequisite);
    const completed = asRecord(pre.completed);
    if (typeof completed.quest === "string" && completed.quest.includes("/")) {
      const [depPack] = completed.quest.split("/");
      if (depPack && depPack !== selfPackId) {
        reqMap.set(depPack, inferPackVersion(depPack));
      }
    }
    const cps = Array.isArray(d.checkpoints) ? d.checkpoints : [];
    for (const cp of cps) {
      const c = asRecord(cp);
      if (typeof c.room_pack === "string" && c.room_pack && c.room_pack !== selfPackId) {
        reqMap.set(c.room_pack, inferPackVersion(c.room_pack));
      }
      const obj = asRecord(c.objective);
      if (
        typeof obj.npc_pack === "string" &&
        obj.npc_pack &&
        obj.npc_pack !== selfPackId
      ) {
        reqMap.set(obj.npc_pack, inferPackVersion(obj.npc_pack));
      }
    }
  }
  manifest.requires = Array.from(reqMap.entries())
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([id, version]) => ({ id, version }));

  return clone as JsonValue;
}

/** Merge a Quest LLM Assist patch into a `QuestEditorModel`. */
export function applyAssistPatchToQuest(
  model: QuestEditorModel,
  scope: string,
  patch: JsonValue,
): QuestEditorModel {
  const p = asRecord(patch);
  const next: QuestEditorModel = { ...model };
  if (scope === "introduction" && typeof p.introduction === "string") {
    next.introduction = p.introduction;
    return next;
  }
  if (scope === "completion" && typeof p.completion === "string") {
    next.completion = p.completion;
    return next;
  }
  if (
    scope === "checkpoint_instructions" &&
    typeof p.instructions === "string" &&
    next.checkpoints[0]
  ) {
    next.checkpoints = [
      { ...next.checkpoints[0], instructions: p.instructions },
      ...next.checkpoints.slice(1),
    ];
    return next;
  }
  if (typeof p.title === "string") next.title = p.title;
  if (typeof p.destination === "string") next.destination = p.destination;
  if (typeof p.introduction === "string") next.introduction = p.introduction;
  if (typeof p.completion === "string") next.completion = p.completion;
  return next;
}
