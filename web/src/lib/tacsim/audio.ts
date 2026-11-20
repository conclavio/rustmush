// Zero-asset procedural diegetic WebAudio synthesizer (`web/src/lib/tacsim/audio.ts`).
// Synthesizes crisp sci-fi UI chimes, market fills, pneumatic airlock pressure cycles,
// 6-DOF hard-dock clamps, radar scan pings, laser pulses, railgun cracks, and torpedo launches
// purely from deterministic sample buffers — zero external audio files required.

import { storageGet, storageSet } from "../storage";

export const TACSIM_AUDIO_PREF_KEY = "rustmush.tacsimAudio";

export type ProceduralSfxKind =
  | "ui_chime"
  | "market_fill"
  | "airlock_cycle"
  | "dock_clamp"
  | "radar_ping"
  | "laser_pulse"
  | "railgun_crack"
  | "torpedo_launch";

export function sanitizeSampleRate(sampleRate: number, fallback = 22_050): number {
  const safe = Number.isFinite(sampleRate) && sampleRate > 0 ? sampleRate : fallback;
  return Math.max(8_000, Math.min(192_000, Math.round(safe)));
}

function endFadeRamp(i: number, n: number, fadeSamples: number): number {
  const remaining = n - 1 - i;
  if (fadeSamples <= 0 || remaining >= fadeSamples) return 1;
  return Math.max(0, remaining / fadeSamples);
}

/**
 * Deterministic PCM waveform synthesis (`Float32Array` in `[-1, 1]`),
 * testable in headless Node without requiring a DOM `AudioContext`.
 */
export function computeProceduralWaveformSamples(
  kind: ProceduralSfxKind,
  sampleRate = 22_050,
): Float32Array {
  const sr = sanitizeSampleRate(sampleRate, 22_050);
  const fadeOutSamples = Math.max(1, Math.floor(sr * 0.005));

  switch (kind) {
    case "ui_chime": {
      const n = Math.max(1, Math.floor(sr * 0.085));
      const out = new Float32Array(n);
      let phase = 0;
      for (let i = 0; i < n; i++) {
        const t = i / sr;
        const attack = Math.min(1, t / 0.003);
        const env = attack * Math.exp(-t * 42) * endFadeRamp(i, n, fadeOutSamples);
        const freq = i < n / 2 ? 880 : 1174.66;
        phase += (freq / sr) * Math.PI * 2;
        out[i] = Math.sin(phase) * env * 0.42;
      }
      return out;
    }

    case "market_fill": {
      const n = Math.max(1, Math.floor(sr * 0.24));
      const out = new Float32Array(n);
      let phase1 = 0;
      let phase2 = 0;
      for (let i = 0; i < n; i++) {
        const t = i / sr;
        const second = t >= 0.09;
        const localT = second ? t - 0.09 : t;
        const attack = Math.min(1, localT / 0.004);
        const env = attack * Math.exp(-localT * 22) * endFadeRamp(i, n, fadeOutSamples);
        const f1 = second ? 783.99 : 587.33;
        const f2 = second ? 1174.66 : 880.0;
        phase1 += (f1 / sr) * Math.PI * 2;
        phase2 += (f2 / sr) * Math.PI * 2;
        out[i] = (Math.sin(phase1) * 0.32 + Math.sin(phase2) * 0.18) * env;
      }
      return out;
    }

    case "airlock_cycle": {
      const n = Math.max(1, Math.floor(sr * 0.95));
      const out = new Float32Array(n);
      let rng = 0x1234_5678;
      let lp = 0;
      const lpKeep = Math.pow(0.78, 22_050 / sr);
      const lpTake = 1 - lpKeep;
      for (let i = 0; i < n; i++) {
        const t = i / sr;
        const attack = Math.min(1, t / 0.08);
        const decay = Math.exp(-Math.max(0, t - 0.12) * 3.4);
        const clunk = Math.min(1, t / 0.003) * Math.exp(-t * 28) * Math.sin(t * 92 * Math.PI * 2) * 0.58;
        rng = (Math.imul(rng, 1_664_525) + 1_013_904_223) >>> 0;
        const white = (rng & 0xffff) / 32_768 - 1;
        lp = lp * lpKeep + white * lpTake;
        const s = (clunk + lp * attack * decay * 0.48) * endFadeRamp(i, n, fadeOutSamples);
        out[i] = Math.max(-1, Math.min(1, s));
      }
      return out;
    }

    case "dock_clamp": {
      const n = Math.max(1, Math.floor(sr * 0.85));
      const out = new Float32Array(n);
      for (let i = 0; i < n; i++) {
        const t = i / sr;
        const thudAttack = Math.min(1, t / 0.003);
        const thud = thudAttack * Math.exp(-t * 18) * Math.sin(t * 68 * Math.PI * 2) * 0.65;
        const chimeDt = t - 0.1;
        const chimeEnv =
          chimeDt > 0 ? Math.min(1, chimeDt / 0.005) * Math.exp(-chimeDt * 4.8) : 0;
        const chord =
          (Math.sin(t * 523.25 * Math.PI * 2) +
            Math.sin(t * 659.25 * Math.PI * 2) +
            Math.sin(t * 783.99 * Math.PI * 2)) *
          0.16 *
          chimeEnv;
        const s = (thud + chord) * endFadeRamp(i, n, fadeOutSamples);
        out[i] = Math.max(-1, Math.min(1, s));
      }
      return out;
    }

    case "radar_ping": {
      const n = Math.max(1, Math.floor(sr * 0.34));
      const out = new Float32Array(n);
      let phase = 0;
      for (let i = 0; i < n; i++) {
        const t = i / sr;
        const attack = Math.min(1, t / 0.004);
        const env = attack * Math.exp(-t * 11.5) * endFadeRamp(i, n, fadeOutSamples);
        const sweepHz = 1320 + 220 * Math.exp(-t * 18);
        phase += (sweepHz / sr) * Math.PI * 2;
        out[i] = Math.sin(phase) * env * 0.36;
      }
      return out;
    }

    case "laser_pulse": {
      const n = Math.max(1, Math.floor(sr * 0.19));
      const out = new Float32Array(n);
      let phase = 0;
      for (let i = 0; i < n; i++) {
        const t = i / sr;
        const freq = 1480 * Math.exp(-t * 7.5) + 220;
        phase += (freq / sr) * Math.PI * 2;
        const attack = Math.min(1, t / 0.003);
        const env = attack * Math.exp(-t * 14) * endFadeRamp(i, n, fadeOutSamples);
        out[i] = Math.sin(phase) * env * 0.4;
      }
      return out;
    }

    case "railgun_crack": {
      const n = Math.max(1, Math.floor(sr * 0.28));
      const out = new Float32Array(n);
      let rng = 0x89ab_cdef;
      let bp = 0;
      const bpKeep = Math.pow(0.62, 22_050 / sr);
      const bpTake = 1 - bpKeep;
      for (let i = 0; i < n; i++) {
        const t = i / sr;
        rng = (Math.imul(rng, 1_664_525) + 1_013_904_223) >>> 0;
        const white = (rng & 0xffff) / 32_768 - 1;
        bp = bp * bpKeep + white * bpTake;
        const attack = Math.min(1, t / 0.002);
        const crack = bp * attack * Math.exp(-t * 34) * 0.72;
        const sub = Math.sin(t * 96 * Math.PI * 2) * attack * Math.exp(-t * 16) * 0.48;
        const s = (crack + sub) * endFadeRamp(i, n, fadeOutSamples);
        out[i] = Math.max(-1, Math.min(1, s));
      }
      return out;
    }

    case "torpedo_launch": {
      const n = Math.max(1, Math.floor(sr * 0.46));
      const out = new Float32Array(n);
      let rng = 0x5150_cafe;
      let lp = 0;
      const lpKeep = Math.pow(0.85, 22_050 / sr);
      const lpTake = 1 - lpKeep;
      for (let i = 0; i < n; i++) {
        const t = i / sr;
        rng = (Math.imul(rng, 1_664_525) + 1_013_904_223) >>> 0;
        const white = (rng & 0xffff) / 32_768 - 1;
        lp = lp * lpKeep + white * lpTake;
        const tubeThump =
          Math.min(1, t / 0.003) * Math.sin(t * 74 * Math.PI * 2) * Math.exp(-t * 15) * 0.55;
        const booster = lp * Math.min(1, t / 0.06) * Math.exp(-Math.max(0, t - 0.08) * 5.2) * 0.45;
        const s = (tubeThump + booster) * endFadeRamp(i, n, fadeOutSamples);
        out[i] = Math.max(-1, Math.min(1, s));
      }
      return out;
    }
  }
}

interface WebAudioGainNodeLike {
  gain: { value: number };
  connect: (dest: unknown) => void;
}

interface WebAudioContextLike {
  readonly sampleRate: number;
  readonly state: string;
  readonly destination: unknown;
  resume?: () => Promise<void>;
  createBuffer: (channels: number, length: number, sampleRate: number) => {
    getChannelData: (channel: number) => Float32Array;
  };
  createBufferSource: () => {
    buffer: unknown;
    connect: (dest: unknown) => void;
    start: (when?: number) => void;
  };
  createGain: () => WebAudioGainNodeLike;
}

export class TacsimWebAudio {
  #muted: boolean;
  #ctx: WebAudioContextLike | null = null;
  #masterGain: WebAudioGainNodeLike | null = null;
  readonly #cache = new Map<string, unknown>();

  constructor() {
    this.#muted = storageGet(TACSIM_AUDIO_PREF_KEY) === "off";
  }

  get muted(): boolean {
    return this.#muted;
  }

  setMuted(muted: boolean): void {
    this.#muted = muted;
    if (this.#masterGain) {
      this.#masterGain.gain.value = muted ? 0 : 1;
    }
    storageSet(TACSIM_AUDIO_PREF_KEY, muted ? "off" : "on");
  }

  toggleMuted(): boolean {
    this.setMuted(!this.#muted);
    return this.#muted;
  }

  play(kind: ProceduralSfxKind, volume = 0.28): boolean {
    if (this.#muted) return false;
    const ctx = this.#ensureContext();
    if (!ctx) return false;

    try {
      if (ctx.state === "suspended") {
        // Attempt unlock without unhandled rejection, and do NOT queue clips while suspended
        // so multiple muted-by-autoplay triggers never burst simultaneously on next gesture.
        Promise.resolve(ctx.resume?.()).catch(() => {});
        return false;
      }
      if (ctx.state === "closed") return false;

      const sr = sanitizeSampleRate(ctx.sampleRate, 44_100);
      const cacheKey = `${kind}:${sr}`;
      let buf = this.#cache.get(cacheKey);
      if (!buf) {
        const pcm = computeProceduralWaveformSamples(kind, sr);
        const audioBuf = ctx.createBuffer(1, pcm.length, sr);
        audioBuf.getChannelData(0).set(pcm);
        buf = audioBuf;
        this.#cache.set(cacheKey, buf);
      }

      const safeVolume = Number.isFinite(volume) ? Math.max(0, Math.min(1, volume)) : 0.28;
      const gain = ctx.createGain();
      gain.gain.value = safeVolume;
      gain.connect(this.#masterGain ?? ctx.destination);

      const src = ctx.createBufferSource();
      src.buffer = buf;
      src.connect(gain);
      src.start(0);
      return true;
    } catch {
      return false;
    }
  }

  playUiChime(): boolean {
    return this.play("ui_chime", 0.22);
  }

  playMarketFillChime(): boolean {
    return this.play("market_fill", 0.30);
  }

  playAirlockCycle(): boolean {
    return this.play("airlock_cycle", 0.36);
  }

  playDockClamp(): boolean {
    return this.play("dock_clamp", 0.42);
  }

  playRadarPing(): boolean {
    return this.play("radar_ping", 0.28);
  }

  playLaserPulse(): boolean {
    return this.play("laser_pulse", 0.28);
  }

  playRailgunCrack(): boolean {
    return this.play("railgun_crack", 0.34);
  }

  playTorpedoLaunch(): boolean {
    return this.play("torpedo_launch", 0.34);
  }

  /** Trigger diegetic weapon/sensor SFX matching an outbound helm/tactical line. */
  playForCommand(cmd: string): void {
    const low = cmd.trim().toLowerCase();
    if (low.startsWith("+tac/beam")) {
      this.playLaserPulse();
    } else if (low.startsWith("+tac/railgun")) {
      this.playRailgunCrack();
    } else if (low.startsWith("+tac/torp")) {
      this.playTorpedoLaunch();
    } else if (low.startsWith("+tac/scan") || low === "scan") {
      this.playRadarPing();
    } else if (low.startsWith("+tac/pdc")) {
      this.playUiChime();
    } else if (low.startsWith("fire ")) {
      const target = low.slice(5).trim();
      if (
        target === "1" ||
        target.includes("laser") ||
        target.includes("beam") ||
        target.includes("pulse")
      ) {
        this.playLaserPulse();
      } else if (
        target.includes("missile") ||
        target.includes("torp") ||
        target.includes("rocket") ||
        target.includes("tube") ||
        target.includes("srm") ||
        target.includes("lrm")
      ) {
        this.playTorpedoLaunch();
      } else {
        this.playRailgunCrack();
      }
    }
  }

  #ensureContext(): WebAudioContextLike | null {
    if (this.#ctx) return this.#ctx;
    if (typeof globalThis === "undefined") return null;
    const g = globalThis as unknown as {
      AudioContext?: new () => WebAudioContextLike;
      webkitAudioContext?: new () => WebAudioContextLike;
    };
    const Ctor = g.AudioContext ?? g.webkitAudioContext;
    if (typeof Ctor !== "function") return null;
    try {
      const ctx = new Ctor();
      this.#ctx = ctx;
      const master = ctx.createGain();
      master.gain.value = this.#muted ? 0 : 1;
      master.connect(ctx.destination);
      this.#masterGain = master;
      return ctx;
    } catch {
      this.#ctx = null;
      this.#masterGain = null;
      return null;
    }
  }
}

export const tacsimAudio = new TacsimWebAudio();
