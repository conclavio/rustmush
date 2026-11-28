// Three.js 3D Station & Ship Interior Renderer (`web/src/lib/tacsim/stage3d.ts`).
// Builds multi-deck stations (`ceres_freeport`, `stickney_yards`, `aitken_highport`,
// `ooc_area`) and walkable 3-deck ship interiors (`dray`, `dray_p`, `corvette`)
// with procedural PBR + normal maps, persistent actor pooling, and 1-in-flight BFS sync.

import * as THREE from "three";
import { isTextField } from "../keys";
import { tacsimAudio } from "./audio";
import {
  AMBER_RIG_RAMP,
  BLUE_TEAM_RAMP,
  createAsyncMupkUnitGroup,
  RED_TEAM_RAMP,
} from "./mupk-mesh";
import {
  areRoomsConnectedWithinHops,
  atlasToWorldPos,
  BRIDGE_STAIRWELL_XZ,
  classifyStationDesk,
  computeDoorwaysAndStairs,
  computeInterFloorStairPlacement,
  computePackGeometrySignature,
  computeRightHandedMoveDelta,
  computeShipTemplateSignature,
  findShipExitPath,
  findStationExitPath,
  getDefaultSpawnRoomEntry,
  getFloorElevationY,
  getFloorHeight,
  getPackBoundsCenter2D,
  HOLD_STAIRWELL_XZ,
  resolveCompartmentDef,
  RoomBoundNavWalker,
  tryFindContainingCompartment,
  tryFindContainingRoomWithHysteresis,
  worldToAtlas2D,
  type AtlasExitDef,
  type AtlasNeighborhoodPack,
  type AtlasRoomDef,
  type DecodedRoomAmbient,
  type DockedHullSummary,
  type HorizontalDoorDef,
  type RectXZ,
  type ShipCompartmentDef,
  type ShipUnitTemplate,
  type VerticalDoorDef,
  type VisibleRoomActor,
} from "./spatial";

export type InteractableKind3D =
  | "desk"
  | "npc"
  | "plaque"
  | "airlock_board"
  | "airlock_leave"
  | "helm_seat"
  | "sim_pod"
  | "vehicle_rig";

export interface StageQuestMarker {
  npcId: string;
  npcName?: string;
  marker: "turn_in" | "available" | "in_progress";
  symbol: "!" | "?";
  questKey?: string;
  questTitle?: string;
}

export function questMarkerPriority(marker: StageQuestMarker["marker"]): number {
  if (marker === "turn_in") return 0;
  if (marker === "available") return 1;
  return 2;
}

export function formatActorInteractPrompt(
  name: string,
  isNpc: boolean,
  shortDesc: string,
  questMarker: StageQuestMarker | null,
  extraQuestCount = 0,
): { title: string; subtitle: string } {
  const moreSuffix = extraQuestCount > 0 ? ` (+${extraQuestCount} more)` : "";
  if (questMarker) {
    if (questMarker.marker === "turn_in") {
      return {
        title: `[?] ${name} — Turn In Quest`,
        subtitle:
          (questMarker.questTitle
            ? `Complete: ${questMarker.questTitle}`
            : "Ready to turn in quest") + moreSuffix,
      };
    }
    if (questMarker.marker === "available") {
      return {
        title: `[!] ${name} — Quest Available`,
        subtitle:
          (questMarker.questTitle
            ? `Offer: ${questMarker.questTitle}`
            : "Optional quest available") + moreSuffix,
      };
    }
    return {
      title: `[?] Talk to ${name}`,
      subtitle:
        (questMarker.questTitle
          ? `Active quest: ${questMarker.questTitle}`
          : shortDesc || "Speak with station cast") + moreSuffix,
    };
  }
  return {
    title: isNpc ? `Talk to ${name}` : `Greet ${name}`,
    subtitle: isNpc ? shortDesc || "Speak with station cast" : "Greet pilot",
  };
}

export interface WorldInteractable3D {
  id: string;
  kind: InteractableKind3D;
  title: string;
  subtitle: string;
  actionOrKey: string;
  detailBody?: string;
  npcMeta?: {
    id: string;
    name: string;
    bio: string;
    isNpc: boolean;
    questMarker?: StageQuestMarker;
  };
  worldPos: THREE.Vector3;
  maxDistance: number;
  meshTargets: THREE.Object3D[];
}

export interface Stage3DCallbacks {
  onSendCommand: (cmd: string) => void;
  onInteractableTriggered: (wi: WorldInteractable3D) => void;
  onFocusedPromptChanged: (prompt: { title: string; subtitle: string } | null) => void;
  onPhysicalLocationChanged: (locKey: string | null) => void;
  isUiBlockingInput: () => boolean;
  /** Optional renderer factory seam for testing WebGL success/failure paths. */
  createRenderer?: (canvas: HTMLCanvasElement) => Pick<
    THREE.WebGLRenderer,
    "setPixelRatio" | "setSize" | "render" | "dispose" | "toneMapping" | "toneMappingExposure"
  >;
}

interface SpawnedActorRecord {
  id: string;
  name: string;
  isNpc: boolean;
  actor: VisibleRoomActor;
  group: THREE.Group;
  leftLeg: THREE.Object3D;
  rightLeg: THREE.Object3D;
  leftArm: THREE.Object3D;
  rightArm: THREE.Object3D;
  targetPos: THREE.Vector3;
  walkPhase: number;
  billboardSprite: THREE.Sprite;
  billboardTex: THREE.CanvasTexture;
  billboardCanvas: HTMLCanvasElement;
  lastRenderedLabel: string;
  speechExpiresAt: number;
  lastIsTyping: boolean;
  questMarker: StageQuestMarker | null;
  interactable: WorldInteractable3D;
  safeBounds: { minX: number; maxX: number; minZ: number; maxZ: number; margin: number };
}

interface StairSpan {
  minX: number;
  maxX: number;
  minZ: number;
  maxZ: number;
  pA: THREE.Vector3;
  pB: THREE.Vector3;
}

interface SharedPbrTextures {
  bulkheadMap: THREE.DataTexture;
  bulkheadNormal: THREE.DataTexture;
  deckMap: THREE.DataTexture;
  deckNormal: THREE.DataTexture;
  ceilingMap: THREE.DataTexture;
  rockMap: THREE.DataTexture;
}

let cachedTextures: SharedPbrTextures | null = null;
const sharedBaseTextures = new Set<THREE.Texture>();

function getOrBuildProceduralTextures(): SharedPbrTextures {
  if (cachedTextures) return cachedTextures;

  const S = 128;
  const makeTex = (data: Uint8Array, linear = false): THREE.DataTexture => {
    const tex = new THREE.DataTexture(data, S, S, THREE.RGBAFormat);
    tex.wrapS = THREE.RepeatWrapping;
    tex.wrapT = THREE.RepeatWrapping;
    tex.colorSpace = linear ? THREE.NoColorSpace : THREE.SRGBColorSpace;
    tex.needsUpdate = true;
    return tex;
  };

  const buildNormalFromHeight = (h: Float32Array, strength: number): THREE.DataTexture => {
    const nData = new Uint8Array(S * S * 4);
    for (let y = 0; y < S; y++) {
      const ym = (y - 1 + S) % S;
      const yp = (y + 1) % S;
      for (let x = 0; x < S; x++) {
        const xm = (x - 1 + S) % S;
        const xp = (x + 1) % S;
        const dx = (h[y * S + xp]! - h[y * S + xm]!) * strength;
        const dy = (h[yp * S + x]! - h[ym * S + x]!) * strength;
        const inv = 1 / Math.sqrt(dx * dx + dy * dy + 1);
        const idx = (y * S + x) * 4;
        nData[idx] = Math.round((-dx * inv * 0.5 + 0.5) * 255);
        nData[idx + 1] = Math.round((-dy * inv * 0.5 + 0.5) * 255);
        nData[idx + 2] = Math.round((inv * 0.5 + 0.5) * 255);
        nData[idx + 3] = 255;
      }
    }
    return makeTex(nData, true);
  };

  // 1. Ribbed Bulkhead
  const bhPx = new Uint8Array(S * S * 4);
  const bhH = new Float32Array(S * S);
  for (let y = 0; y < S; y++) {
    for (let x = 0; x < S; x++) {
      const bx = x % 64;
      const edgeX = Math.min(bx, 63 - bx);
      const kick = y < 18;
      const rail = y >= 18 && y <= 22;
      const seam = edgeX < 2 || y > S - 4;
      const recess = bx >= 8 && bx <= 55 && y >= 28 && y <= 108;
      let v = 132;
      let ht = 0.5;
      if (kick) { v = 86; ht = 0.65; }
      else if (rail) { v = 158; ht = 0.85; }
      else if (seam) { v = 78; ht = 0.15; }
      else if (recess) { v = 116 + ((y % 8 < 2) ? -6 : 4); ht = 0.32; }
      bhH[y * S + x] = ht;
      const idx = (y * S + x) * 4;
      bhPx[idx] = Math.round(v * 0.88);
      bhPx[idx + 1] = Math.round(v * 0.95);
      bhPx[idx + 2] = Math.min(255, Math.round(v * 1.06));
      bhPx[idx + 3] = 255;
    }
  }

  // 2. Nonskid Deck Tiles
  const dkPx = new Uint8Array(S * S * 4);
  const dkH = new Float32Array(S * S);
  for (let y = 0; y < S; y++) {
    for (let x = 0; x < S; x++) {
      const tx = x % 64;
      const ty = y % 64;
      const seam = tx < 2 || ty < 2 || tx > 61 || ty > 61;
      const tread = !seam && ((tx + ty) % 12 < 2 || (tx - ty + 128) % 12 < 2);
      let v = seam ? 74 : tread ? 128 : 112;
      dkH[y * S + x] = seam ? 0.15 : tread ? 0.68 : 0.45;
      const idx = (y * S + x) * 4;
      dkPx[idx] = Math.round(v * 0.86);
      dkPx[idx + 1] = Math.round(v * 0.94);
      dkPx[idx + 2] = Math.round(v * 1.05);
      dkPx[idx + 3] = 255;
    }
  }

  // 3. Matte Coffered Ceiling
  const clPx = new Uint8Array(S * S * 4);
  for (let y = 0; y < S; y++) {
    for (let x = 0; x < S; x++) {
      const tx = x % 64;
      const ty = y % 64;
      const rib = tx < 4 || ty < 4 || tx > 59 || ty > 59;
      const v = rib ? 105 : 82;
      const idx = (y * S + x) * 4;
      clPx[idx] = Math.round(v * 0.90);
      clPx[idx + 1] = Math.round(v * 0.96);
      clPx[idx + 2] = Math.round(v * 1.06);
      clPx[idx + 3] = 255;
    }
  }

  // 4. Stickney Basalt Rock-Cut Wall
  const rkPx = new Uint8Array(S * S * 4);
  for (let y = 0; y < S; y++) {
    for (let x = 0; x < S; x++) {
      const n = Math.sin(x * 0.21 + Math.cos(y * 0.17) * 2.4) * 18 + Math.cos(y * 0.31 - x * 0.11) * 12;
      const v = Math.max(55, Math.min(145, Math.round(96 + n)));
      const idx = (y * S + x) * 4;
      rkPx[idx] = Math.round(v * 0.96);
      rkPx[idx + 1] = Math.round(v * 0.88);
      rkPx[idx + 2] = Math.round(v * 0.82);
      rkPx[idx + 3] = 255;
    }
  }

  cachedTextures = {
    bulkheadMap: makeTex(bhPx),
    bulkheadNormal: buildNormalFromHeight(bhH, 2.2),
    deckMap: makeTex(dkPx),
    deckNormal: buildNormalFromHeight(dkH, 1.8),
    ceilingMap: makeTex(clPx),
    rockMap: makeTex(rkPx),
  };
  sharedBaseTextures.add(cachedTextures.bulkheadMap);
  sharedBaseTextures.add(cachedTextures.bulkheadNormal);
  sharedBaseTextures.add(cachedTextures.deckMap);
  sharedBaseTextures.add(cachedTextures.deckNormal);
  sharedBaseTextures.add(cachedTextures.ceilingMap);
  sharedBaseTextures.add(cachedTextures.rockMap);
  return cachedTextures;
}

function cloneRepeatTex(base: THREE.DataTexture, rx: number, ry: number): THREE.DataTexture {
  const c = base.clone();
  c.wrapS = THREE.RepeatWrapping;
  c.wrapT = THREE.RepeatWrapping;
  c.repeat.set(Math.max(1, Math.round(rx)), Math.max(1, Math.round(ry)));
  c.needsUpdate = true;
  return c;
}

export interface QuestWayfinderPath3D {
  targetRoomEntry: string;
  targetRoomName: string;
  hopCount: number;
  points: THREE.Vector3[];
}

/**
 * Pure helper computing a 3D world-space floor ribbon polyline from `fromRoomEntry`
 * to `toRoomEntry` along authoritative neighborhood BFS exits.
 */
export function computeQuestWayfinderPoints(
  pack: AtlasNeighborhoodPack,
  fromRoomEntry: string | null,
  toRoomEntry: string | null,
  originOffsetX: number,
  originOffsetZ: number,
  floorLiftM = 0.28,
): QuestWayfinderPath3D | null {
  if (!fromRoomEntry || !toRoomEntry || fromRoomEntry === toRoomEntry) return null;
  const fromRoom = pack.rooms.get(fromRoomEntry);
  const destRoom = pack.rooms.get(toRoomEntry);
  if (!fromRoom || !destRoom) return null;
  const hops = findStationExitPath(pack, fromRoomEntry, toRoomEntry);
  if (hops.length === 0) return null;

  const points: THREE.Vector3[] = [];
  const pushRoomCenter = (room: AtlasRoomDef): void => {
    const cx = (room.minX + room.maxX) * 0.5;
    const cy = (room.minY + room.maxY) * 0.5;
    const y = getFloorElevationY(pack, room.floorIndex) + floorLiftM;
    const raw = atlasToWorldPos(cx, y, cy, originOffsetX, originOffsetZ);
    const wp = new THREE.Vector3(raw.x, raw.y, raw.z);
    const prev = points[points.length - 1];
    if (!prev || prev.distanceTo(wp) > 0.2) {
      points.push(wp);
    }
  };

  pushRoomCenter(fromRoom);
  for (const hop of hops) {
    const r = pack.rooms.get(hop.targetRoom);
    if (r) pushRoomCenter(r);
  }
  if (points.length < 2) return null;

  return {
    targetRoomEntry: toRoomEntry,
    targetRoomName: destRoom.name || toRoomEntry,
    hopCount: hops.length,
    points,
  };
}

/**
 * Pure helper computing a 3D world-space floor ribbon polyline between ship interior
 * compartments along authoritative `ShipUnitTemplate` hatches.
 */
export function computeShipQuestWayfinderPoints(
  template: ShipUnitTemplate,
  fromCompId: string | null,
  toCompId: string | null,
  shipOrigin: THREE.Vector3,
  floorLiftM = 0.28,
): QuestWayfinderPath3D | null {
  if (!fromCompId || !toCompId) return null;
  const fromComp = resolveCompartmentDef(template, fromCompId);
  const resolvedToId = template.compartments.has(toCompId)
    ? toCompId
    : toCompId === "bay" && template.compartments.has("hold")
      ? "hold"
      : toCompId;
  const destComp = resolveCompartmentDef(template, resolvedToId);
  if (!fromComp || !destComp || fromComp.id === destComp.id) return null;
  const hops = findShipExitPath(template, fromComp.id, destComp.id);
  if (hops.length === 0) return null;

  const points: THREE.Vector3[] = [];
  const pushCompCenter = (comp: ShipCompartmentDef): void => {
    const wp = new THREE.Vector3(
      shipOrigin.x + comp.localCenter.x,
      shipOrigin.y + comp.localCenter.y - comp.localSize.y * 0.5 + floorLiftM,
      shipOrigin.z + comp.localCenter.z,
    );
    const prev = points[points.length - 1];
    if (!prev || prev.distanceTo(wp) > 0.2) {
      points.push(wp);
    }
  };

  pushCompCenter(fromComp);
  for (const hop of hops) {
    const c = resolveCompartmentDef(template, hop.targetCompartment);
    if (c) pushCompCenter(c);
  }
  if (points.length < 2) return null;

  return {
    targetRoomEntry: destComp.id,
    targetRoomName: destComp.name || destComp.id,
    hopCount: hops.length,
    points,
  };
}

export class TacsimStage3D {
  readonly supported: boolean;
  private readonly headlessDom: boolean;
  private readonly cb: Stage3DCallbacks;
  private readonly canvas: HTMLCanvasElement;
  private renderer: THREE.WebGLRenderer | null = null;
  private readonly scene = new THREE.Scene();
  private readonly camera = new THREE.PerspectiveCamera(58, 16 / 9, 0.15, 450);

  private readonly stationRoot = new THREE.Group();
  private readonly shipRoot = new THREE.Group();
  private readonly actorsRoot = new THREE.Group();
  private readonly questWayfinderRoot = new THREE.Group();
  private questWayfinderSig = "";
  private questWayfinderTargetRoom: string | null = null;
  private questWayfinderHopCount = 0;
  private readonly shipOrigin = new THREE.Vector3(0, 160, 0);

  private readonly playerGroup = new THREE.Group();
  private playerLeftLeg!: THREE.Object3D;
  private playerRightLeg!: THREE.Object3D;
  private playerLeftArm!: THREE.Object3D;
  private playerRightArm!: THREE.Object3D;
  private playerWalkPhase = 0;

  private camYawRad = 0;
  private camPitchRad = 0.34;
  private camDist = 5.4;
  private rmbOrbiting = false;
  private lastPointerX = 0;
  private lastPointerY = 0;

  private readonly keysDown = new Set<string>();
  private readonly roomLights = new Map<string, THREE.PointLight>();
  private readonly wallObstacles: THREE.Box3[] = [];
  private readonly shipWallObstacles: THREE.Box3[] = [];
  private readonly stationStairs: StairSpan[] = [];
  private readonly shipStairs: StairSpan[] = [];
  private readonly interactables: WorldInteractable3D[] = [];
  private readonly shipInteractables: WorldInteractable3D[] = [];
  private focusedInteractable: WorldInteractable3D | null = null;

  private static readonly HOLO_UNSET = Symbol("holo-unset");

  private holotankContactsGroup: THREE.Group | null = null;
  private holotankStatusBanner: {
    sprite: THREE.Sprite;
    canvas: HTMLCanvasElement;
    tex: THREE.CanvasTexture;
    lastText: string;
  } | null = null;
  private cargoPalletsGroup: THREE.Group | null = null;
  private readonly cargoCrateGeo = new THREE.BoxGeometry(1.4, 1.15, 1.4);
  private readonly cargoCrateMat = new THREE.MeshStandardMaterial({
    color: 0x4f6882,
    roughness: 0.6,
    metalness: 0.35,
  });
  private readonly holoBlipGeo = new THREE.SphereGeometry(1, 10, 8);
  private readonly holoBlipMats = new Map<number, THREE.MeshBasicMaterial>();
  private readonly sharedPoolGeos = new Set<THREE.BufferGeometry>();
  private readonly sharedPoolMats = new Set<THREE.Material>();
  private lastCargoStacks = -1;
  private lastHolotankFeed: unknown = TacsimStage3D.HOLO_UNSET;

  private cradleSignSprites: Array<{ sprite: THREE.Sprite; canvas: HTMLCanvasElement; tex: THREE.CanvasTexture }> = [];
  private cradleHullGroups: THREE.Group[] = [];
  private airlockBoardWi: WorldInteractable3D | null = null;
  private airlockLeaveWi: WorldInteractable3D | null = null;

  private readonly spawnedActors = new Map<string, SpawnedActorRecord>();
  private spawnedActorsScopeKey = "";

  private builtStationSig = "";
  private builtShipSig = "";
  private activeStationPack: AtlasNeighborhoodPack | null = null;
  private activeShipTemplate: ShipUnitTemplate | null = null;
  private originOffsetX = -160;
  private originOffsetZ = -150;

  private isAboardShip = false;
  private activeStationRoomEntry = "room.c03_arrivals";
  private serverStationRoomEntry: string | null = null;
  private activeShipCompartment = "airlock";
  private serverShipCompartment: string | null = null;
  private lastSyncedRoomRevision = -1;
  private awaitingInitialRoomSync = true;
  private readonly navWalker = new RoomBoundNavWalker();
  private nextMoveStepAllowedAtMs = 0;

  private get pendingMoveTarget(): string | null {
    return this.navWalker.inFlightTarget;
  }

  private set pendingMoveTarget(target: string | null) {
    const scope = this.isAboardShip ? "ship" : "station";
    const fromRoom =
      (this.isAboardShip ? this.serverShipCompartment : this.serverStationRoomEntry) ?? "";
    this.navWalker.setExternalInFlightTarget(scope, fromRoom, target, performance.now());
  }

  private get pendingMoveSentAtMs(): number {
    return this.navWalker.inFlightSentAtMs;
  }

  private set pendingMoveSentAtMs(sentAtMs: number) {
    this.navWalker.setExternalInFlightSentAtMs(sentAtMs);
  }

  // Reusable scratch objects for hot frame-loop math (eliminates per-frame GC allocations)
  private readonly tmpProbeBox = new THREE.Box3();
  private readonly tmpVecA = new THREE.Vector3();
  private readonly tmpVecB = new THREE.Vector3();
  private readonly tmpVecC = new THREE.Vector3();

  private rafId = 0;
  private lastFrameMs = 0;
  private eventsBound = false;
  private contextLost = false;
  private disposed = false;

  constructor(canvas: HTMLCanvasElement, callbacks: Stage3DCallbacks) {
    this.canvas = canvas;
    this.cb = callbacks;
    this.headlessDom = typeof navigator !== "undefined" && /jsdom/i.test(navigator.userAgent);

    this.sharedPoolGeos.add(this.cargoCrateGeo);
    this.sharedPoolGeos.add(this.holoBlipGeo);
    this.sharedPoolMats.add(this.cargoCrateMat);

    this.scene.background = new THREE.Color(0x06090f);
    this.scene.fog = new THREE.FogExp2(0x070b13, 0.0055);

    const amb = new THREE.AmbientLight(0x8aa4c4, 1.05);
    this.scene.add(amb);
    const hemi = new THREE.HemisphereLight(0xb8d4f4, 0x1e2634, 0.85);
    this.scene.add(hemi);

    this.stationRoot.name = "StationWorld";
    this.shipRoot.name = "ShipInterior";
    this.questWayfinderRoot.name = "QuestWayfinder3D";
    this.shipRoot.position.copy(this.shipOrigin);
    this.shipRoot.visible = false;
    this.scene.add(this.stationRoot);
    this.scene.add(this.shipRoot);
    this.scene.add(this.actorsRoot);
    this.scene.add(this.questWayfinderRoot);

    this.buildPlayerAvatar();
    this.scene.add(this.playerGroup);

    let glOk = false;
    if (callbacks.createRenderer || !this.headlessDom) {
      try {
        this.renderer = callbacks.createRenderer
          ? (callbacks.createRenderer(this.canvas) as THREE.WebGLRenderer)
          : new THREE.WebGLRenderer({
              canvas: this.canvas,
              antialias: true,
              powerPreference: "high-performance",
            });
        this.renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
        this.renderer.toneMapping = THREE.ACESFilmicToneMapping;
        this.renderer.toneMappingExposure = 1.12;
        glOk = true;
      } catch {
        this.renderer = null;
        glOk = false;
      }
    }
    this.supported = glOk;

    if (this.supported) {
      this.bindDomEvents();
      this.lastFrameMs = performance.now();
      this.rafId = requestAnimationFrame(this.onFrame);
    }
  }

  dispose(): void {
    this.disposed = true;
    if (this.rafId) {
      cancelAnimationFrame(this.rafId);
      this.rafId = 0;
    }
    if (this.eventsBound) {
      this.unbindDomEvents();
    }
    this.clearAllActors();
    this.clearAndDisposeGroup(this.questWayfinderRoot);
    this.clearAndDisposeGroup(this.stationRoot);
    this.clearAndDisposeGroup(this.shipRoot);
    this.clearAndDisposeGroup(this.playerGroup);
    this.cargoCrateGeo.dispose();
    this.cargoCrateMat.dispose();
    this.holoBlipGeo.dispose();
    for (const mat of this.holoBlipMats.values()) {
      mat.dispose();
    }
    this.holoBlipMats.clear();
    this.sharedPoolGeos.clear();
    this.sharedPoolMats.clear();
    this.renderer?.dispose();
    this.renderer = null;
  }

  private disposeSubtree(root: THREE.Object3D): void {
    const geos = new Set<THREE.BufferGeometry>();
    const mats = new Set<THREE.Material>();
    const texs = new Set<THREE.Texture>();

    const collectMaterialTextures = (mat: THREE.Material): void => {
      const m = mat as unknown as Record<string, unknown>;
      for (const prop of [
        "map",
        "normalMap",
        "roughnessMap",
        "metalnessMap",
        "emissiveMap",
        "alphaMap",
        "aoMap",
        "bumpMap",
      ]) {
        const val = m[prop];
        if (val && typeof (val as THREE.Texture).dispose === "function") {
          const t = val as THREE.Texture;
          if (!sharedBaseTextures.has(t)) texs.add(t);
        }
      }
    };

    root.traverse((obj) => {
      obj.userData["disposed"] = true;
      const mesh = obj as THREE.Mesh;
      if (
        mesh.geometry &&
        typeof mesh.geometry.dispose === "function" &&
        !this.sharedPoolGeos.has(mesh.geometry)
      ) {
        geos.add(mesh.geometry);
      }
      const mat = mesh.material;
      if (Array.isArray(mat)) {
        for (const m of mat) {
          if (m && typeof m.dispose === "function" && !this.sharedPoolMats.has(m)) {
            mats.add(m);
            collectMaterialTextures(m);
          }
        }
      } else if (mat && typeof mat.dispose === "function" && !this.sharedPoolMats.has(mat)) {
        mats.add(mat);
        collectMaterialTextures(mat);
      }
    });

    for (const t of texs) t.dispose();
    for (const g of geos) g.dispose();
    for (const m of mats) m.dispose();
  }

  private clearAndDisposeGroup(group: THREE.Group): void {
    while (group.children.length > 0) {
      const child = group.children[0]!;
      group.remove(child);
      this.disposeSubtree(child);
    }
  }

  resize(widthPx: number, heightPx: number): void {
    const w = Math.max(160, Math.floor(widthPx));
    const h = Math.max(140, Math.floor(heightPx));
    this.camera.aspect = w / h;
    this.camera.updateProjectionMatrix();
    this.renderer?.setSize(w, h, false);
  }

  notifyExplicitCommandSent(): void {
    // Cancel any in-flight and queued room hops so an unrelated explicit command
    // never fires stale queued hops and a typed exit teleports the stationary avatar cleanly.
    this.navWalker.cancel();
  }

  flushPendingMovesBeforeAction(nowMs = performance.now()): void {
    if (this.awaitingInitialRoomSync) return;
    this.navWalker.expireIfStale(nowMs);

    if (this.isAboardShip && this.activeShipTemplate) {
      const serverComp = this.serverShipCompartment;
      if (!serverComp) return;
      const lp = this.playerGroup.position.clone().sub(this.shipOrigin);
      const startHint = this.navWalker.inFlightTarget ?? serverComp;
      const phys = tryFindContainingCompartment(this.activeShipTemplate, lp, startHint, 0.1);
      if (!phys) return;

      if (this.navWalker.hasInFlight(nowMs)) {
        const inFlightTarget = this.navWalker.inFlightTarget;
        if (inFlightTarget && phys !== inFlightTarget) {
          const tail = findShipExitPath(this.activeShipTemplate, inFlightTarget, phys).map((h) => ({
            command: h.command,
            targetRoom: h.targetCompartment,
          }));
          this.navWalker.replaceQueuedTailWhileInFlight("ship", tail, nowMs);
        }
        return;
      }

      if (phys !== serverComp) {
        const hops = findShipExitPath(this.activeShipTemplate, serverComp, phys).map((h) => ({
          command: h.command,
          targetRoom: h.targetCompartment,
        }));
        const first = this.navWalker.planAndDispatchOne("ship", serverComp, hops, nowMs);
        if (first) {
          this.nextMoveStepAllowedAtMs = nowMs + 600;
          this.cb.onSendCommand(first.command);
        }
      }
    } else if (!this.isAboardShip && this.activeStationPack) {
      const serverRoom = this.serverStationRoomEntry;
      if (!serverRoom) return;
      const startHint = this.navWalker.inFlightTarget ?? serverRoom;
      const phys = tryFindContainingRoomWithHysteresis(
        this.activeStationPack,
        this.playerGroup.position,
        this.originOffsetX,
        this.originOffsetZ,
        startHint,
        0.1,
      );
      if (!phys) return;

      if (this.navWalker.hasInFlight(nowMs)) {
        const inFlightTarget = this.navWalker.inFlightTarget;
        if (inFlightTarget && phys !== inFlightTarget) {
          const tail = findStationExitPath(this.activeStationPack, inFlightTarget, phys);
          this.navWalker.replaceQueuedTailWhileInFlight("station", tail, nowMs);
        }
        return;
      }

      if (phys !== serverRoom) {
        const hops = findStationExitPath(this.activeStationPack, serverRoom, phys);
        const first = this.navWalker.planAndDispatchOne("station", serverRoom, hops, nowMs);
        if (first) {
          this.nextMoveStepAllowedAtMs = nowMs + 600;
          this.cb.onSendCommand(first.command);
        }
      }
    }
  }

  syncWorldState(params: {
    stationPack: AtlasNeighborhoodPack;
    shipTemplate: ShipUnitTemplate;
    isAboardShip: boolean;
    serverStationRoomEntry: string | null;
    serverShipCompartment: string | null;
    roomRevision?: number;
    ambient: DecodedRoomAmbient;
    berthDockedHulls?: ReadonlyArray<DockedHullSummary>;
    berthOwnHullName?: string | null;
    berthOwnHullDockedHere?: boolean;
    visibleActors: VisibleRoomActor[];
    actorSpeech: Record<string, { text: string; expiresAt: number }>;
    actorComposing: Record<string, boolean>;
    roomMarkers?: ReadonlyArray<StageQuestMarker>;
    questRoomEntry?: string | null;
    questShipCompartment?: string | null;
    stowageCargoKg: number;
    holotankData: unknown;
  }): void {
    const {
      stationPack,
      shipTemplate,
      isAboardShip,
      serverStationRoomEntry,
      serverShipCompartment,
      roomRevision,
      ambient,
      berthDockedHulls,
      berthOwnHullName,
      berthOwnHullDockedHere,
      visibleActors,
      actorSpeech,
      actorComposing,
      roomMarkers,
      questRoomEntry,
      questShipCompartment,
      stowageCargoKg,
      holotankData,
    } = params;

    const prevPackId = this.activeStationPack?.packId ?? null;
    const prevUnitId = this.activeShipTemplate?.unitId ?? null;
    const prevServerRoom = this.serverStationRoomEntry;
    const prevServerComp = this.serverShipCompartment;
    const wasAboard = this.isAboardShip;
    const firstSync = this.awaitingInitialRoomSync;
    const packSwitched = prevPackId !== null && prevPackId !== stationPack.packId;
    const unitChanged = prevUnitId !== null && prevUnitId !== shipTemplate.unitId;
    const modeChanged = wasAboard !== isAboardShip;

    const newSig = computePackGeometrySignature(stationPack);
    if (newSig !== this.builtStationSig) {
      // Preserve player's atlas position when live content pack geometry streams in for the same station
      const preserveAtlasPos =
        !firstSync && !wasAboard && !isAboardShip && !packSwitched && this.activeStationPack !== null;
      const savedAtlas = preserveAtlasPos
        ? worldToAtlas2D(
            this.playerGroup.position.x,
            this.playerGroup.position.z,
            this.originOffsetX,
            this.originOffsetZ,
          )
        : null;
      const savedY = this.playerGroup.position.y;
      this.buildStationWorld(stationPack, newSig);
      if (savedAtlas) {
        const remapped = this.atlasToWorld(savedAtlas.x, savedY, savedAtlas.y);
        this.playerGroup.position.copy(remapped);
      }
    } else {
      this.activeStationPack = stationPack;
    }

    const newShipSig = computeShipTemplateSignature(shipTemplate);
    const shipRebuilt = newShipSig !== this.builtShipSig;
    if (shipRebuilt) {
      this.buildShipInterior(shipTemplate, newShipSig);
    } else {
      this.activeShipTemplate = shipTemplate;
    }

    this.isAboardShip = isAboardShip;
    this.stationRoot.visible = !isAboardShip;
    this.shipRoot.visible = isAboardShip;

    const nowMs = performance.now();
    const roomEventArrived =
      roomRevision !== undefined
        ? roomRevision !== this.lastSyncedRoomRevision
        : serverStationRoomEntry !== prevServerRoom || serverShipCompartment !== prevServerComp;
    if (roomRevision !== undefined) {
      this.lastSyncedRoomRevision = roomRevision;
    }

    if (isAboardShip && serverShipCompartment) {
      const serverCompChanged = serverShipCompartment !== prevServerComp;
      const authoritativeEvent = firstSync || modeChanged || unitChanged || serverCompChanged || roomEventArrived;
      const shouldReconcile = authoritativeEvent || shipRebuilt;

      if (shouldReconcile) {
        const lp = this.tmpVecA.copy(this.playerGroup.position).sub(this.shipOrigin);
        const physComp = tryFindContainingCompartment(
          shipTemplate,
          lp,
          this.activeShipCompartment,
          0.55,
        );

        let ackTarget: string | null = null;
        let nextHopCmd: string | null = null;
        if (modeChanged || unitChanged) {
          this.navWalker.cancel();
        } else if (authoritativeEvent) {
          const allowAdvance = !firstSync && physComp !== null && physComp !== serverShipCompartment;
          const navRes = this.navWalker.onAuthoritativeRoomUpdate(
            "ship",
            serverShipCompartment,
            nowMs,
            allowAdvance,
          );
          ackTarget = navRes.acknowledgedTarget;
          nextHopCmd = navRes.nextHopToSend?.command ?? null;
        }

        this.awaitingInitialRoomSync = false;
        this.nextMoveStepAllowedAtMs = nowMs + 220;
        this.serverShipCompartment = serverShipCompartment;
        this.serverStationRoomEntry = null;

        const alreadyInComp =
          wasAboard && !firstSync && !unitChanged && physComp === serverShipCompartment;
        const walkedAhead =
          wasAboard &&
          !firstSync &&
          !unitChanged &&
          ackTarget === serverShipCompartment &&
          physComp !== null &&
          findShipExitPath(shipTemplate, serverShipCompartment, physComp).length > 0;

        this.activeShipCompartment = walkedAhead && physComp ? physComp : serverShipCompartment;
        if (!alreadyInComp && !walkedAhead) {
          const spawnYaw = serverShipCompartment === "bridge" ? 0 : Math.PI;
          this.teleportPlayer(this.getShipCompartmentSpawn(serverShipCompartment), spawnYaw);
        }
        this.cb.onPhysicalLocationChanged(this.activeShipCompartment);

        if (nextHopCmd) {
          this.nextMoveStepAllowedAtMs = nowMs + 350;
          this.cb.onSendCommand(nextHopCmd);
        }
      }
      if (this.airlockLeaveWi) {
        this.airlockLeaveWi.title = ambient.docked
          ? `Disembark ${ambient.hullName ?? "Ship"} Ashore`
          : "Outer Airlock Sealed (Under Way)";
      }
      this.updateCargoFill(Math.round(stowageCargoKg / 1000));
      this.updateHolotank(holotankData);
    } else if (!isAboardShip && serverStationRoomEntry) {
      const serverRoomChanged = serverStationRoomEntry !== prevServerRoom;
      const shouldReconcile =
        firstSync || modeChanged || packSwitched || serverRoomChanged || roomEventArrived;

      if (shouldReconcile) {
        const physRoom = tryFindContainingRoomWithHysteresis(
          stationPack,
          this.playerGroup.position,
          this.originOffsetX,
          this.originOffsetZ,
          this.activeStationRoomEntry,
        );

        let ackTarget: string | null = null;
        let nextHopCmd: string | null = null;
        if (modeChanged || packSwitched) {
          this.navWalker.cancel();
        } else {
          const allowAdvance = !firstSync && physRoom !== null && physRoom !== serverStationRoomEntry;
          const navRes = this.navWalker.onAuthoritativeRoomUpdate(
            "station",
            serverStationRoomEntry,
            nowMs,
            allowAdvance,
          );
          ackTarget = navRes.acknowledgedTarget;
          nextHopCmd = navRes.nextHopToSend?.command ?? null;
        }

        this.awaitingInitialRoomSync = false;
        this.nextMoveStepAllowedAtMs = nowMs + 220;
        this.serverStationRoomEntry = serverStationRoomEntry;
        this.serverShipCompartment = null;

        const alreadyInRoom =
          !wasAboard && !firstSync && !packSwitched && physRoom === serverStationRoomEntry;
        const walkedAhead =
          !wasAboard &&
          !firstSync &&
          !packSwitched &&
          ackTarget === serverStationRoomEntry &&
          physRoom !== null &&
          areRoomsConnectedWithinHops(stationPack, serverStationRoomEntry, physRoom, 3);

        const activeRoom = walkedAhead && physRoom ? physRoom : serverStationRoomEntry;
        this.highlightStationRoom(activeRoom);
        if (!alreadyInRoom && !walkedAhead) {
          // Face North (`-Z`, `yaw = Math.PI`) in right-handed station coordinates
          this.teleportPlayer(this.getStationRoomSpawn(serverStationRoomEntry), Math.PI);
        }
        this.cb.onPhysicalLocationChanged(activeRoom);

        if (nextHopCmd) {
          this.nextMoveStepAllowedAtMs = nowMs + 350;
          this.cb.onSendCommand(nextHopCmd);
        }
      }
      this.updateBerthGalleryCradles(
        ambient,
        berthDockedHulls,
        berthOwnHullName,
        berthOwnHullDockedHere,
      );
    } else if (roomEventArrived || modeChanged || packSwitched || unitChanged) {
      this.navWalker.cancel();
    }

    const scopeKey = isAboardShip
      ? `ship:${shipTemplate.unitId}:${serverShipCompartment ?? "airlock"}`
      : `station:${stationPack.packId}:${serverStationRoomEntry ?? this.activeStationRoomEntry}`;
    this.syncOccupants(scopeKey, visibleActors, actorSpeech, actorComposing, roomMarkers ?? []);
    this.syncQuestWayfinder(
      isAboardShip,
      questRoomEntry ?? null,
      questShipCompartment ?? null,
    );
  }

  private syncQuestWayfinder(
    isAboardShip: boolean,
    targetRoomEntry: string | null,
    targetShipComp: string | null,
  ): void {
    const pack = this.activeStationPack;
    const shipTpl = this.activeShipTemplate;
    const fromRoom = this.activeStationRoomEntry ?? this.serverStationRoomEntry;
    const fromComp = this.activeShipCompartment ?? this.serverShipCompartment;
    const sig = isAboardShip
      ? shipTpl && targetShipComp && fromComp
        ? `ship:${shipTpl.unitId}:${fromComp}:${targetShipComp}`
        : ""
      : pack && targetRoomEntry && fromRoom
        ? `station:${pack.packId}:${fromRoom}:${targetRoomEntry}`
        : "";
    if (sig === this.questWayfinderSig) return;
    this.questWayfinderSig = sig;
    this.clearAndDisposeGroup(this.questWayfinderRoot);
    this.questWayfinderTargetRoom = null;
    this.questWayfinderHopCount = 0;

    let route: QuestWayfinderPath3D | null = null;
    if (isAboardShip) {
      if (shipTpl && targetShipComp && fromComp) {
        route = computeShipQuestWayfinderPoints(
          shipTpl,
          fromComp,
          targetShipComp,
          this.shipOrigin,
        );
      }
    } else if (pack && targetRoomEntry && fromRoom) {
      route = computeQuestWayfinderPoints(
        pack,
        fromRoom,
        targetRoomEntry,
        this.originOffsetX,
        this.originOffsetZ,
      );
    }
    if (!route) return;

    const lineGeo = new THREE.BufferGeometry().setFromPoints(route.points);
    const lineMat = new THREE.LineBasicMaterial({ color: 0xffd84d });
    const ribbon = new THREE.Line(lineGeo, lineMat);
    ribbon.name = "QuestWayfinderRibbon";
    this.questWayfinderRoot.add(ribbon);

    const dest = route.points[route.points.length - 1];
    if (dest) {
      const pillarGeo = new THREE.CylinderGeometry(0.18, 0.28, 3.2, 12);
      const pillarMat = new THREE.MeshBasicMaterial({
        color: 0xffd84d,
        transparent: true,
        opacity: 0.42,
      });
      const pillar = new THREE.Mesh(pillarGeo, pillarMat);
      pillar.name = "QuestWayfinderPillar";
      pillar.position.set(dest.x, dest.y + 1.5, dest.z);
      this.questWayfinderRoot.add(pillar);

      const ringGeo = new THREE.RingGeometry(0.75, 1.15, 24);
      const ringMat = new THREE.MeshBasicMaterial({
        color: 0xffe875,
        side: THREE.DoubleSide,
        transparent: true,
        opacity: 0.72,
      });
      const ring = new THREE.Mesh(ringGeo, ringMat);
      ring.name = "QuestWayfinderRing";
      ring.rotation.x = -Math.PI / 2;
      ring.position.set(dest.x, dest.y + 0.04, dest.z);
      this.questWayfinderRoot.add(ring);
    }

    this.questWayfinderTargetRoom = route.targetRoomEntry;
    this.questWayfinderHopCount = route.hopCount;
  }

  /** Read-only test inspection helper for the 3D golden quest wayfinder trail. */
  getQuestWayfinderForTest(): { targetRoom: string | null; hopCount: number; childCount: number } {
    return {
      targetRoom: this.questWayfinderTargetRoom,
      hopCount: this.questWayfinderHopCount,
      childCount: this.questWayfinderRoot.children.length,
    };
  }

  /** Read-only test inspection helper for a spawned actor's active quest marker. */
  getSpawnedActorQuestMarkerForTest(actorId: string): StageQuestMarker | null {
    return this.spawnedActors.get(actorId)?.questMarker ?? null;
  }

  /** Read-only test inspection helper for a spawned actor's interactable prompt. */
  getSpawnedActorInteractableForTest(actorId: string): WorldInteractable3D | null {
    return this.spawnedActors.get(actorId)?.interactable ?? null;
  }

  // -- Avatar & Occupants ---------------------------------------------------

  private buildPlayerAvatar(): void {
    const suit = this.createArticulatedSuit(0x365a82, 0x33ddff);
    this.playerGroup.add(suit.root);
    this.playerLeftLeg = suit.leftLeg;
    this.playerRightLeg = suit.rightLeg;
    this.playerLeftArm = suit.leftArm;
    this.playerRightArm = suit.rightArm;
  }

  private createArticulatedSuit(
    suitHex: number,
    visorHex: number,
  ): {
    root: THREE.Group;
    leftLeg: THREE.Group;
    rightLeg: THREE.Group;
    leftArm: THREE.Group;
    rightArm: THREE.Group;
  } {
    const root = new THREE.Group();
    const suitMat = new THREE.MeshStandardMaterial({ color: suitHex, roughness: 0.46, metalness: 0.35 });
    const darkMat = new THREE.MeshStandardMaterial({ color: 0x1d232c, roughness: 0.58, metalness: 0.45 });
    const visorMat = new THREE.MeshStandardMaterial({
      color: visorHex,
      emissive: visorHex,
      emissiveIntensity: 1.4,
      roughness: 0.2,
    });

    const torso = new THREE.Mesh(new THREE.BoxGeometry(0.42, 0.56, 0.26), suitMat);
    torso.position.set(0, 1.14, 0);
    root.add(torso);

    const pack = new THREE.Mesh(new THREE.BoxGeometry(0.30, 0.38, 0.14), darkMat);
    pack.position.set(0, 1.16, -0.18);
    root.add(pack);

    const head = new THREE.Mesh(new THREE.SphereGeometry(0.15, 14, 12), suitMat);
    head.position.set(0, 1.60, 0);
    root.add(head);

    const visor = new THREE.Mesh(new THREE.BoxGeometry(0.20, 0.08, 0.10), visorMat);
    visor.position.set(0, 1.61, 0.11);
    root.add(visor);

    const makeLimb = (hx: number, hy: number, w: number, len: number, mat: THREE.Material): THREE.Group => {
      const pivot = new THREE.Group();
      pivot.position.set(hx, hy, 0);
      const mesh = new THREE.Mesh(new THREE.BoxGeometry(w, len, w), mat);
      mesh.position.set(0, -len * 0.5, 0);
      pivot.add(mesh);
      root.add(pivot);
      return pivot;
    };

    const leftLeg = makeLimb(-0.11, 0.84, 0.14, 0.84, darkMat);
    const rightLeg = makeLimb(0.11, 0.84, 0.14, 0.84, darkMat);
    const leftArm = makeLimb(-0.28, 1.38, 0.11, 0.62, suitMat);
    const rightArm = makeLimb(0.28, 1.38, 0.11, 0.62, suitMat);

    return { root, leftLeg, rightLeg, leftArm, rightArm };
  }

  private pruneActorInteractable(actorId: string): void {
    const targetId = `actor:${actorId}`;
    for (const list of [this.interactables, this.shipInteractables]) {
      for (let i = list.length - 1; i >= 0; i--) {
        if (list[i]?.id === targetId) {
          list.splice(i, 1);
        }
      }
    }
    if (this.focusedInteractable?.id === targetId) {
      this.focusedInteractable = null;
      this.cb.onFocusedPromptChanged(null);
    }
  }

  private removeAndDisposeActor(rec: SpawnedActorRecord): void {
    this.actorsRoot.remove(rec.group);
    this.disposeSubtree(rec.group);
    this.actorsRoot.remove(rec.billboardSprite);
    this.disposeSubtree(rec.billboardSprite);
    rec.billboardTex.dispose();
    this.pruneActorInteractable(rec.id);
  }

  private clearAllActors(): void {
    for (const rec of this.spawnedActors.values()) {
      this.removeAndDisposeActor(rec);
    }
    this.spawnedActors.clear();
  }

  private syncOccupants(
    scopeKey: string,
    actors: VisibleRoomActor[],
    actorSpeech: Record<string, { text: string; expiresAt: number }>,
    actorComposing: Record<string, boolean>,
    roomMarkers: ReadonlyArray<StageQuestMarker>,
  ): void {
    if (scopeKey !== this.spawnedActorsScopeKey) {
      this.clearAllActors();
      this.spawnedActorsScopeKey = scopeKey;
    }

    const markerById = new Map<string, StageQuestMarker>();
    const markerByName = new Map<string, StageQuestMarker>();
    const keysById = new Map<string, Set<string>>();
    const keysByName = new Map<string, Set<string>>();
    for (const m of roomMarkers) {
      const qKey = m.questKey || `${m.marker}:${m.questTitle ?? ""}`;
      if (m.npcId) {
        const cur = markerById.get(m.npcId);
        if (!cur || questMarkerPriority(m.marker) < questMarkerPriority(cur.marker)) {
          markerById.set(m.npcId, m);
        }
        let set = keysById.get(m.npcId);
        if (!set) {
          set = new Set<string>();
          keysById.set(m.npcId, set);
        }
        set.add(qKey);
      }
      if (m.npcName) {
        const nameKey = m.npcName.toLowerCase();
        const cur = markerByName.get(nameKey);
        if (!cur || questMarkerPriority(m.marker) < questMarkerPriority(cur.marker)) {
          markerByName.set(nameKey, m);
        }
        let set = keysByName.get(nameKey);
        if (!set) {
          set = new Set<string>();
          keysByName.set(nameKey, set);
        }
        set.add(qKey);
      }
    }

    const seenIds = new Set<string>();
    const now = Date.now();

    actors.forEach((act, idx) => {
      seenIds.add(act.id);
      const spot = this.computeActorStandSpot(idx);
      const safeBounds = this.computeCurrentRoomSafeBounds();
      const actNameLow = act.name.toLowerCase();
      const byId = markerById.get(act.id) ?? null;
      const byName = markerByName.get(actNameLow) ?? null;
      const qMarker =
        byId && byName
          ? questMarkerPriority(byName.marker) < questMarkerPriority(byId.marker)
            ? byName
            : byId
          : (byId ?? byName);
      const distinctKeys = new Set<string>([
        ...(keysById.get(act.id) ?? []),
        ...(keysByName.get(actNameLow) ?? []),
      ]);
      const extraQuestCount = Math.max(0, distinctKeys.size - 1);
      const prompt = formatActorInteractPrompt(
        act.name,
        act.isNpc,
        act.shortDesc,
        qMarker,
        extraQuestCount,
      );

      const existing = this.spawnedActors.get(act.id);
      if (existing) {
        existing.actor = act;
        existing.targetPos.copy(spot);
        existing.safeBounds = safeBounds;
        existing.questMarker = qMarker;
        const promptChanged =
          existing.interactable.title !== prompt.title ||
          existing.interactable.subtitle !== prompt.subtitle;
        existing.interactable.title = prompt.title;
        existing.interactable.subtitle = prompt.subtitle;
        if (existing.interactable.npcMeta) {
          existing.interactable.npcMeta.questMarker = qMarker ?? undefined;
        }
        if (promptChanged && this.focusedInteractable?.id === existing.interactable.id) {
          this.cb.onFocusedPromptChanged({
            title: `[E] ${existing.interactable.title}`,
            subtitle: existing.interactable.subtitle,
          });
        }
        this.refreshActorBillboard(existing, act, actorSpeech, actorComposing, now);
        return;
      }

      let hash = 2166136261;
      for (let i = 0; i < act.name.length; i++) {
        hash = Math.imul(hash ^ act.name.charCodeAt(i), 16777619) >>> 0;
      }
      const hue = (hash % 360) / 360;
      const suitCol = new THREE.Color().setHSL(hue, 0.42, 0.42).getHex();
      const visorCol = act.isNpc ? 0xffb844 : 0x55ff99;
      const suit = this.createArticulatedSuit(suitCol, visorCol);
      suit.root.position.copy(spot);
      suit.root.rotation.y = Math.PI;

      const canvas = document.createElement("canvas");
      canvas.width = 384;
      canvas.height = 224;
      const tex = new THREE.CanvasTexture(canvas);
      tex.colorSpace = THREE.SRGBColorSpace;
      const spriteMat = new THREE.SpriteMaterial({ map: tex, transparent: true, depthTest: true });
      const sprite = new THREE.Sprite(spriteMat);
      sprite.scale.set(2.9, 1.69, 1);
      sprite.position.set(spot.x, spot.y + 2.45, spot.z);

      // Add `suit.root` directly to `actorsRoot` so `actorsRoot.remove(rec.group)` always removes it cleanly (`W3D-C1`).
      this.actorsRoot.add(suit.root);
      this.actorsRoot.add(sprite);

      // Register click/[E] interactable on actor mesh
      const actorWi: WorldInteractable3D = {
        id: `actor:${act.id}`,
        kind: "npc",
        title: prompt.title,
        subtitle: prompt.subtitle,
        actionOrKey: act.name,
        npcMeta: {
          id: act.id,
          name: act.name,
          bio: act.shortDesc,
          isNpc: act.isNpc,
          questMarker: qMarker ?? undefined,
        },
        worldPos: spot.clone(),
        maxDistance: 4.2,
        meshTargets: [suit.root],
      };
      if (this.isAboardShip) this.shipInteractables.push(actorWi);
      else this.interactables.push(actorWi);

      const rec: SpawnedActorRecord = {
        id: act.id,
        name: act.name,
        isNpc: act.isNpc,
        actor: act,
        group: suit.root,
        leftLeg: suit.leftLeg,
        rightLeg: suit.rightLeg,
        leftArm: suit.leftArm,
        rightArm: suit.rightArm,
        targetPos: spot.clone(),
        walkPhase: 0,
        billboardSprite: sprite,
        billboardTex: tex,
        billboardCanvas: canvas,
        lastRenderedLabel: "",
        speechExpiresAt: 0,
        lastIsTyping: false,
        questMarker: qMarker,
        interactable: actorWi,
        safeBounds,
      };
      this.spawnedActors.set(act.id, rec);
      this.refreshActorBillboard(rec, act, actorSpeech, actorComposing, now);
    });

    for (const [id, rec] of [...this.spawnedActors.entries()]) {
      if (!seenIds.has(id)) {
        this.removeAndDisposeActor(rec);
        this.spawnedActors.delete(id);
      }
    }
  }

  private refreshActorBillboard(
    rec: SpawnedActorRecord,
    act: VisibleRoomActor,
    actorSpeech: Record<string, { text: string; expiresAt: number }>,
    actorComposing: Record<string, boolean>,
    nowMs: number,
  ): void {
    const lowerName = act.name.toLowerCase();
    const sp = Object.hasOwn(actorSpeech, act.id)
      ? actorSpeech[act.id]
      : Object.hasOwn(actorSpeech, lowerName)
        ? actorSpeech[lowerName]
        : undefined;
    const activeSpeech = sp && sp.expiresAt > nowMs ? sp.text : "";
    const isTyping = Boolean(
      (Object.hasOwn(actorComposing, act.id) && actorComposing[act.id]) ||
        (Object.hasOwn(actorComposing, lowerName) && actorComposing[lowerName]),
    );
    rec.speechExpiresAt = activeSpeech && sp ? sp.expiresAt : 0;
    rec.lastIsTyping = isTyping;
    this.renderActorBillboard(rec, act, activeSpeech, isTyping);
  }

  private renderActorBillboard(
    rec: SpawnedActorRecord,
    act: VisibleRoomActor,
    activeSpeech: string,
    isTyping: boolean,
  ): void {
    const bubble = activeSpeech || (isTyping ? "..." : "");
    const qm = rec.questMarker;
    const qSig = qm ? `${qm.marker}:${qm.symbol}` : "";
    const sig = `${act.name}|${act.isNpc ? "N" : "P"}|${qSig}|${bubble}`;
    if (sig === rec.lastRenderedLabel) return;
    rec.lastRenderedLabel = sig;
    if (this.headlessDom) return;

    const ctx = rec.billboardCanvas.getContext("2d");
    if (!ctx) return;
    const W = rec.billboardCanvas.width;
    const H = rec.billboardCanvas.height;
    ctx.clearRect(0, 0, W, H);

    // WoW-style overhead glowing Quest Marker emblem (`!` gold, `?` gold turn-in, `?` silver in-progress)
    if (qm) {
      const gold = qm.marker === "available" || qm.marker === "turn_in";
      const cx = bubble ? W - 36 : W * 0.5;
      const cy = bubble ? 30 : 72;
      const rad = bubble ? 22 : 30;

      // Soft outer halo
      ctx.save();
      ctx.beginPath();
      ctx.arc(cx, cy, rad + 10, 0, Math.PI * 2);
      ctx.fillStyle = gold ? "rgba(255, 214, 48, 0.30)" : "rgba(160, 186, 212, 0.22)";
      ctx.fill();

      // Core crest disc
      ctx.beginPath();
      ctx.arc(cx, cy, rad, 0, Math.PI * 2);
      ctx.fillStyle = gold ? "#ffd633" : "#8fa2b8";
      ctx.fill();
      ctx.lineWidth = 3;
      ctx.strokeStyle = gold ? "#fff5bd" : "#d8e4f0";
      ctx.stroke();

      // Bold `!` or `?` symbol
      ctx.fillStyle = gold ? "#2b1900" : "#121a24";
      ctx.font = `900 ${bubble ? 28 : 40}px ui-sans-serif, system-ui, sans-serif`;
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      ctx.fillText(qm.symbol, cx, cy + 2);
      ctx.restore();
    }

    if (bubble) {
      const clipped = bubble.length > 92 ? `${bubble.slice(0, 89)}…` : bubble;
      const rightPad = qm ? 68 : 14;
      ctx.fillStyle = "rgba(10, 18, 30, 0.88)";
      ctx.strokeStyle = "rgba(255, 226, 130, 0.75)";
      ctx.lineWidth = 2;
      ctx.beginPath();
      ctx.roundRect(14, 38, W - 14 - rightPad, 96, 10);
      ctx.fill();
      ctx.stroke();

      ctx.fillStyle = "#fff2a8";
      ctx.font = "16px ui-sans-serif, system-ui, sans-serif";
      ctx.textAlign = "center";
      const words = clipped.split(/\s+/);
      const lines: string[] = [];
      let cur = "";
      for (const w of words) {
        const test = cur ? `${cur} ${w}` : w;
        if (test.length > 32 && cur) {
          lines.push(cur);
          cur = w;
        } else {
          cur = test;
        }
      }
      if (cur) lines.push(cur);
      const bx = 14 + (W - 14 - rightPad) * 0.5;
      lines.slice(0, 3).forEach((ln, idx) => {
        ctx.fillText(ln, bx, 66 + idx * 22);
      });
    }

    // Nameplate pill at bottom
    ctx.fillStyle = "rgba(8, 14, 24, 0.84)";
    ctx.beginPath();
    ctx.roundRect(44, 150, W - 88, 58, 8);
    ctx.fill();
    if (qm && (qm.marker === "available" || qm.marker === "turn_in")) {
      ctx.strokeStyle = "rgba(255, 214, 64, 0.82)";
      ctx.lineWidth = 2;
      ctx.stroke();
    }

    ctx.fillStyle = "#ffffff";
    ctx.font = "bold 20px ui-sans-serif, system-ui, sans-serif";
    ctx.textAlign = "center";
    ctx.fillText(act.name, W * 0.5, 176);

    ctx.fillStyle = qm
      ? qm.marker === "in_progress"
        ? "#b8cde0"
        : "#ffdd55"
      : act.isNpc
        ? "#88ccff"
        : "#66eeaa";
    ctx.font = "bold 14px ui-monospace, monospace";
    const subTag = qm
      ? qm.marker === "turn_in"
        ? "[?] TURN IN QUEST"
        : qm.marker === "available"
          ? "[!] QUEST AVAILABLE"
          : "[?] QUEST IN PROGRESS"
      : act.isNpc
        ? "[NPC]"
        : "[CREW / PILOT]";
    ctx.fillText(subTag, W * 0.5, 198);

    rec.billboardTex.needsUpdate = true;
  }

  private computeActorStandSpot(idx: number): THREE.Vector3 {
    if (this.isAboardShip && this.activeShipTemplate) {
      const comp =
        resolveCompartmentDef(this.activeShipTemplate, this.serverShipCompartment) ??
        this.activeShipTemplate.compartments.get("airlock")!;
      const floorY = comp.localCenter.y - comp.localSize.y * 0.5;
      const maxX = Math.max(0.85, comp.localSize.x * 0.5 - 0.95);
      const maxZ = Math.max(0.85, comp.localSize.z * 0.5 - 1.1);
      const side = idx % 2 === 0 ? 1 : -1;
      const xOff = Math.max(-maxX, Math.min(maxX, side * (1.35 + Math.floor(idx / 2) * 0.45)));
      const zOff = Math.max(-maxZ, Math.min(maxZ, ((idx % 3) - 1) * 1.15));
      return new THREE.Vector3(
        this.shipOrigin.x + comp.localCenter.x + xOff,
        this.shipOrigin.y + floorY,
        this.shipOrigin.z + comp.localCenter.z + zOff,
      );
    }

    const pack = this.activeStationPack;
    const room =
      pack?.rooms.get(this.serverStationRoomEntry ?? this.activeStationRoomEntry) ??
      pack?.rooms.values().next().value;
    if (!pack || !room) return new THREE.Vector3(0, 6, 0);

    const elev = getFloorElevationY(pack, room.floorIndex);
    const centerPos = this.atlasToWorld(
      (room.minX + room.maxX) * 0.5,
      elev,
      (room.minY + room.maxY) * 0.5,
    );
    const halfW = Math.max(1.5, (room.maxX - room.minX) * 0.5 - 2.2);
    const halfD = Math.max(1.5, (room.maxY - room.minY) * 0.5 - 2.2);
    const angle = idx * 1.35 + 0.6;
    const rad = Math.min(halfW, halfD, 2.8 + (idx % 3) * 1.1);
    return new THREE.Vector3(
      centerPos.x + Math.cos(angle) * rad,
      elev,
      centerPos.z + Math.sin(angle) * rad,
    );
  }

  private computeCurrentRoomSafeBounds(): SpawnedActorRecord["safeBounds"] {
    if (this.isAboardShip && this.activeShipTemplate) {
      const comp =
        resolveCompartmentDef(this.activeShipTemplate, this.serverShipCompartment) ??
        this.activeShipTemplate.compartments.get("airlock")!;
      return {
        minX: this.shipOrigin.x + comp.localCenter.x - comp.localSize.x * 0.5,
        maxX: this.shipOrigin.x + comp.localCenter.x + comp.localSize.x * 0.5,
        minZ: this.shipOrigin.z + comp.localCenter.z - comp.localSize.z * 0.5,
        maxZ: this.shipOrigin.z + comp.localCenter.z + comp.localSize.z * 0.5,
        margin: 1.05,
      };
    }
    const pack = this.activeStationPack;
    const room = pack?.rooms.get(this.serverStationRoomEntry ?? this.activeStationRoomEntry);
    if (!pack || !room) {
      return { minX: -100, maxX: 100, minZ: -100, maxZ: 100, margin: 1.4 };
    }
    return {
      minX: room.minX + this.originOffsetX,
      maxX: room.maxX + this.originOffsetX,
      minZ: this.atlasZ(room.maxY),
      maxZ: this.atlasZ(room.minY),
      margin: 1.5,
    };
  }

  // -- Station World Builder ------------------------------------------------

  private buildStationWorld(pack: AtlasNeighborhoodPack, sig: string): void {
    this.activeStationPack = pack;
    this.builtStationSig = sig;

    this.clearAndDisposeGroup(this.stationRoot);
    this.roomLights.clear();
    this.wallObstacles.length = 0;
    this.stationStairs.length = 0;
    // Preserve live actor interactables? Rebuilding station clears station fixtures, re-keep spawned actors in syncOccupants.
    for (let i = this.interactables.length - 1; i >= 0; i--) {
      if (!this.interactables[i]?.id.startsWith("actor:")) {
        this.interactables.splice(i, 1);
      }
    }
    this.cradleSignSprites = [];
    this.cradleHullGroups = [];
    this.airlockBoardWi = null;

    const center = getPackBoundsCenter2D(pack);
    this.originOffsetX = -center.x;
    this.originOffsetZ = -center.y;
    this.activeStationRoomEntry = getDefaultSpawnRoomEntry(pack);

    const tex = getOrBuildProceduralTextures();
    const isRock =
      pack.packId.includes("stickney") ||
      pack.packId.includes("kirkwood") ||
      pack.packId.includes("voltaire");

    const { horizontalDoors, verticalDoors, interFloorExits } = computeDoorwaysAndStairs(pack);

    let maxFloorIdx = 0;
    for (const r of pack.rooms.values()) {
      if (r.floorIndex > maxFloorIdx) maxFloorIdx = r.floorIndex;
    }

    for (const room of pack.rooms.values()) {
      this.buildStationRoom(
        room,
        pack,
        tex,
        isRock,
        maxFloorIdx,
        horizontalDoors,
        verticalDoors,
        interFloorExits,
      );
    }

    this.buildStationStairs(pack, interFloorExits, tex);
    this.buildStationDesksAndPods(pack);
    if (pack.packId !== "neighborhood.ooc_area") {
      this.buildExteriorBerthCradles(pack);
    }
  }

  private atlasZ(ay: number): number {
    return -(ay + this.originOffsetZ);
  }

  private atlasToWorld(ax: number, elevY: number, ay: number): THREE.Vector3 {
    const p = atlasToWorldPos(ax, elevY, ay, this.originOffsetX, this.originOffsetZ);
    return new THREE.Vector3(p.x, p.y, p.z);
  }

  private buildStationRoom(
    room: AtlasRoomDef,
    pack: AtlasNeighborhoodPack,
    tex: SharedPbrTextures,
    isRock: boolean,
    maxFloorIdx: number,
    hDoors: HorizontalDoorDef[],
    vDoors: VerticalDoorDef[],
    interFloorExits: AtlasExitDef[],
  ): void {
    const elev = getFloorElevationY(pack, room.floorIndex);
    const height = getFloorHeight(pack, room.floorIndex);
    const w = room.maxX - room.minX;
    const d = room.maxY - room.minY;
    const roomMinX = room.minX + this.originOffsetX;
    const roomMaxX = room.maxX + this.originOffsetX;
    const roomMinZ = this.atlasZ(room.maxY);
    const roomMaxZ = this.atlasZ(room.minY);
    const cx = (roomMinX + roomMaxX) * 0.5;
    const cz = (roomMinZ + roomMaxZ) * 0.5;
    const isGallery = room.entryId.includes("berth_gallery");

    const floorMat = new THREE.MeshStandardMaterial({
      color: isRock ? 0x6f6860 : 0x6c7a8c,
      map: cloneRepeatTex(tex.deckMap, w / 3.2, d / 3.2),
      normalMap: cloneRepeatTex(tex.deckNormal, w / 3.2, d / 3.2),
      roughness: 0.62,
      metalness: 0.38,
    });

    // Cut stairwell opening ONLY when this room is the UPPER end of an inter-floor exit (`W3D-H3`),
    // using the exact authored `portal.at` placement clamped inside upper ∩ lower footprints.
    let stairCut: RectXZ | null = null;
    for (const ex of interFloorExits) {
      const placement = computeInterFloorStairPlacement(pack, ex);
      if (placement && placement.upperRoomId === room.entryId) {
        stairCut = {
          minX: placement.cutMinAx + this.originOffsetX,
          maxX: placement.cutMaxAx + this.originOffsetX,
          minZ: this.atlasZ(placement.cutMaxAy),
          maxZ: this.atlasZ(placement.cutMinAy),
        };
        break;
      }
    }
    this.addSlabWithCutout(
      this.stationRoot,
      elev - 0.14,
      0.28,
      roomMinX,
      roomMaxX,
      roomMinZ,
      roomMaxZ,
      stairCut,
      floorMat,
    );

    const ceilMat = new THREE.MeshStandardMaterial({
      color: 0x4f5a68,
      map: cloneRepeatTex(tex.ceilingMap, w / 4.0, d / 4.0),
      roughness: 0.88,
      metalness: 0.16,
    });
    if (elev > 0.5) {
      this.addSlabWithCutout(
        this.stationRoot,
        elev - 0.30,
        0.05,
        roomMinX,
        roomMaxX,
        roomMinZ,
        roomMaxZ,
        stairCut,
        ceilMat,
      );
    }
    if (room.floorIndex >= maxFloorIdx || isGallery) {
      this.addBox(this.stationRoot, new THREE.Vector3(cx, elev + height + 0.08, cz), new THREE.Vector3(w, 0.16, d), ceilMat, false);
    }

    const wallMat = new THREE.MeshStandardMaterial({
      color: isRock ? 0x7d7268 : 0x748294,
      map: cloneRepeatTex(isRock ? tex.rockMap : tex.bulkheadMap, Math.max(1, w / 4.2), 1),
      normalMap: isRock ? null : cloneRepeatTex(tex.bulkheadNormal, Math.max(1, w / 4.2), 1),
      roughness: 0.68,
      metalness: 0.32,
    });

    // North wall (maxY -> roomMinZ) & South wall (minY -> roomMaxZ)
    this.buildHorizontalWall(room, room.maxY, elev, height, wallMat, hDoors);
    if (isGallery) {
      this.buildGalleryViewportWall(room, elev, height);
    } else {
      this.buildHorizontalWall(room, room.minY, elev, height, wallMat, hDoors);
    }

    // West wall (minX) & East wall (maxX)
    this.buildVerticalWall(room, room.minX, elev, height, wallMat, vDoors);
    this.buildVerticalWall(room, room.maxX, elev, height, wallMat, vDoors);

    // Recessed ceiling luminaire + room point light
    const lightHex = room.lighting === "warm" ? 0xffd2a0 : isGallery ? 0xaad8ff : 0xdbeeff;
    const housingMat = new THREE.MeshStandardMaterial({ color: 0x1a2028, roughness: 0.6, metalness: 0.5 });
    const diffMat = new THREE.MeshStandardMaterial({ color: lightHex, emissive: lightHex, emissiveIntensity: 1.8 });
    this.addBox(this.stationRoot, new THREE.Vector3(cx, elev + height - 0.05, cz), new THREE.Vector3(0.85, 0.08, 2.4), housingMat, false);
    this.addBox(this.stationRoot, new THREE.Vector3(cx, elev + height - 0.09, cz), new THREE.Vector3(0.55, 0.04, 2.1), diffMat, false);

    const pt = new THREE.PointLight(lightHex, 2.2, Math.max(w, d) * 1.15, 1.3);
    pt.position.set(cx, elev + height - 0.55, cz);
    this.stationRoot.add(pt);
    this.roomLights.set(room.entryId, pt);

    // Diegetic Room Status Board on North wall (roomMinZ), facing South (+Z, yaw=0) into the room
    const boardX = cx;
    const hasNorthDoorCenter = hDoors.some(
      (hd) => hd.floor === room.floorIndex && Math.abs(hd.yWall - room.maxY) < 0.3 && Math.abs(hd.centerX + this.originOffsetX - boardX) < 3.5,
    );
    const placedBoardX = hasNorthDoorCenter ? cx - Math.min(6, w * 0.28) : cx;
    this.buildDiegeticWallSign(
      this.stationRoot,
      `${room.name.toUpperCase()}\n${room.shortDesc}`,
      new THREE.Vector3(placedBoardX, elev + 2.35, roomMinZ + 0.16),
      0,
      3.8,
      1.35,
    );

    // Detail plaques (>= 3.6m spacing)
    const detailKeys = Object.keys(room.details);
    detailKeys.slice(0, 4).forEach((kw, idx) => {
      const px = room.minX + 3.2 + idx * 4.0;
      if (px > room.maxX - 2.5) return;
      const worldPos = this.atlasToWorld(px, elev, room.maxY - 1.25);
      this.buildObservationPlaque(this.stationRoot, kw, room.details[kw] ?? "", worldPos, false);
    });
  }

  private buildHorizontalWall(
    room: AtlasRoomDef,
    yAtlas: number,
    elev: number,
    height: number,
    mat: THREE.Material,
    hDoors: HorizontalDoorDef[],
  ): void {
    const doors = hDoors
      .filter(
        (d) =>
          d.floor === room.floorIndex &&
          Math.abs(d.yWall - yAtlas) < 0.3 &&
          d.centerX > room.minX + 1 &&
          d.centerX < room.maxX - 1,
      )
      .map((d) => d.centerX)
      .sort((a, b) => a - b);

    const doorHalfW = 1.65;
    const doorH = Math.min(2.9, height - 0.4);
    const wz = this.atlasZ(yAtlas);
    let curX = room.minX;

    for (const dx of doors) {
      const segEnd = dx - doorHalfW;
      if (segEnd - curX > 0.2) {
        const sw = segEnd - curX;
        const scx = (curX + segEnd) * 0.5 + this.originOffsetX;
        this.addBox(this.stationRoot, new THREE.Vector3(scx, elev + height * 0.5, wz), new THREE.Vector3(sw, height, 0.26), mat, true);
      }
      if (height - doorH > 0.2) {
        const lh = height - doorH;
        this.addBox(
          this.stationRoot,
          new THREE.Vector3(dx + this.originOffsetX, elev + doorH + lh * 0.5, wz),
          new THREE.Vector3(doorHalfW * 2, lh, 0.26),
          mat,
          false,
        );
      }
      curX = dx + doorHalfW;
    }

    if (room.maxX - curX > 0.2) {
      const sw = room.maxX - curX;
      const scx = (curX + room.maxX) * 0.5 + this.originOffsetX;
      this.addBox(this.stationRoot, new THREE.Vector3(scx, elev + height * 0.5, wz), new THREE.Vector3(sw, height, 0.26), mat, true);
    }
  }

  private buildVerticalWall(
    room: AtlasRoomDef,
    xAtlas: number,
    elev: number,
    height: number,
    mat: THREE.Material,
    vDoors: VerticalDoorDef[],
  ): void {
    const doors = vDoors
      .filter(
        (d) =>
          d.floor === room.floorIndex &&
          Math.abs(d.xWall - xAtlas) < 0.3 &&
          d.centerY > room.minY + 1 &&
          d.centerY < room.maxY - 1,
      )
      .map((d) => d.centerY)
      .sort((a, b) => a - b);

    const doorHalfW = 1.65;
    const doorH = Math.min(2.9, height - 0.4);
    const wx = xAtlas + this.originOffsetX;
    let curY = room.minY;

    for (const dy of doors) {
      const segEnd = dy - doorHalfW;
      if (segEnd - curY > 0.2) {
        const sd = segEnd - curY;
        const scz = this.atlasZ((curY + segEnd) * 0.5);
        this.addBox(this.stationRoot, new THREE.Vector3(wx, elev + height * 0.5, scz), new THREE.Vector3(0.26, height, sd), mat, true);
      }
      if (height - doorH > 0.2) {
        const lh = height - doorH;
        this.addBox(
          this.stationRoot,
          new THREE.Vector3(wx, elev + doorH + lh * 0.5, this.atlasZ(dy)),
          new THREE.Vector3(0.26, lh, doorHalfW * 2),
          mat,
          false,
        );
      }
      curY = dy + doorHalfW;
    }

    if (room.maxY - curY > 0.2) {
      const sd = room.maxY - curY;
      const scz = this.atlasZ((curY + room.maxY) * 0.5);
      this.addBox(this.stationRoot, new THREE.Vector3(wx, elev + height * 0.5, scz), new THREE.Vector3(0.26, height, sd), mat, true);
    }
  }

  private buildGalleryViewportWall(room: AtlasRoomDef, elev: number, height: number): void {
    const w = room.maxX - room.minX;
    const cx = (room.minX + room.maxX) * 0.5 + this.originOffsetX;
    const wz = this.atlasZ(room.minY);
    const frameMat = new THREE.MeshStandardMaterial({ color: 0x2c3644, roughness: 0.45, metalness: 0.65 });
    const railMat = new THREE.MeshStandardMaterial({ color: 0x8090a4, roughness: 0.35, metalness: 0.78 });
    const glassMat = new THREE.MeshPhysicalMaterial({
      color: 0x88c8ff,
      transparent: true,
      opacity: 0.14,
      roughness: 0.15,
      metalness: 0.05,
    });

    this.addBox(this.stationRoot, new THREE.Vector3(cx, elev + 0.42, wz), new THREE.Vector3(w, 0.84, 0.28), frameMat, true);
    this.addBox(this.stationRoot, new THREE.Vector3(cx, elev + height - 0.32, wz), new THREE.Vector3(w, 0.64, 0.28), frameMat, false);
    this.addBox(this.stationRoot, new THREE.Vector3(cx, elev + height * 0.5, wz), new THREE.Vector3(w, height - 1.4, 0.08), glassMat, true);

    // Brushed-steel stanchion handrail inside the gallery (North of South viewport wall -> -Z)
    const railZ = this.atlasZ(room.minY + 0.85);
    this.addBox(this.stationRoot, new THREE.Vector3(cx, elev + 1.02, railZ), new THREE.Vector3(w - 2.0, 0.06, 0.06), railMat, false);
    const stanchions = Math.max(4, Math.floor(w / 6));
    for (let i = 0; i <= stanchions; i++) {
      const sx = room.minX + 1.0 + ((w - 2.0) * i) / stanchions + this.originOffsetX;
      this.addBox(this.stationRoot, new THREE.Vector3(sx, elev + 0.51, railZ), new THREE.Vector3(0.05, 1.02, 0.05), railMat, false);
    }

    // Berth Gallery Airlock Boarding Hatch (`+ship/board`), inside the gallery
    const airlockPos = new THREE.Vector3(room.minX + 8 + this.originOffsetX, elev + 1.35, this.atlasZ(room.minY + 0.25));
    const hatchMat = new THREE.MeshStandardMaterial({ color: 0x22553a, emissive: 0x22cc66, emissiveIntensity: 0.8 });
    const hatch = this.addBox(this.stationRoot, airlockPos, new THREE.Vector3(2.3, 2.7, 0.32), hatchMat, false);
    this.airlockBoardWi = {
      id: "station:airlock_board",
      kind: "airlock_board",
      title: "Board Chartered Ship",
      subtitle: "Cycle berth airlock (+ship/board)",
      actionOrKey: "+ship/board",
      worldPos: airlockPos.clone(),
      maxDistance: 4.2,
      meshTargets: [hatch],
    };
    this.interactables.push(this.airlockBoardWi);
  }

  private buildExteriorBerthCradles(pack: AtlasNeighborhoodPack): void {
    let gallery: AtlasRoomDef | undefined;
    for (const r of pack.rooms.values()) {
      if (r.entryId.includes("berth_gallery")) {
        gallery = r;
        break;
      }
    }
    if (!gallery) return;

    const elev = getFloorElevationY(pack, gallery.floorIndex);
    const wz = this.atlasZ(gallery.minY);
    const gantryMat = new THREE.MeshStandardMaterial({ color: 0x28323e, roughness: 0.55, metalness: 0.68 });
    const hullMat = new THREE.MeshStandardMaterial({ color: 0x5c6a7a, roughness: 0.48, metalness: 0.52 });
    const trimMat = new THREE.MeshStandardMaterial({ color: 0x3b4858, roughness: 0.42, metalness: 0.62 });

    // Exterior starfield outside the South gallery viewport (+Z)
    const starGeo = new THREE.BufferGeometry();
    const starPts = new Float32Array(180 * 3);
    for (let i = 0; i < 180; i++) {
      starPts[i * 3] = ((i * 53) % 180) - 90;
      starPts[i * 3 + 1] = elev - 18 + ((i * 29) % 52);
      starPts[i * 3 + 2] = wz + 26 + ((i * 41) % 95);
    }
    starGeo.setAttribute("position", new THREE.BufferAttribute(starPts, 3));
    const stars = new THREE.Points(starGeo, new THREE.PointsMaterial({ color: 0xd8ecff, size: 0.35 }));
    this.stationRoot.add(stars);

    const cradleCount = 4;
    const span = gallery.maxX - gallery.minX;
    for (let c = 0; c < cradleCount; c++) {
      const cx = gallery.minX + span * ((c + 0.5) / cradleCount) + this.originOffsetX;
      const cz = wz + 17;

      // Shipyard Cradle Gantry & Docking Boom (South of gallery viewport, +Z)
      this.addBox(this.stationRoot, new THREE.Vector3(cx, elev - 2.6, cz), new THREE.Vector3(10.5, 0.55, 22), gantryMat, false);
      this.addBox(this.stationRoot, new THREE.Vector3(cx - 5.1, elev + 1.2, cz), new THREE.Vector3(0.45, 7.6, 0.45), gantryMat, false);
      this.addBox(this.stationRoot, new THREE.Vector3(cx + 5.1, elev + 1.2, cz), new THREE.Vector3(0.45, 7.6, 0.45), gantryMat, false);
      this.addBox(this.stationRoot, new THREE.Vector3(cx, elev + 4.9, cz), new THREE.Vector3(10.6, 0.45, 0.45), gantryMat, false);
      this.addBox(this.stationRoot, new THREE.Vector3(cx, elev + 1.5, wz + 6.5), new THREE.Vector3(1.1, 1.1, 12.5), trimMat, false);

      const spot = new THREE.PointLight(0x99ccff, 2.4, 28, 1.2);
      spot.position.set(cx, elev + 4.4, cz - 4);
      this.stationRoot.add(spot);

      const hull = this.buildSculptedDrayHull(hullMat, trimMat);
      hull.position.set(cx, elev - 0.1, cz);
      hull.rotation.y = Math.PI; // Nose points -Z toward gallery docking boom
      hull.visible = false;
      this.stationRoot.add(hull);
      this.cradleHullGroups.push(hull);

      const sign = this.createFloatingBannerSprite(`Cradle 0${c + 1} — Clear Berth`);
      sign.sprite.position.set(cx, elev + 3.6, cz - 7.5);
      this.stationRoot.add(sign.sprite);
      this.cradleSignSprites.push(sign);
    }
  }

  private buildSculptedDrayHull(hullMat: THREE.Material, trimMat: THREE.Material): THREE.Group {
    const g = new THREE.Group();
    this.addBox(g, new THREE.Vector3(0, 0, 0), new THREE.Vector3(5.2, 4.2, 17.5), hullMat, false);
    this.addBox(g, new THREE.Vector3(0, 0.2, 9.6), new THREE.Vector3(3.2, 3.1, 2.2), trimMat, false);
    this.addBox(g, new THREE.Vector3(-3.8, -0.2, -0.8), new THREE.Vector3(2.4, 3.4, 11.2), hullMat, false);
    this.addBox(g, new THREE.Vector3(3.8, -0.2, -0.8), new THREE.Vector3(2.4, 3.4, 11.2), hullMat, false);
    this.addBox(g, new THREE.Vector3(0, 2.8, 4.4), new THREE.Vector3(3.8, 1.6, 4.8), trimMat, false);
    return g;
  }

  private updateBerthGalleryCradles(
    ambient: DecodedRoomAmbient,
    berthDockedHulls?: ReadonlyArray<DockedHullSummary>,
    berthOwnHullName?: string | null,
    berthOwnHullDockedHere?: boolean,
  ): void {
    const ownName = berthOwnHullName ?? ambient.ownHullName;
    const ownDockedHere = berthOwnHullDockedHere ?? ambient.ownHullDockedHere;
    const dockedList = berthDockedHulls ?? ambient.dockedHulls;

    if (this.airlockBoardWi && ownName) {
      this.airlockBoardWi.title = ownDockedHere
        ? `Board ${ownName}`
        : `${ownName} (Not Cradled Here)`;
    }

    const labels: string[] = [];
    if (ownDockedHere && ownName) {
      labels.push(`Cradle 01 — ${ownName} [Your Hull]`);
    }
    for (const dh of dockedList) {
      if (!dh.name) continue;
      const upper = dh.name.toUpperCase();
      if (labels.some((l) => l.toUpperCase().includes(upper))) continue;
      const suffix = dh.mark ? ` (${dh.mark})` : "";
      labels.push(`Cradle 0${labels.length + 1} — ${dh.name}${suffix}`);
    }

    this.cradleSignSprites.forEach((entry, idx) => {
      const lbl = labels[idx] ?? `Cradle 0${idx + 1} — Clear Berth`;
      this.drawBannerCanvas(entry.canvas, entry.tex, lbl);
      const hull = this.cradleHullGroups[idx];
      // Show 3D hull strictly when a ship occupies this cradle (`W3D-L3`: no ghost hull in empty Cradle 01)
      if (hull) hull.visible = idx < labels.length;
    });
  }

  private buildStationStairs(
    pack: AtlasNeighborhoodPack,
    interFloorExits: AtlasExitDef[],
    tex: SharedPbrTextures,
  ): void {
    const stepMat = new THREE.MeshStandardMaterial({
      color: 0x627082,
      map: cloneRepeatTex(tex.deckMap, 2, 2),
      roughness: 0.55,
      metalness: 0.48,
    });
    const railMat = new THREE.MeshStandardMaterial({ color: 0x8596aa, roughness: 0.4, metalness: 0.72 });

    for (const ex of interFloorExits) {
      const st = computeInterFloorStairPlacement(pack, ex);
      if (!st) continue;
      const upper = pack.rooms.get(st.upperRoomId);
      const lower = pack.rooms.get(st.lowerRoomId);
      if (!upper || !lower) continue;

      const yTop = getFloorElevationY(pack, upper.floorIndex);
      const yBot = getFloorElevationY(pack, lower.floorIndex);

      const pBot =
        st.axis === "x"
          ? this.atlasToWorld(st.centerAx - st.halfRun, yBot, st.centerAy)
          : this.atlasToWorld(st.centerAx, yBot, st.centerAy - st.halfRun);
      const pTop =
        st.axis === "x"
          ? this.atlasToWorld(st.centerAx + st.halfRun, yTop, st.centerAy)
          : this.atlasToWorld(st.centerAx, yTop, st.centerAy + st.halfRun);

      const steps = 14;
      const treadW = st.halfWidth * 1.8;
      const treadD = Math.max(0.36, (st.halfRun * 2) / steps + 0.12);
      const stepSize =
        st.axis === "x"
          ? new THREE.Vector3(treadD, 0.28, treadW)
          : new THREE.Vector3(treadW, 0.28, treadD);

      for (let i = 0; i < steps; i++) {
        const t = (i + 0.5) / steps;
        const pos = new THREE.Vector3().lerpVectors(pBot, pTop, t);
        this.addBox(this.stationRoot, pos, stepSize, stepMat, false);
      }

      // Brushed-steel stair handrails
      const railOff = st.halfWidth * 0.92;
      for (const side of [-railOff, railOff]) {
        const offsetVec =
          st.axis === "x"
            ? new THREE.Vector3(0, 0.95, side)
            : new THREE.Vector3(side, 0.95, 0);
        const rBot = pBot.clone().add(offsetVec);
        const rTop = pTop.clone().add(offsetVec);
        const mid = rBot.clone().add(rTop).multiplyScalar(0.5);
        const len = rBot.distanceTo(rTop);
        const rail = this.addBox(this.stationRoot, mid, new THREE.Vector3(0.05, 0.05, len), railMat, false);
        rail.lookAt(rTop);
      }

      const cx = st.centerAx + this.originOffsetX;
      const cz = this.atlasZ(st.centerAy);
      this.stationStairs.push({
        minX: st.axis === "x" ? Math.min(pBot.x, pTop.x) - 0.3 : cx - st.halfWidth,
        maxX: st.axis === "x" ? Math.max(pBot.x, pTop.x) + 0.3 : cx + st.halfWidth,
        minZ: st.axis === "x" ? cz - st.halfWidth : Math.min(pBot.z, pTop.z) - 0.3,
        maxZ: st.axis === "x" ? cz + st.halfWidth : Math.max(pBot.z, pTop.z) + 0.3,
        pA: pBot,
        pB: pTop,
      });
    }
  }

  private buildStationDesksAndPods(pack: AtlasNeighborhoodPack): void {
    const deskMat = new THREE.MeshStandardMaterial({ color: 0x243244, roughness: 0.42, metalness: 0.58 });
    const screenMat = new THREE.MeshStandardMaterial({ color: 0x164466, emissive: 0x2299dd, emissiveIntensity: 1.1 });

    const placeServiceDesk = (
      roomId: string,
      deskKey: string,
      title: string,
      subtitle: string,
    ): void => {
      const room = pack.rooms.get(roomId);
      if (!room) return;
      const elev = getFloorElevationY(pack, room.floorIndex);
      const pos = this.atlasToWorld((room.minX + room.maxX) * 0.5, elev, (room.minY + room.maxY) * 0.5 - 2.2);
      const base = this.addBox(this.stationRoot, pos.clone().add(new THREE.Vector3(0, 0.52, 0)), new THREE.Vector3(3.2, 1.04, 1.1), deskMat, true);
      const mon = this.addBox(this.stationRoot, pos.clone().add(new THREE.Vector3(0, 1.28, -0.15)), new THREE.Vector3(1.6, 0.55, 0.08), screenMat, false);
      const sign = this.createFloatingBannerSprite(title.toUpperCase());
      sign.sprite.position.set(pos.x, elev + 2.05, pos.z);
      sign.sprite.scale.set(2.8, 0.72, 1);
      this.stationRoot.add(sign.sprite);

      this.interactables.push({
        id: `desk:${roomId}:${deskKey}`,
        kind: "desk",
        title,
        subtitle,
        actionOrKey: deskKey,
        worldPos: pos,
        maxDistance: 4.2,
        meshTargets: [base, mon],
      });
    };

    for (const r of pack.rooms.values()) {
      const deskSpec = classifyStationDesk(pack.packId, r);
      if (deskSpec) {
        placeServiceDesk(r.entryId, deskSpec.deskKey, deskSpec.title, deskSpec.subtitle);
        continue;
      }
      const low = `${r.entryId} ${r.name}`.toLowerCase();
      if (low.includes("sim_red") || low.includes("sim_blue")) {
        const isRed = low.includes("sim_red");
        const teamRamp = isRed ? RED_TEAM_RAMP : BLUE_TEAM_RAMP;
        const simModels = ["vf.mech.brawler_med", "vf.mech.assault_hvy", "vf.mech.recon_lt"];
        const podCol = isRed ? 0x5c2222 : 0x203e66;
        const podMat = new THREE.MeshStandardMaterial({ color: podCol, roughness: 0.45, metalness: 0.62 });
        const elev = getFloorElevationY(pack, r.floorIndex);
        for (let p = 0; p < 6; p++) {
          const row = Math.floor(p / 3);
          const col = p % 3;
          const px = r.minX + 6 + col * 7.5;
          const py = r.minY + 5.5 + row * 8.5;
          const wPos = this.atlasToWorld(px, elev, py);
          const plinth = this.addBox(
            this.stationRoot,
            wPos.clone().add(new THREE.Vector3(0, 0.14, 0)),
            new THREE.Vector3(2.3, 0.28, 2.3),
            podMat,
            true,
          );
          const mupkUnit = createAsyncMupkUnitGroup(
            simModels[p % simModels.length]!,
            "sim_mech",
            teamRamp,
            2.25,
          );
          mupkUnit.group.position.copy(wPos).add(new THREE.Vector3(0, 0.28, 0));
          mupkUnit.group.rotation.y = row === 0 ? 0 : Math.PI;
          this.stationRoot.add(mupkUnit.group);
          this.interactables.push({
            id: `pod:${r.entryId}:${p + 1}`,
            kind: "sim_pod",
            title: `Board ${isRed ? "Red" : "Blue"} Pod ${p + 1}`,
            subtitle: "Climb into armored simulator cockpit (board)",
            actionOrKey: "board",
            worldPos: wPos,
            maxDistance: 3.8,
            meshTargets: [plinth, mupkUnit.hitTarget, mupkUnit.group],
          });
        }
      }
    }
  }

  // -- Ship Interior Builder ------------------------------------------------

  private buildShipInterior(tpl: ShipUnitTemplate, sig = computeShipTemplateSignature(tpl)): void {
    this.activeShipTemplate = tpl;
    this.builtShipSig = sig;

    this.clearAndDisposeGroup(this.shipRoot);
    this.shipWallObstacles.length = 0;
    this.shipStairs.length = 0;
    for (let i = this.shipInteractables.length - 1; i >= 0; i--) {
      if (!this.shipInteractables[i]?.id.startsWith("actor:")) {
        this.shipInteractables.splice(i, 1);
      }
    }
    this.holotankContactsGroup = null;
    this.holotankStatusBanner = null;
    this.cargoPalletsGroup = null;
    this.lastCargoStacks = -1;
    this.lastHolotankFeed = TacsimStage3D.HOLO_UNSET;
    this.airlockLeaveWi = null;

    const tex = getOrBuildProceduralTextures();
    const frameMat = new THREE.MeshStandardMaterial({ color: 0x566577, roughness: 0.42, metalness: 0.68 });

    for (const comp of tpl.compartments.values()) {
      const floorY = comp.localCenter.y - comp.localSize.y * 0.5;
      const ceilY = comp.localCenter.y + comp.localSize.y * 0.5;
      const minX = comp.localCenter.x - comp.localSize.x * 0.5;
      const maxX = comp.localCenter.x + comp.localSize.x * 0.5;
      const minZ = comp.localCenter.z - comp.localSize.z * 0.5;
      const maxZ = comp.localCenter.z + comp.localSize.z * 0.5;

      const deckMat = new THREE.MeshStandardMaterial({
        color: comp.id === "hold" || comp.id === "bay" ? 0x48586e : 0x5c6b7d,
        map: cloneRepeatTex(tex.deckMap, comp.localSize.x / 2.6, comp.localSize.z / 2.6),
        normalMap: cloneRepeatTex(tex.deckNormal, comp.localSize.x / 2.6, comp.localSize.z / 2.6),
        roughness: 0.58,
        metalness: 0.48,
      });
      const sideMat = new THREE.MeshStandardMaterial({
        color: 0x6e7d8f,
        map: cloneRepeatTex(tex.bulkheadMap, comp.localSize.z / 3.8, 1),
        normalMap: cloneRepeatTex(tex.bulkheadNormal, comp.localSize.z / 3.8, 1),
        roughness: 0.64,
        metalness: 0.38,
      });
      const ceilMat = new THREE.MeshStandardMaterial({
        color: 0x4d5866,
        map: cloneRepeatTex(tex.ceilingMap, comp.localSize.x / 3.2, comp.localSize.z / 3.2),
        roughness: 0.88,
        metalness: 0.18,
      });

      let floorCut: RectXZ | null = null;
      if (comp.id === "passage" || comp.id === "corridor") {
        floorCut = { ...HOLD_STAIRWELL_XZ, maxZ: HOLD_STAIRWELL_XZ.maxZ - 0.06 };
      } else if (comp.id === "bridge") {
        floorCut = { minX: BRIDGE_STAIRWELL_XZ.minX, maxX: BRIDGE_STAIRWELL_XZ.maxX, minZ, maxZ: BRIDGE_STAIRWELL_XZ.maxZ - 0.06 };
      }
      this.addSlabWithCutout(this.shipRoot, floorY - 0.12, 0.24, minX, maxX, minZ, maxZ, floorCut, deckMat);

      let ceilCut: RectXZ | null = null;
      if (comp.id === "passage" || comp.id === "corridor") ceilCut = BRIDGE_STAIRWELL_XZ;
      else if (comp.id === "hold") ceilCut = HOLD_STAIRWELL_XZ;
      this.addSlabWithCutout(this.shipRoot, ceilY + 0.06, 0.12, minX, maxX, minZ, maxZ, ceilCut, ceilMat);

      // Port & Starboard bulkheads
      this.addShipBox(new THREE.Vector3(minX, comp.localCenter.y, comp.localCenter.z), new THREE.Vector3(0.22, comp.localSize.y, comp.localSize.z), sideMat, true);
      this.addShipBox(new THREE.Vector3(maxX, comp.localCenter.y, comp.localCenter.z), new THREE.Vector3(0.22, comp.localSize.y, comp.localSize.z), sideMat, true);

      if (comp.id === "airlock" || comp.id === "hold" || comp.id === "bay") {
        this.addShipBox(new THREE.Vector3(comp.localCenter.x, comp.localCenter.y, maxZ), new THREE.Vector3(comp.localSize.x, comp.localSize.y, 0.24), sideMat, true);
      } else if (comp.id === "bridge") {
        this.buildBridgeCanopy(comp, floorY, ceilY, maxZ, sideMat, frameMat);
        // Framed aft bulkhead around `BRIDGE_STAIRWELL_XZ` with solid side panels (`W3D-M5`)
        const openMinX = Math.max(minX, BRIDGE_STAIRWELL_XZ.minX);
        const openMaxX = Math.min(maxX, BRIDGE_STAIRWELL_XZ.maxX);
        if (openMinX - minX > 0.1) {
          this.addShipBox(
            new THREE.Vector3((minX + openMinX) * 0.5, comp.localCenter.y, minZ),
            new THREE.Vector3(openMinX - minX, comp.localSize.y, 0.22),
            sideMat,
            true,
          );
        }
        if (maxX - openMaxX > 0.1) {
          this.addShipBox(
            new THREE.Vector3((openMaxX + maxX) * 0.5, comp.localCenter.y, minZ),
            new THREE.Vector3(maxX - openMaxX, comp.localSize.y, 0.22),
            sideMat,
            true,
          );
        }
        if (openMaxX - openMinX > 0.2) {
          const lintelH = 0.36;
          this.addShipBox(
            new THREE.Vector3((openMinX + openMaxX) * 0.5, ceilY - lintelH * 0.5, minZ),
            new THREE.Vector3(openMaxX - openMinX, lintelH, 0.22),
            frameMat,
            false,
          );
        }
      }

      // Always seal `crew`, `engineering`, and `bay` aft bulkheads (`W3D-M5`: prevents walking across
      // the 0.5 m gap between `engineering` and `bay`, since `bay` is accessed via `+tv/bay in` from `hold`).
      if (comp.id === "crew" || comp.id === "engineering" || comp.id === "bay") {
        this.addShipBox(new THREE.Vector3(comp.localCenter.x, comp.localCenter.y, minZ), new THREE.Vector3(comp.localSize.x, comp.localSize.y, 0.24), sideMat, true);
      }

      // Recessed luminaire & compartment point light
      const lCol = comp.id === "hold" ? 0x72b8ff : comp.id === "crew" || comp.id === "engineering" ? 0xffc888 : 0xd8f0ff;
      const diffMat = new THREE.MeshStandardMaterial({ color: lCol, emissive: lCol, emissiveIntensity: 1.9 });
      this.addShipBox(new THREE.Vector3(comp.localCenter.x, ceilY - 0.05, comp.localCenter.z), new THREE.Vector3(0.5, 0.05, 1.8), diffMat, false);
      const pt = new THREE.PointLight(lCol, 2.3, 14, 1.3);
      pt.position.set(comp.localCenter.x, ceilY - 0.38, comp.localCenter.z);
      this.shipRoot.add(pt);

      // Uniform-scale compartment Gauge Slate
      this.buildDiegeticWallSign(
        this.shipRoot,
        `${comp.name.toUpperCase()}\n${comp.shortDesc}`,
        new THREE.Vector3(minX + 0.16, floorY + 1.65, comp.localCenter.z),
        Math.PI * 0.5,
        2.35,
        0.95,
      );

      this.dressShipCompartment(comp, floorY);
    }

    // Companionway ramps + handrails
    this.buildShipRamp(new THREE.Vector3(-1.6, -1.6, 2.5), new THREE.Vector3(-1.6, 1.8, 6.2), 1.6, frameMat);
    this.buildShipRamp(new THREE.Vector3(1.6, -1.6, 3.5), new THREE.Vector3(1.6, -5.3, 0.2), 1.6, frameMat);

    // Airlock outer hatch (`+ship/leave`)
    const airlock = tpl.compartments.get("airlock");
    if (airlock) {
      const fy = airlock.localCenter.y - airlock.localSize.y * 0.5;
      const localPos = new THREE.Vector3(0, fy + 1.4, airlock.localCenter.z + airlock.localSize.z * 0.5 - 0.2);
      const hatchMat = new THREE.MeshStandardMaterial({ color: 0x22553a, emissive: 0x28dd77, emissiveIntensity: 0.9 });
      const hatch = this.addShipBox(localPos, new THREE.Vector3(2.1, 2.5, 0.3), hatchMat, false);
      this.airlockLeaveWi = {
        id: "ship:airlock_leave",
        kind: "airlock_leave",
        title: "Cycle Outer Airlock Ashore",
        subtitle: "Step out into the Berth Gallery (+ship/leave)",
        actionOrKey: "+ship/leave",
        worldPos: localPos.clone().add(this.shipOrigin),
        maxDistance: 3.8,
        meshTargets: [hatch],
      };
      this.shipInteractables.push(this.airlockLeaveWi);
    }

    // Bridge Holo-Tank & Helm Couch (`seat:helm`)
    const bridge = tpl.compartments.get("bridge");
    if (bridge) {
      const fy = bridge.localCenter.y - bridge.localSize.y * 0.5;
      const pedMat = new THREE.MeshStandardMaterial({ color: 0x1e2938, roughness: 0.45, metalness: 0.62 });
      this.addShipBox(new THREE.Vector3(0, fy + 0.52, bridge.localCenter.z + 0.4), new THREE.Vector3(1.7, 1.04, 1.7), pedMat, true);
      const disc = new THREE.Mesh(
        new THREE.CylinderGeometry(0.75, 0.75, 0.05, 24),
        new THREE.MeshBasicMaterial({ color: 0x28b8ff, transparent: true, opacity: 0.38 }),
      );
      disc.position.set(0, fy + 1.08, bridge.localCenter.z + 0.4);
      this.shipRoot.add(disc);

      this.holotankContactsGroup = new THREE.Group();
      this.holotankContactsGroup.position.set(0, fy + 1.45, bridge.localCenter.z + 0.4);
      this.shipRoot.add(this.holotankContactsGroup);
      const statusBanner = this.createFloatingBannerSprite("HOLOTANK STANDBY — NO TELEMETRY");
      statusBanner.sprite.position.set(0, fy + 2.02, bridge.localCenter.z + 0.4);
      statusBanner.sprite.scale.set(2.2, 0.42, 1);
      this.shipRoot.add(statusBanner.sprite);
      this.holotankStatusBanner = {
        ...statusBanner,
        lastText: "HOLOTANK STANDBY — NO TELEMETRY",
      };
      this.updateHolotank(null);

      const couchPos = new THREE.Vector3(0, fy + 0.45, bridge.localCenter.z + 2.5);
      const couchMat = new THREE.MeshStandardMaterial({ color: 0x2e4562, emissive: 0x1e78c8, emissiveIntensity: 0.5 });
      const couch = this.addShipBox(couchPos, new THREE.Vector3(1.15, 0.88, 1.15), couchMat, true);
      this.shipInteractables.push({
        id: "ship:helm_seat",
        kind: "helm_seat",
        title: "Take Bridge Helm & Flight Visor",
        subtitle: "Open orbital flight & 60 Hz docking controls (+helm/station command)",
        actionOrKey: "seat:helm",
        worldPos: couchPos.clone().add(this.shipOrigin),
        maxDistance: 3.8,
        meshTargets: [couch],
      });
    }
  }

  private buildBridgeCanopy(
    comp: ShipCompartmentDef,
    floorY: number,
    ceilY: number,
    maxZ: number,
    bulkheadMat: THREE.Material,
    frameMat: THREE.Material,
  ): void {
    const w = comp.localSize.x;
    const h = comp.localSize.y;
    const kneeH = 0.92;
    const headH = 0.48;
    const glassH = Math.max(1.0, h - kneeH - headH);
    const glassY = floorY + kneeH + glassH * 0.5;

    this.addShipBox(new THREE.Vector3(comp.localCenter.x, floorY + kneeH * 0.5, maxZ), new THREE.Vector3(w, kneeH, 0.24), bulkheadMat, false);
    this.addShipBox(new THREE.Vector3(comp.localCenter.x, ceilY - headH * 0.5, maxZ), new THREE.Vector3(w, headH, 0.24), bulkheadMat, false);

    const glassMat = new THREE.MeshPhysicalMaterial({ color: 0x88ccff, transparent: true, opacity: 0.14, roughness: 0.15 });
    this.addShipBox(new THREE.Vector3(comp.localCenter.x, glassY, maxZ), new THREE.Vector3(w - 0.3, glassH, 0.10), glassMat, true);

    for (const mx of [-w * 0.48, -w * 0.18, w * 0.18, w * 0.48]) {
      this.addShipBox(new THREE.Vector3(comp.localCenter.x + mx, glassY, maxZ - 0.02), new THREE.Vector3(0.14, glassH + 0.06, 0.2), frameMat, false);
    }

    const starGeo = new THREE.BufferGeometry();
    const pts = new Float32Array(42 * 3);
    for (let i = 0; i < 42; i++) {
      pts[i * 3] = ((i * 37) % 44) - 22;
      pts[i * 3 + 1] = floorY - 2 + ((i * 19) % 18) * 0.85;
      pts[i * 3 + 2] = maxZ + 16 + ((i * 29) % 34);
    }
    starGeo.setAttribute("position", new THREE.BufferAttribute(pts, 3));
    this.shipRoot.add(new THREE.Points(starGeo, new THREE.PointsMaterial({ color: 0xe6f2ff, size: 0.25 })));
  }

  private dressShipCompartment(comp: ShipCompartmentDef, floorY: number): void {
    if (comp.id === "hold") {
      this.cargoPalletsGroup = new THREE.Group();
      this.shipRoot.add(this.cargoPalletsGroup);
      this.updateCargoFill(20);
    } else if (comp.id === "crew") {
      const bunkMat = new THREE.MeshStandardMaterial({ color: 0x38485c, roughness: 0.65 });
      this.addShipBox(new THREE.Vector3(comp.localCenter.x + 2.4, floorY + 0.45, comp.localCenter.z - 1.2), new THREE.Vector3(1.4, 0.42, 2.3), bunkMat, true);
      this.addShipBox(new THREE.Vector3(comp.localCenter.x + 2.4, floorY + 1.50, comp.localCenter.z - 1.2), new THREE.Vector3(1.4, 0.28, 2.3), bunkMat, false);
    } else if (comp.id === "bay") {
      const cradleMat = new THREE.MeshStandardMaterial({ color: 0x222830, roughness: 0.72, metalness: 0.4 });
      for (const slot of [1, 2]) {
        const rx = slot === 1 ? -2.25 : 2.25;
        const rz = comp.localCenter.z - 0.6;
        const cradle = this.addShipBox(new THREE.Vector3(rx, floorY + 0.12, rz), new THREE.Vector3(2.4, 0.24, 4.2), cradleMat, true);
        const rigUnit = createAsyncMupkUnitGroup(
          slot === 1 ? "mc.vh.bug_rover" : "mc.vh.bug_truck",
          "prospector_rover",
          AMBER_RIG_RAMP,
          1.72,
        );
        rigUnit.group.position.set(rx, floorY + 0.24, rz);
        this.shipRoot.add(rigUnit.group);
        this.shipInteractables.push({
          id: `ship:rig:${slot}`,
          kind: "vehicle_rig",
          title: `Operate Prospector Rig ${slot}`,
          subtitle: "Launch down stern ramp or lease rig (+tv)",
          actionOrKey: "ui:open_rig",
          worldPos: new THREE.Vector3(rx, floorY + 0.85, rz).add(this.shipOrigin),
          maxDistance: 4.4,
          meshTargets: [cradle, rigUnit.hitTarget, rigUnit.group],
        });
      }
    }
  }

  private buildShipRamp(pBot: THREE.Vector3, pTop: THREE.Vector3, width: number, railMat: THREE.Material): void {
    const stepMat = new THREE.MeshStandardMaterial({ color: 0x526072, roughness: 0.52, metalness: 0.55 });
    const steps = 12;
    for (let i = 0; i < steps; i++) {
      const t = (i + 0.5) / steps;
      const p = new THREE.Vector3().lerpVectors(pBot, pTop, t);
      this.addShipBox(p, new THREE.Vector3(width, 0.24, 0.54), stepMat, false);
    }
    for (const side of [-width * 0.5 - 0.04, width * 0.5 + 0.04]) {
      const rA = pBot.clone().add(new THREE.Vector3(side, 0.94, 0));
      const rB = pTop.clone().add(new THREE.Vector3(side, 0.94, 0));
      const mid = rA.clone().add(rB).multiplyScalar(0.5);
      const rail = this.addShipBox(mid, new THREE.Vector3(0.05, 0.05, rA.distanceTo(rB)), railMat, false);
      rail.lookAt(rB.clone().add(this.shipOrigin));
    }
    this.shipStairs.push({
      minX: Math.min(pBot.x, pTop.x) - width * 0.55,
      maxX: Math.max(pBot.x, pTop.x) + width * 0.55,
      minZ: Math.min(pBot.z, pTop.z) - 0.35,
      maxZ: Math.max(pBot.z, pTop.z) + 0.35,
      pA: pBot.clone().add(this.shipOrigin),
      pB: pTop.clone().add(this.shipOrigin),
    });
  }

  private getOrCreateHoloMat(hex: number): THREE.MeshBasicMaterial {
    let mat = this.holoBlipMats.get(hex);
    if (!mat) {
      mat = new THREE.MeshBasicMaterial({ color: hex });
      this.holoBlipMats.set(hex, mat);
      this.sharedPoolMats.add(mat);
    }
    return mat;
  }

  private updateCargoFill(loadedTonnes: number): void {
    if (!this.cargoPalletsGroup || !this.activeShipTemplate) return;
    const hold = this.activeShipTemplate.compartments.get("hold");
    if (!hold) return;
    const stacks = Math.max(0, Math.min(8, Math.floor((loadedTonnes + 11) / 12)));
    if (stacks === this.lastCargoStacks && this.cargoPalletsGroup.children.length === stacks) {
      return;
    }
    this.lastCargoStacks = stacks;
    while (this.cargoPalletsGroup.children.length > 0) {
      this.cargoPalletsGroup.remove(this.cargoPalletsGroup.children[0]!);
    }
    const floorY = hold.localCenter.y - hold.localSize.y * 0.5;
    for (let i = 0; i < stacks; i++) {
      const sx = i % 2 === 0 ? -2.8 : 2.8;
      const sz = hold.localCenter.z - 3.6 + Math.floor(i / 2) * 2.4;
      const mesh = new THREE.Mesh(this.cargoCrateGeo, this.cargoCrateMat);
      mesh.position.set(sx, floorY + 0.58, sz);
      this.cargoPalletsGroup.add(mesh);
    }
  }

  private setHolotankStatus(text: string): void {
    const banner = this.holotankStatusBanner;
    if (!banner || banner.lastText === text) return;
    banner.lastText = text;
    this.drawBannerCanvas(banner.canvas, banner.tex, text);
  }

  private updateHolotank(holoFeed: unknown): void {
    if (!this.holotankContactsGroup) return;
    if (holoFeed === this.lastHolotankFeed) {
      return;
    }
    this.lastHolotankFeed = holoFeed;
    while (this.holotankContactsGroup.children.length > 0) {
      this.holotankContactsGroup.remove(this.holotankContactsGroup.children[0]!);
    }

    const obj =
      typeof holoFeed === "object" && holoFeed !== null && !Array.isArray(holoFeed)
        ? (holoFeed as Record<string, unknown>)
        : null;
    const contacts = obj && Array.isArray(obj["contacts"]) ? obj["contacts"] : null;
    if (!obj || obj["refused"] === true || obj["closed"] === true || !contacts) {
      this.setHolotankStatus("HOLOTANK STANDBY — NO TELEMETRY");
      return;
    }

    const addBlip = (
      x: number,
      y: number,
      z: number,
      r: number,
      hex: number,
      kind: "self" | "contact",
    ): void => {
      const m = new THREE.Mesh(this.holoBlipGeo, this.getOrCreateHoloMat(hex));
      m.scale.setScalar(r);
      m.position.set(x, y, z);
      m.userData["holoKind"] = kind;
      this.holotankContactsGroup!.add(m);
    };

    // Resolve observer position in bubble-absolute coordinates so all contacts plot in
    // the same observer-centered frame as the own-ship center marker.
    let obsX = 0;
    let obsY = 0;
    let obsZ = 0;
    for (const c of contacts) {
      if (typeof c !== "object" || c === null) continue;
      const rec = c as Record<string, unknown>;
      if (rec["you"] === true) {
        obsX = typeof rec["x_km"] === "number" && Number.isFinite(rec["x_km"]) ? rec["x_km"] : 0;
        obsY = typeof rec["y_km"] === "number" && Number.isFinite(rec["y_km"]) ? rec["y_km"] : 0;
        obsZ = typeof rec["z_km"] === "number" && Number.isFinite(rec["z_km"]) ? rec["z_km"] : 0;
        break;
      }
    }

    addBlip(0, 0, 0, 0.06, 0x44ffaa, "self");

    const rawRange = typeof obj["range_km"] === "number" && Number.isFinite(obj["range_km"]) ? obj["range_km"] : 1000;
    const rangeKm = Math.max(1, rawRange);
    let plottedContacts = 0;
    for (const c of contacts) {
      if (typeof c !== "object" || c === null) continue;
      const rec = c as Record<string, unknown>;
      if (rec["you"] === true) continue;
      const cx = typeof rec["x_km"] === "number" && Number.isFinite(rec["x_km"]) ? rec["x_km"] : 0;
      const cy = typeof rec["y_km"] === "number" && Number.isFinite(rec["y_km"]) ? rec["y_km"] : 0;
      const cz = typeof rec["z_km"] === "number" && Number.isFinite(rec["z_km"]) ? rec["z_km"] : 0;
      let x = ((cx - obsX) / rangeKm) * 0.65;
      let z = ((cy - obsY) / rangeKm) * 0.65;
      let y = ((cz - obsZ) / rangeKm) * 0.45;
      const horizR = Math.hypot(x, z);
      if (horizR > 0.72) {
        const s = 0.72 / horizR;
        x *= s;
        z *= s;
      }
      y = Math.max(-0.45, Math.min(0.45, y));
      const rel = typeof rec["relation"] === "string" ? rec["relation"] : "neutral";
      const kind = typeof rec["kind"] === "string" ? rec["kind"] : "ship";
      const col = kind === "asteroid" || kind === "rock" ? 0xffc844 : rel === "hostile" ? 0xff5544 : 0x55ccff;
      addBlip(x, y, z, 0.045, col, "contact");
      plottedContacts += 1;
    }

    this.setHolotankStatus(
      plottedContacts === 0
        ? "HOLOTANK CLEAR — 0 CONTACTS"
        : `HOLOTANK LIVE — ${plottedContacts} CONTACT${plottedContacts === 1 ? "" : "S"}`,
    );
  }

  // -- Primitives & Plaques -------------------------------------------------

  private addSlabWithCutout(
    parent: THREE.Group,
    yCenter: number,
    thickness: number,
    minX: number,
    maxX: number,
    minZ: number,
    maxZ: number,
    cut: RectXZ | null,
    mat: THREE.Material,
  ): void {
    const mk = (x0: number, x1: number, z0: number, z1: number): void => {
      if (x1 - x0 < 0.05 || z1 - z0 < 0.05) return;
      const mesh = new THREE.Mesh(new THREE.BoxGeometry(x1 - x0, thickness, z1 - z0), mat);
      mesh.position.set((x0 + x1) * 0.5, yCenter, (z0 + z1) * 0.5);
      parent.add(mesh);
    };
    if (!cut) {
      mk(minX, maxX, minZ, maxZ);
      return;
    }
    const cMinX = Math.max(minX, Math.min(maxX, cut.minX));
    const cMaxX = Math.max(minX, Math.min(maxX, cut.maxX));
    const cMinZ = Math.max(minZ, Math.min(maxZ, cut.minZ));
    const cMaxZ = Math.max(minZ, Math.min(maxZ, cut.maxZ));
    if (cMaxX - cMinX < 0.05 || cMaxZ - cMinZ < 0.05) {
      mk(minX, maxX, minZ, maxZ);
      return;
    }
    mk(minX, maxX, minZ, cMinZ);
    mk(minX, maxX, cMaxZ, maxZ);
    mk(minX, cMinX, cMinZ, cMaxZ);
    mk(cMaxX, maxX, cMinZ, cMaxZ);
  }

  private addBox(
    parent: THREE.Group,
    pos: THREE.Vector3,
    size: THREE.Vector3,
    mat: THREE.Material,
    isObstacle: boolean,
  ): THREE.Mesh {
    const mesh = new THREE.Mesh(new THREE.BoxGeometry(size.x, size.y, size.z), mat);
    mesh.position.copy(pos);
    parent.add(mesh);
    if (isObstacle) {
      const half = size.clone().multiplyScalar(0.5);
      this.wallObstacles.push(new THREE.Box3(pos.clone().sub(half), pos.clone().add(half)));
    }
    return mesh;
  }

  private addShipBox(
    localPos: THREE.Vector3,
    size: THREE.Vector3,
    mat: THREE.Material,
    isObstacle: boolean,
  ): THREE.Mesh {
    const mesh = new THREE.Mesh(new THREE.BoxGeometry(size.x, size.y, size.z), mat);
    mesh.position.copy(localPos);
    this.shipRoot.add(mesh);
    if (isObstacle) {
      const worldCenter = localPos.clone().add(this.shipOrigin);
      const half = size.clone().multiplyScalar(0.5);
      this.shipWallObstacles.push(new THREE.Box3(worldCenter.clone().sub(half), worldCenter.clone().add(half)));
    }
    return mesh;
  }

  private buildObservationPlaque(
    parent: THREE.Group,
    keyword: string,
    bodyText: string,
    floorPos: THREE.Vector3,
    isShip: boolean,
  ): void {
    const postMat = new THREE.MeshStandardMaterial({ color: 0x263240, roughness: 0.5, metalness: 0.62 });
    const scrMat = new THREE.MeshStandardMaterial({ color: 0x164a68, emissive: 0x2899cc, emissiveIntensity: 0.9 });
    const ped = this.addBox(parent, floorPos.clone().add(new THREE.Vector3(0, 0.48, 0)), new THREE.Vector3(0.24, 0.96, 0.24), postMat, false);
    const scr = this.addBox(parent, floorPos.clone().add(new THREE.Vector3(0, 1.0, 0)), new THREE.Vector3(0.68, 0.12, 0.44), scrMat, false);

    const sign = this.createFloatingBannerSprite(`[${keyword.toUpperCase()}]`);
    sign.sprite.position.set(floorPos.x, floorPos.y + 1.48, floorPos.z);
    sign.sprite.scale.set(1.9, 0.48, 1);
    parent.add(sign.sprite);

    const wi: WorldInteractable3D = {
      id: `plaque:${keyword}:${floorPos.x.toFixed(1)}`,
      kind: "plaque",
      title: `Inspect ${keyword}`,
      subtitle: `look ${keyword}`,
      actionOrKey: `look ${keyword}`,
      detailBody: bodyText,
      worldPos: floorPos.clone(),
      maxDistance: 3.6,
      meshTargets: [ped, scr],
    };
    if (isShip) this.shipInteractables.push(wi);
    else this.interactables.push(wi);
  }

  private buildDiegeticWallSign(
    parent: THREE.Group,
    text: string,
    pos: THREE.Vector3,
    yawRad: number,
    widthM: number,
    heightM: number,
  ): void {
    const canvas = document.createElement("canvas");
    canvas.width = 512;
    canvas.height = 192;
    const ctx = this.headlessDom ? null : canvas.getContext("2d");
    if (ctx) {
      ctx.fillStyle = "#0b1420";
      ctx.fillRect(0, 0, 512, 192);
      ctx.strokeStyle = "#2c5a84";
      ctx.lineWidth = 4;
      ctx.strokeRect(4, 4, 504, 184);
      const lines = text.split("\n");
      ctx.fillStyle = "#88ddff";
      ctx.font = "bold 28px ui-monospace, monospace";
      ctx.fillText((lines[0] ?? "").slice(0, 32), 20, 46);
      ctx.fillStyle = "#cde6ff";
      ctx.font = "20px ui-sans-serif, system-ui, sans-serif";
      const sub = lines.slice(1).join(" ");
      ctx.fillText(sub.slice(0, 48), 20, 92);
      if (sub.length > 48) ctx.fillText(sub.slice(48, 96), 20, 122);
    }
    const tex = new THREE.CanvasTexture(canvas);
    tex.colorSpace = THREE.SRGBColorSpace;
    const mesh = new THREE.Mesh(
      new THREE.PlaneGeometry(widthM, heightM),
      new THREE.MeshBasicMaterial({ map: tex }),
    );
    mesh.position.copy(pos);
    mesh.rotation.y = yawRad;
    parent.add(mesh);
  }

  private createFloatingBannerSprite(text: string): {
    sprite: THREE.Sprite;
    canvas: HTMLCanvasElement;
    tex: THREE.CanvasTexture;
  } {
    const canvas = document.createElement("canvas");
    canvas.width = 512;
    canvas.height = 96;
    const tex = new THREE.CanvasTexture(canvas);
    tex.colorSpace = THREE.SRGBColorSpace;
    this.drawBannerCanvas(canvas, tex, text);
    const sprite = new THREE.Sprite(new THREE.SpriteMaterial({ map: tex, transparent: true }));
    sprite.scale.set(4.8, 0.9, 1);
    return { sprite, canvas, tex };
  }

  private drawBannerCanvas(canvas: HTMLCanvasElement, tex: THREE.CanvasTexture, text: string): void {
    if (this.headlessDom) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    ctx.fillStyle = "rgba(8, 16, 28, 0.82)";
    ctx.beginPath();
    ctx.roundRect(8, 12, canvas.width - 16, canvas.height - 24, 10);
    ctx.fill();
    ctx.fillStyle = "#9ad8ff";
    ctx.font = "bold 24px ui-monospace, monospace";
    ctx.textAlign = "center";
    ctx.fillText(text.toUpperCase().slice(0, 42), canvas.width * 0.5, 58);
    tex.needsUpdate = true;
  }

  // -- Spawns & Room Highlights ---------------------------------------------

  private getStationRoomSpawn(entryId: string): THREE.Vector3 {
    const pack = this.activeStationPack;
    const room = pack?.rooms.get(entryId);
    if (!pack || !room) return new THREE.Vector3(0, 6, 0);
    const elev = getFloorElevationY(pack, room.floorIndex);
    const cx = (room.minX + room.maxX) * 0.5;
    const cy = (room.minY + room.maxY) * 0.5;
    if (entryId === "room.c03_arrivals") return this.atlasToWorld(cx + 3, elev, cy + 2);
    return this.atlasToWorld(cx, elev, cy);
  }

  private getShipCompartmentSpawn(compId: string): THREE.Vector3 {
    const tpl = this.activeShipTemplate;
    const comp = tpl ? resolveCompartmentDef(tpl, compId) : null;
    if (!comp) return this.shipOrigin.clone().add(new THREE.Vector3(0, -1.6, 13));
    return this.shipOrigin.clone().add(
      new THREE.Vector3(comp.localCenter.x, comp.localCenter.y - comp.localSize.y * 0.5, comp.localCenter.z),
    );
  }

  private highlightStationRoom(entryId: string): void {
    this.activeStationRoomEntry = entryId;
    for (const [id, lt] of this.roomLights) {
      lt.intensity = id === entryId ? 3.4 : 1.35;
    }
  }

  private teleportPlayer(worldPos: THREE.Vector3, yawRad: number): void {
    this.playerGroup.position.copy(worldPos);
    this.playerGroup.rotation.y = yawRad;
    this.camYawRad = yawRad;
  }

  // -- Frame Loop, Movement, Collision & Sync -------------------------------

  private readonly onFrame = (nowMs: number): void => {
    if (this.disposed || this.contextLost) {
      this.rafId = 0;
      return;
    }
    this.rafId = requestAnimationFrame(this.onFrame);
    const dt = Math.min(0.08, Math.max(0.001, (nowMs - this.lastFrameMs) / 1000));
    this.lastFrameMs = nowMs;

    this.stepPlayerLocomotion(dt);
    this.stepActors(dt);
    if (this.holotankContactsGroup) {
      this.holotankContactsGroup.rotation.y += dt * 0.28;
    }
    this.updateCameraTransform();
    this.updateFocusedInteractable();
    this.sync3DPositionWithServer(nowMs);

    if (this.renderer) {
      this.renderer.render(this.scene, this.camera);
    }
  };

  private stepPlayerLocomotion(dt: number): void {
    const blocked =
      this.cb.isUiBlockingInput() ||
      (typeof document !== "undefined" && isTextField(document.activeElement));

    let fwdIn = 0;
    let rightIn = 0;
    let sprint = false;
    if (!blocked) {
      if (this.keysDown.has("KeyW") || this.keysDown.has("ArrowUp")) fwdIn += 1;
      if (this.keysDown.has("KeyS") || this.keysDown.has("ArrowDown")) fwdIn -= 1;
      if (this.keysDown.has("KeyD") || this.keysDown.has("ArrowRight")) rightIn += 1;
      if (this.keysDown.has("KeyA") || this.keysDown.has("ArrowLeft")) rightIn -= 1;
      sprint = this.keysDown.has("ShiftLeft") || this.keysDown.has("ShiftRight");
    }

    const speed = sprint ? 6.8 : 4.3;
    const { dx: moveX, dz: moveZ } = computeRightHandedMoveDelta(
      this.camYawRad,
      fwdIn,
      rightIn,
      speed,
      dt,
    );
    if (Math.abs(moveX) > 1e-6 || Math.abs(moveZ) > 1e-6) {
      this.tryMovePlayerAxis(moveX, 0);
      this.tryMovePlayerAxis(0, moveZ);
      this.snapPlayerToDeckOrStair();

      this.playerGroup.rotation.y = Math.atan2(moveX, moveZ);
      this.playerWalkPhase += dt * (sprint ? 12 : 8.2);
      const swing = Math.sin(this.playerWalkPhase) * (sprint ? 0.58 : 0.42);
      this.playerLeftLeg.rotation.x = -swing;
      this.playerRightLeg.rotation.x = swing;
      this.playerLeftArm.rotation.x = swing * 0.7;
      this.playerRightArm.rotation.x = -swing * 0.7;
    } else {
      const decay = Math.exp(-12 * dt);
      this.playerLeftLeg.rotation.x *= decay;
      this.playerRightLeg.rotation.x *= decay;
      this.playerLeftArm.rotation.x *= decay;
      this.playerRightArm.rotation.x *= decay;
    }
  }

  private tryMovePlayerAxis(dx: number, dz: number): void {
    const nextX = this.playerGroup.position.x + dx;
    const nextZ = this.playerGroup.position.z + dz;
    const y = this.playerGroup.position.y;
    const r = 0.36;
    this.tmpVecA.set(nextX - r, y + 0.35, nextZ - r);
    this.tmpVecB.set(nextX + r, y + 1.65, nextZ + r);
    this.tmpProbeBox.set(this.tmpVecA, this.tmpVecB);
    const list = this.isAboardShip ? this.shipWallObstacles : this.wallObstacles;
    for (const box of list) {
      if (this.tmpProbeBox.intersectsBox(box)) return;
    }
    this.playerGroup.position.x = nextX;
    this.playerGroup.position.z = nextZ;
  }

  private snapPlayerToDeckOrStair(): void {
    const pos = this.playerGroup.position;
    const stairs = this.isAboardShip ? this.shipStairs : this.stationStairs;
    for (const st of stairs) {
      if (pos.x >= st.minX && pos.x <= st.maxX && pos.z >= st.minZ && pos.z <= st.maxZ) {
        const dx = st.pB.x - st.pA.x;
        const dz = st.pB.z - st.pA.z;
        const denom = dx * dx + dz * dz;
        const rawT =
          denom > 0.0001 ? ((pos.x - st.pA.x) * dx + (pos.z - st.pA.z) * dz) / denom : 0.5;
        const t = Math.max(0, Math.min(1, rawT));
        const stairY = st.pA.y + (st.pB.y - st.pA.y) * t;
        if (Math.abs(pos.y - stairY) < 2.2) {
          pos.y = stairY;
          return;
        }
      }
    }

    if (this.isAboardShip && this.activeShipTemplate) {
      const lp = this.tmpVecA.copy(pos).sub(this.shipOrigin);
      const compId = tryFindContainingCompartment(this.activeShipTemplate, lp, this.activeShipCompartment, 0.55);
      const comp = compId ? resolveCompartmentDef(this.activeShipTemplate, compId) : null;
      if (comp) {
        pos.y = this.shipOrigin.y + comp.localCenter.y - comp.localSize.y * 0.5;
      }
    } else if (!this.isAboardShip && this.activeStationPack) {
      const rId = tryFindContainingRoomWithHysteresis(
        this.activeStationPack,
        pos,
        this.originOffsetX,
        this.originOffsetZ,
        this.activeStationRoomEntry,
      );
      const room = rId ? this.activeStationPack.rooms.get(rId) : null;
      if (room) {
        pos.y = getFloorElevationY(this.activeStationPack, room.floorIndex);
      }
    }
  }

  private stepActors(dt: number): void {
    const nowWallMs = Date.now();
    const decay = Math.exp(-10 * dt);
    for (const rec of this.spawnedActors.values()) {
      const cur = rec.group.position;
      const dist = cur.distanceTo(rec.targetPos);
      if (dist > 0.02) {
        const step = Math.min(dist, 3.2 * dt);
        const dir = this.tmpVecA.copy(rec.targetPos).sub(cur).normalize();
        cur.addScaledVector(dir, step);
        rec.group.rotation.y = Math.atan2(dir.x, dir.z);
        rec.walkPhase += dt * 7.8;
        const sw = Math.sin(rec.walkPhase) * 0.38;
        rec.leftLeg.rotation.x = -sw;
        rec.rightLeg.rotation.x = sw;
        rec.leftArm.rotation.x = sw * 0.65;
        rec.rightArm.rotation.x = -sw * 0.65;
      } else {
        rec.leftLeg.rotation.x *= decay;
        rec.rightLeg.rotation.x *= decay;
        rec.leftArm.rotation.x *= decay;
        rec.rightArm.rotation.x *= decay;
      }

      // Keep actor interactable position synced with live avatar movement
      rec.interactable.worldPos.copy(cur);

      // Automatically clear expired speech bubbles without waiting for reactive events (`W3D-L2`)
      if (rec.speechExpiresAt > 0 && nowWallMs >= rec.speechExpiresAt) {
        rec.speechExpiresAt = 0;
        this.renderActorBillboard(rec, rec.actor, "", rec.lastIsTyping);
      }

      // Clamp billboard inside safe room bounds so speech bubbles never clip into walls
      const sb = rec.safeBounds;
      const minX = sb.minX + sb.margin;
      const maxX = sb.maxX - sb.margin;
      const minZ = sb.minZ + sb.margin;
      const maxZ = sb.maxZ - sb.margin;
      const bx = minX < maxX ? Math.max(minX, Math.min(maxX, cur.x)) : (sb.minX + sb.maxX) * 0.5;
      const bz = minZ < maxZ ? Math.max(minZ, Math.min(maxZ, cur.z)) : (sb.minZ + sb.maxZ) * 0.5;
      const questBob =
        rec.questMarker && rec.questMarker.marker !== "in_progress"
          ? Math.sin(nowWallMs * 0.0045) * 0.08
          : 0;
      rec.billboardSprite.position.set(bx, cur.y + 2.45 + questBob, bz);
    }
  }

  private updateCameraTransform(): void {
    const target = this.tmpVecA.copy(this.playerGroup.position);
    target.y += 1.55;
    const hDist = Math.cos(this.camPitchRad) * this.camDist;
    const vDist = Math.sin(this.camPitchRad) * this.camDist;
    this.tmpVecB.set(
      -Math.sin(this.camYawRad) * hDist,
      vDist,
      -Math.cos(this.camYawRad) * hDist,
    );
    this.camera.position.copy(target).add(this.tmpVecB);
    this.camera.lookAt(target);
  }

  private updateFocusedInteractable(): void {
    const list = this.isAboardShip ? this.shipInteractables : this.interactables;
    const pPos = this.playerGroup.position;
    let best: WorldInteractable3D | null = null;
    let bestScore = Infinity;

    for (const wi of list) {
      const d = pPos.distanceTo(wi.worldPos);
      if (d <= wi.maxDistance && d < bestScore) {
        bestScore = d;
        best = wi;
      }
    }

    if (best !== this.focusedInteractable) {
      this.focusedInteractable = best;
      this.cb.onFocusedPromptChanged(
        best ? { title: `[E] ${best.title}`, subtitle: best.subtitle } : null,
      );
    }
  }

  private sync3DPositionWithServer(nowMs: number): void {
    if (this.awaitingInitialRoomSync) return;
    this.navWalker.expireIfStale(nowMs);
    const moveInFlight = this.navWalker.hasInFlight(nowMs);

    if (this.isAboardShip && this.activeShipTemplate && this.serverShipCompartment) {
      const lp = this.playerGroup.position.clone().sub(this.shipOrigin);
      const phys = tryFindContainingCompartment(
        this.activeShipTemplate,
        lp,
        this.activeShipCompartment,
        0.55,
      );
      if (!phys) return;
      if (phys !== this.activeShipCompartment) {
        this.activeShipCompartment = phys;
        this.cb.onPhysicalLocationChanged(phys);
      }
      if (phys === this.serverShipCompartment || moveInFlight || nowMs < this.nextMoveStepAllowedAtMs) {
        return;
      }
      const path = findShipExitPath(this.activeShipTemplate, this.serverShipCompartment, phys).map(
        (h) => ({ command: h.command, targetRoom: h.targetCompartment }),
      );
      const first = this.navWalker.planAndDispatchOne("ship", this.serverShipCompartment, path, nowMs);
      if (first) {
        this.nextMoveStepAllowedAtMs = nowMs + 350;
        this.cb.onSendCommand(first.command);
      }
    } else if (!this.isAboardShip && this.activeStationPack && this.serverStationRoomEntry) {
      const phys = tryFindContainingRoomWithHysteresis(
        this.activeStationPack,
        this.playerGroup.position,
        this.originOffsetX,
        this.originOffsetZ,
        this.activeStationRoomEntry,
      );
      if (!phys) return;
      if (phys !== this.activeStationRoomEntry) {
        this.highlightStationRoom(phys);
        this.cb.onPhysicalLocationChanged(phys);
      }
      if (phys === this.serverStationRoomEntry || moveInFlight || nowMs < this.nextMoveStepAllowedAtMs) {
        return;
      }
      const path = findStationExitPath(this.activeStationPack, this.serverStationRoomEntry, phys);
      const first = this.navWalker.planAndDispatchOne(
        "station",
        this.serverStationRoomEntry,
        path,
        nowMs,
      );
      if (first) {
        this.nextMoveStepAllowedAtMs = nowMs + 350;
        this.cb.onSendCommand(first.command);
      }
    }
  }

  // -- Input Events ---------------------------------------------------------

  private readonly onKeyDown = (ev: KeyboardEvent): void => {
    if (!this.supported || this.contextLost) return;
    if (ev.altKey || ev.ctrlKey || ev.metaKey) return;
    if (this.cb.isUiBlockingInput() || isTextField(ev.target)) return;

    if (
      ev.code === "KeyW" ||
      ev.code === "KeyA" ||
      ev.code === "KeyS" ||
      ev.code === "KeyD" ||
      ev.code === "ShiftLeft" ||
      ev.code === "ShiftRight" ||
      ev.code === "ArrowUp" ||
      ev.code === "ArrowDown" ||
      ev.code === "ArrowLeft" ||
      ev.code === "ArrowRight"
    ) {
      // Capture WASD / arrows for smooth 3D locomotion before the global exit-key listener fires
      ev.preventDefault();
      ev.stopPropagation();
      this.keysDown.add(ev.code);
      return;
    }

    if (ev.code === "KeyE" && this.focusedInteractable) {
      ev.preventDefault();
      ev.stopPropagation();
      this.triggerInteractableWithAudio(this.focusedInteractable);
    }
  };

  private triggerInteractableWithAudio(wi: WorldInteractable3D): void {
    if (wi.kind === "airlock_board" || wi.kind === "airlock_leave") {
      tacsimAudio.playAirlockCycle();
    } else {
      tacsimAudio.playUiChime();
    }
    this.flushPendingMovesBeforeAction();
    this.cb.onInteractableTriggered(wi);
  }

  private readonly onKeyUp = (ev: KeyboardEvent): void => {
    this.keysDown.delete(ev.code);
  };

  private readonly onBlur = (): void => {
    this.keysDown.clear();
    this.rmbOrbiting = false;
  };

  private readonly onPointerDown = (ev: PointerEvent): void => {
    if (!this.supported || this.contextLost || this.cb.isUiBlockingInput()) return;
    if (ev.button === 2 || ev.button === 1 || (ev.button === 0 && ev.shiftKey)) {
      ev.preventDefault();
      this.rmbOrbiting = true;
      this.lastPointerX = ev.clientX;
      this.lastPointerY = ev.clientY;
      this.canvas.setPointerCapture?.(ev.pointerId);
      return;
    }
    if (ev.button === 0) {
      const rect = this.canvas.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0) return;
      const ndc = new THREE.Vector2(
        ((ev.clientX - rect.left) / rect.width) * 2 - 1,
        -(((ev.clientY - rect.top) / rect.height) * 2 - 1),
      );
      const raycaster = new THREE.Raycaster();
      raycaster.setFromCamera(ndc, this.camera);
      const list = this.isAboardShip ? this.shipInteractables : this.interactables;
      // Pick closest raycast hit by distance (`W3D-M7`), never array-order first hit.
      let closestWi: WorldInteractable3D | null = null;
      let closestDist = Infinity;
      for (const wi of list) {
        if (this.playerGroup.position.distanceTo(wi.worldPos) > wi.maxDistance + 2.5) continue;
        const hits = raycaster.intersectObjects(wi.meshTargets, true);
        const hitDist = hits[0]?.distance;
        if (hitDist !== undefined && hitDist < closestDist) {
          closestDist = hitDist;
          closestWi = wi;
        }
      }
      if (closestWi) {
        this.triggerInteractableWithAudio(closestWi);
        return;
      }
      // Left-drag also orbits smoothly when not clicking an interactable
      this.rmbOrbiting = true;
      this.lastPointerX = ev.clientX;
      this.lastPointerY = ev.clientY;
      this.canvas.setPointerCapture?.(ev.pointerId);
    }
  };

  private readonly onPointerMove = (ev: PointerEvent): void => {
    if (!this.rmbOrbiting) return;
    const dx = ev.clientX - this.lastPointerX;
    const dy = ev.clientY - this.lastPointerY;
    this.lastPointerX = ev.clientX;
    this.lastPointerY = ev.clientY;
    this.camYawRad -= dx * 0.0065;
    this.camPitchRad = Math.max(0.08, Math.min(1.18, this.camPitchRad + dy * 0.0055));
  };

  private readonly onPointerUp = (ev: PointerEvent): void => {
    if (this.rmbOrbiting) {
      this.rmbOrbiting = false;
      this.canvas.releasePointerCapture?.(ev.pointerId);
    }
  };

  private readonly onWheel = (ev: WheelEvent): void => {
    if (!this.supported || this.contextLost || this.cb.isUiBlockingInput()) return;
    ev.preventDefault();
    this.camDist = Math.max(2.4, Math.min(11.5, this.camDist + Math.sign(ev.deltaY) * 0.55));
  };

  private readonly onContextMenu = (ev: MouseEvent): void => {
    ev.preventDefault();
  };

  private readonly onContextLost = (ev: Event): void => {
    ev.preventDefault();
    this.contextLost = true;
    if (this.rafId) {
      cancelAnimationFrame(this.rafId);
      this.rafId = 0;
    }
  };

  private readonly onContextRestored = (): void => {
    this.contextLost = false;
    if (this.disposed || !this.supported) return;
    for (const tex of sharedBaseTextures) {
      tex.needsUpdate = true;
    }
    for (const rec of this.spawnedActors.values()) {
      rec.billboardTex.needsUpdate = true;
    }
    this.builtStationSig = "";
    this.builtShipSig = "";
    if (this.activeStationPack) {
      this.buildStationWorld(
        this.activeStationPack,
        computePackGeometrySignature(this.activeStationPack),
      );
    }
    if (this.activeShipTemplate) {
      this.buildShipInterior(
        this.activeShipTemplate,
        computeShipTemplateSignature(this.activeShipTemplate),
      );
    }
    this.lastFrameMs = performance.now();
    if (!this.rafId) {
      this.rafId = requestAnimationFrame(this.onFrame);
    }
  };

  private bindDomEvents(): void {
    if (typeof window === "undefined" || this.eventsBound) return;
    this.eventsBound = true;
    window.addEventListener("keydown", this.onKeyDown, true);
    window.addEventListener("keyup", this.onKeyUp, true);
    window.addEventListener("blur", this.onBlur);
    this.canvas.addEventListener("pointerdown", this.onPointerDown);
    this.canvas.addEventListener("pointermove", this.onPointerMove);
    this.canvas.addEventListener("pointerup", this.onPointerUp);
    this.canvas.addEventListener("wheel", this.onWheel, { passive: false });
    this.canvas.addEventListener("contextmenu", this.onContextMenu);
    this.canvas.addEventListener("webglcontextlost", this.onContextLost);
    this.canvas.addEventListener("webglcontextrestored", this.onContextRestored);
  }

  private unbindDomEvents(): void {
    if (typeof window === "undefined" || !this.eventsBound) return;
    this.eventsBound = false;
    window.removeEventListener("keydown", this.onKeyDown, true);
    window.removeEventListener("keyup", this.onKeyUp, true);
    window.removeEventListener("blur", this.onBlur);
    this.canvas.removeEventListener("pointerdown", this.onPointerDown);
    this.canvas.removeEventListener("pointermove", this.onPointerMove);
    this.canvas.removeEventListener("pointerup", this.onPointerUp);
    this.canvas.removeEventListener("wheel", this.onWheel);
    this.canvas.removeEventListener("contextmenu", this.onContextMenu);
    this.canvas.removeEventListener("webglcontextlost", this.onContextLost);
    this.canvas.removeEventListener("webglcontextrestored", this.onContextRestored);
  }
}
