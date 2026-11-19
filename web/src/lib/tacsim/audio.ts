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

/**
 * Deterministic PCM waveform synthesis (`Float32Array` in `[-1, 1]`),
 * testable in headless Node without requiring a DOM `AudioContext`.
 */
export function computeProceduralWaveformSamples(
  kind: ProceduralSfxKind,
  sampleRate = 22_050,
): Float32Array {
  const sr = Math.max(8_000, Math.min(48_000, Math.round(sampleRate)));

  switch (kind) {
    case "ui_chime": {
      const n = Math.max(1, Math.floor(sr * 0.085));
      const out = new Float32Array(n);
      for (let i = 0; i < n; i++) {
        const t = i / sr;
        const env = Math.exp(-t * 42);
        const freq = i < n / 2 ? 880 : 1174.66;
        out[i] = Math.sin(t * freq * Math.PI * 2) * env * 0.42;
      }
      return out;
    }

    case "market_fill": {
      const n = Math.max(1, Math.floor(sr * 0.24));
      const out = new Float32Array(n);
      for (let i = 0; i < n; i++) {
        const t = i / sr;
        const second = t >= 0.09;
        const localT = second ? t - 0.09 : t;
        const env = Math.exp(-localT * 22);
        const f1 = second ? 783.99 : 587.33;
        const f2 = second ? 1174.66 : 880.0;
        out[i] =
          (Math.sin(t * f1 * Math.PI * 2) * 0.32 + Math.sin(t * f2 * Math.PI * 2) * 0.18) * env;
      }
      return out;
    }

    case "airlock_cycle": {
      const n = Math.max(1, Math.floor(sr * 0.95));
      const out = new Float32Array(n);
      let rng = 0x1234_5678;
      let lp = 0;
      for (let i = 0; i < n; i++) {
        const t = i / sr;
        const attack = Math.min(1, t / 0.08);
        const decay = Math.exp(-Math.max(0, t - 0.12) * 3.4);
        const clunk = Math.exp(-t * 28) * Math.sin(t * 92 * Math.PI * 2) * 0.58;
        rng = (Math.imul(rng, 1_664_525) + 1_013_904_223) >>> 0;
        const white = (rng & 0xffff) / 32_768 - 1;
        lp = lp * 0.78 + white * 0.22;
        out[i] = Math.max(-1, Math.min(1, clunk + lp * attack * decay * 0.48));
      }
      return out;
    }

    case "dock_clamp": {
      const n = Math.max(1, Math.floor(sr * 0.85));
      const out = new Float32Array(n);
      for (let i = 0; i < n; i++) {
        const t = i / sr;
        const thud = Math.exp(-t * 18) * Math.sin(t * 68 * Math.PI * 2) * 0.65;
        const chimeEnv = t > 0.1 ? Math.exp(-(t - 0.1) * 4.8) : 0;
        const chord =
          (Math.sin(t * 523.25 * Math.PI * 2) +
            Math.sin(t * 659.25 * Math.PI * 2) +
            Math.sin(t * 783.99 * Math.PI * 2)) *
          0.16 *
          chimeEnv;
        out[i] = Math.max(-1, Math.min(1, thud + chord));
      }
      return out;
    }

    case "radar_ping": {
      const n = Math.max(1, Math.floor(sr * 0.34));
      const out = new Float32Array(n);
      for (let i = 0; i < n; i++) {
        const t = i / sr;
        const env = Math.exp(-t * 11.5);
        const sweepHz = 1320 + 220 * Math.exp(-t * 18);
        out[i] = Math.sin(t * sweepHz * Math.PI * 2) * env * 0.36;
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
        const env = Math.exp(-t * 14);
        out[i] = Math.sin(phase) * env * 0.4;
      }
      return out;
    }

    case "railgun_crack": {
      const n = Math.max(1, Math.floor(sr * 0.28));
      const out = new Float32Array(n);
      let rng = 0x89ab_cdef;
      let bp = 0;
      for (let i = 0; i < n; i++) {
        const t = i / sr;
        rng = (Math.imul(rng, 1_664_525) + 1_013_904_223) >>> 0;
        const white = (rng & 0xffff) / 32_768 - 1;
        bp = bp * 0.62 + white * 0.38;
        const crack = bp * Math.exp(-t * 34) * 0.72;
        const sub = Math.sin(t * 96 * Math.PI * 2) * Math.exp(-t * 16) * 0.48;
        out[i] = Math.max(-1, Math.min(1, crack + sub));
      }
      return out;
    }

    case "torpedo_launch": {
      const n = Math.max(1, Math.floor(sr * 0.46));
      const out = new Float32Array(n);
      let rng = 0x5150_cafe;
      let lp = 0;
      for (let i = 0; i < n; i++) {
        const t = i / sr;
        rng = (Math.imul(rng, 1_664_525) + 1_013_904_223) >>> 0;
        const white = (rng & 0xffff) / 32_768 - 1;
        lp = lp * 0.85 + white * 0.15;
        const tubeThump = Math.sin(t * 74 * Math.PI * 2) * Math.exp(-t * 15) * 0.55;
        const booster = lp * Math.min(1, t / 0.06) * Math.exp(-Math.max(0, t - 0.08) * 5.2) * 0.45;
        out[i] = Math.max(-1, Math.min(1, tubeThump + booster));
      }
      return out;
    }
  }
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
  createGain: () => {
    gain: { value: number };
    connect: (dest: unknown) => void;
  };
}

export class TacsimWebAudio {
  #muted: boolean;
  #ctx: WebAudioContextLike | null = null;
  readonly #cache = new Map<ProceduralSfxKind, unknown>();

  constructor() {
    this.#muted = storageGet(TACSIM_AUDIO_PREF_KEY) === "off";
  }

  get muted(): boolean {
    return this.#muted;
  }

  setMuted(muted: boolean): void {
    this.#muted = muted;
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
        void ctx.resume?.();
      }
      let buf = this.#cache.get(kind);
      if (!buf) {
        const pcm = computeProceduralWaveformSamples(kind, ctx.sampleRate);
        const audioBuf = ctx.createBuffer(1, pcm.length, ctx.sampleRate);
        audioBuf.getChannelData(0).set(pcm);
        buf = audioBuf;
        this.#cache.set(kind, buf);
      }

      const gain = ctx.createGain();
      gain.gain.value = Math.max(0, Math.min(1, volume));
      gain.connect(ctx.destination);

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
    if (low.startsWith("+tac/beam") || low.startsWith("fire 1")) {
      this.playLaserPulse();
    } else if (low.startsWith("+tac/railgun")) {
      this.playRailgunCrack();
    } else if (low.startsWith("+tac/torp")) {
      this.playTorpedoLaunch();
    } else if (low.startsWith("+tac/scan") || low === "scan") {
      this.playRadarPing();
    } else if (low.startsWith("fire ")) {
      this.playRailgunCrack();
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
      this.#ctx = new Ctor();
      return this.#ctx;
    } catch {
      return null;
    }
  }
}

export const tacsimAudio = new TacsimWebAudio();
