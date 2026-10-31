<script lang="ts">
  // The cockpit's picture in the web client (doc 21 cut G.3c, doc 22 cut
  // P.10, doc 28 P2): the first-person or overhead view drawn locally by
  // `mush-pixel-wasm` from this connection's own `helm.cockpit` and
  // `arena.map` feeds — the flight HUD over it, the other machines in their
  // team's colours — rendered at the browser's refresh rate on `<canvas>`.
  //
  // It is drawn only while wanted AND the page is visible, and paused when
  // this view goes (the tab changes, the pilot leaves the seat). Whether the
  // viewer wants it is remembered in this browser alone.
  //
  // Keys, while the picture has focus (click it): the arrows turn the tank
  // (held) — `HEADING` orders, sent as the SSH TUI's view sends them, the one
  // place the cockpit tab sends an order rather than prefilling one — `,` and
  // `.` swing the look (again to stop), `c` centres it, space stops both.
  // Both are let go the moment their release could no longer be seen: the
  // frame losing focus, the window losing focus, the page going hidden (doc
  // 21a `X-02`: a tab switched away with an arrow down kept ordering
  // `HEADING` every 700 ms, and the look kept swinging).
  //
  // **The rest of the flight keys** (doc 21 §7.3, cut G.12c), the letters the
  // TUI's screen 0 reads: `a`/`d` turn as the arrows do; `w`/`s` step the
  // ordered speed (`SPEED`, a tenth of the frame's top speed a step, clamped
  // to its ceilings, coalesced to one line a quarter second) and the header
  // shows the ordered speed beside the frame's until they meet; `x` all stop;
  // `q`/`e` swing the look as `,`/`.` do; `f` flips the weapons gate (`FIRE
  // FREE` / `FIRE HOLD`, the header saying which); `g` a volley (`FIRE ALL`);
  // `t`/`T` walk the lock through the frame's contacts (`LOCK`); `v` the
  // overhead; `F1`–`F6` open the chat line with an `ORDER` ready — the one
  // family here that prefills rather than sends, through the screen's own
  // `prefill`. Every line goes through `app.helm`, as the turn's does.
  //
  // **The overhead** (doc 21 §7.4; doc 28 P2): the cockpit's second camera,
  // drawn client-side by the same wasm module (`KIND_OVERHEAD`) from the same
  // `arena.cockpit` and `arena.map` keyframes; `v` swaps between first person
  // and overhead.
  //
  // **Drawn here** (doc 21 cut G.3b; doc 28 P2): every web picture is drawn in
  // the browser by `mush-pixel-wasm` from the keyframes this connection
  // already holds — the gateway is never asked and draws nothing.
  import type { CockpitFrame } from "$lib/cockpit";
  import type { GameApp } from "$lib/game.svelte";
  import { isTextField } from "$lib/keys";
  import {
    DEFAULT_MAX_SPEED_KPH,
    DEFAULT_REVERSE_KPH,
    DEFAULT_TURN_RATE_MDEG_S,
    HELM_LEAD_DEG,
    HELM_REPEAT_MS,
    LOCK_PENDING_MS,
    LOOK_STEP_MS,
    MIN_TAP_TURN_MDEG,
    PICTURE_PREF_KEY,
    SPEED_COALESCE_MS,
    ZOOM_DEFAULT_PERMILLE,
    arcMdeg,
    canvasClickHeadingDeg,
    gateLine,
    gateWord,
    helmLine,
    nextLock,
    nextPictureKind,
    pickCanvasContact,
    pictureKey,
    predictedOffsetCm,
    speedLine,
    speedOutstanding,
    stepZoomPermille,
    steppedSpeed,
    swingLook,
    wrap360Mdeg,
  } from "$lib/picture";
  import {
    FRAME_H,
    FRAME_H_HD,
    FRAME_W,
    FRAME_W_HD,
    LOCAL_FRAME_MS,
    LocalPicture,
    localCapable,
    pixelModule,
  } from "$lib/pixel/local";
  import { storageGet, storageSet } from "$lib/storage";
  import { detachmentKeyPrefill } from "$lib/detachment";
  import { untrack } from "svelte";

  let {
    app,
    headingDeg,
    frame = null,
    prefill,
  }: {
    app: GameApp;
    headingDeg: number;
    /** The cockpit frame the screen holds (doc 21 cut G.12c): the speed, the
     * ceilings, the gate and the contacts the keys step from. */
    frame?: CockpitFrame | null;
    /** The screen's prefill (`F1`–`F6`): the chat line opened with a line
     * for the pilot to finish. */
    prefill?: (line: string) => void;
  } = $props();

  let wanted = $state(storageGet(PICTURE_PREF_KEY) === "on");
  let visible = $state(typeof document === "undefined" || document.visibilityState === "visible");
  let canvas: HTMLCanvasElement | undefined = $state();
  /** The look's swing: `-1` left, `0` still, `1` right. */
  let lookTurn = $state<-1 | 0 | 1>(0);
  /** The held turn: `-1` left, `0` none, `1` right. */
  let turning = $state<-1 | 0 | 1>(0);
  /** The 2.5D camera zoom in permille (`500..=2000`, `1000` = 100%). */
  let zoomPermille = $state(ZOOM_DEFAULT_PERMILLE);

  const PICTURE_WIDTH_KEY = "rustmush.pictureWidth";
  const PICTURE_HD_KEY = "rustmush.pictureHd";
  const COCKPIT_GLASS_KEY = "rustmush.cockpit_glass";
  const DEFAULT_PICTURE_W = 780;
  const MIN_PICTURE_W = 280;
  const MAX_PICTURE_W = 1280;

  let hdMode = $state(storageGet(PICTURE_HD_KEY) === "on");

  function toggleHd(): void {
    hdMode = !hdMode;
    storageSet(PICTURE_HD_KEY, hdMode ? "on" : "off");
    local?.forget();
  }

  let glassOn = $state(storageGet(COCKPIT_GLASS_KEY) !== "off");

  function toggleGlass(): void {
    glassOn = !glassOn;
    storageSet(COCKPIT_GLASS_KEY, glassOn ? "on" : "off");
  }

  /** Canopy glass damage level (`0` intact, `1` starred, `2` spiderwebbed) derived from hull armour. */
  const glassDamageLevel = $derived.by<0 | 1 | 2>(() => {
    if (!frame || frame.sections.length === 0) return 0;
    if (frame.wreck) return 2;
    let minArmourRatio = 1;
    let minCoreInternalsRatio = 1;
    let hasDamaged = false;
    let hasDestroyed = false;
    for (const sec of frame.sections) {
      if (sec.condition === "destroyed") hasDestroyed = true;
      else if (sec.condition !== "intact") hasDamaged = true;
      if (
        sec.internals_max > 0 &&
        (sec.id === "hull" || sec.id === "turret" || sec.id === "head" || sec.id === "torso")
      ) {
        minCoreInternalsRatio = Math.min(minCoreInternalsRatio, sec.internals / sec.internals_max);
      }
      for (const f of sec.facets) {
        if (f.armor_max > 0) minArmourRatio = Math.min(minArmourRatio, f.armor / f.armor_max);
      }
    }
    if (hasDestroyed || minCoreInternalsRatio < 0.6) return 2;
    if (hasDamaged || minArmourRatio < 0.65) return 1;
    return 0;
  });

  /** Subtle horizontal parallax (in SVG viewBox units) when the pilot swings their look left or right. */
  const glassLookShift = $derived(
    Math.max(-18, Math.min(18, Math.round((-app.picture.lookMdeg / 90_000) * 18))),
  );

  function loadPictureWidth(): number {
    const raw = Number(storageGet(PICTURE_WIDTH_KEY));
    if (Number.isFinite(raw) && raw >= MIN_PICTURE_W && raw <= MAX_PICTURE_W) {
      return Math.round(raw);
    }
    return DEFAULT_PICTURE_W;
  }

  let frameWidth = $state(loadPictureWidth());
  let dragging = $state(false);
  let dragStartX = 0;
  let dragStartW = DEFAULT_PICTURE_W;

  function onResizeDown(ev: PointerEvent): void {
    ev.preventDefault();
    ev.stopPropagation();
    dragging = true;
    dragStartX = ev.clientX;
    dragStartW = frameWidth;
    (ev.currentTarget as HTMLElement | null)?.setPointerCapture?.(ev.pointerId);
  }

  function onResizeMove(ev: PointerEvent): void {
    if (!dragging) return;
    const next = Math.round(
      Math.min(MAX_PICTURE_W, Math.max(MIN_PICTURE_W, dragStartW + (ev.clientX - dragStartX))),
    );
    frameWidth = next;
  }

  function onResizeUp(ev: PointerEvent): void {
    if (!dragging) return;
    dragging = false;
    (ev.currentTarget as HTMLElement | null)?.releasePointerCapture?.(ev.pointerId);
    storageSet(PICTURE_WIDTH_KEY, String(frameWidth));
  }

  /** Why the picture could not be drawn here, once it could not. */
  let localFailed = $state<string | null>(null);
  let local = $state.raw<LocalPicture | null>(null);
  /** The canvas the picture was last painted on. A fresh one — the picture
   * hidden and shown again makes a new element — has nothing on it whatever
   * the renderer last drew, so the first frame goes onto it whether or not
   * the picture changed (doc 21a `X-04`). */
  let painted = $state.raw<HTMLCanvasElement | null>(null);
  /** A frame has been drawn here, on the canvas that is up. */
  const drawnHere = $derived(painted !== null && painted === canvas);
  /** Which camera is chosen (`cockpit`, `overhead`, or `iso25d`). */
  const viewKind = $derived(app.picture.kind);
  /** Effective zoom in permille (`1000..=2000` in first-person, `500..=2000` in overhead/2.5D). */
  const effectiveZoomPermille = $derived(
    viewKind === "cockpit" ? Math.max(ZOOM_DEFAULT_PERMILLE, zoomPermille) : zoomPermille,
  );
  /** Whether the picture is up: wanted, visible, and not failed. */
  const active = $derived(wanted && visible && localFailed === null);

  /** The look, in millidegrees: the store's alone. */
  function lookTo(mdeg: number): void {
    app.picture.lookMdeg = mdeg;
  }

  function fail(err: unknown): void {
    localFailed = err instanceof Error ? err.message : String(err);
    local = null;
  }

  // The latest heading, for the helm's repeats and client-side hull prediction
  // (matching `mush-net-ssh/src/tui/helm.rs`).
  let heading = 0;
  let frameHeadingMdeg = 0;
  let baseHeadingMdeg = 0;
  let baseAtMs = 0;
  let orderedHeadingMdeg: number | null = null;
  let orderedAtMs = 0;
  let turnStartAtMs = 0;
  let turnTapBaseMdeg = 0;
  let turnRateMdegPerS = DEFAULT_TURN_RATE_MDEG_S;
  let prevFrameTMs: number | null = null;
  let prevFrameHeadingMdeg: number | null = null;
  let frameArrivedAtMs = 0;
  let livePulseStream = false;

  function hullStuck(): boolean {
    if (frame === null) return false;
    return (frame.power !== "" && frame.power !== "online") || frame.wreck || frame.immobile;
  }

  function predictedHeadingMdeg(now: number): number {
    if (frame === null) return wrap360Mdeg(heading * 1_000);
    if (hullStuck()) return frameHeadingMdeg;
    if (orderedHeadingMdeg === null) return baseHeadingMdeg;
    const elapsed = Math.max(0, Math.min(now - baseAtMs, 2_500));
    const step = (turnRateMdegPerS * elapsed) / 1_000;
    const d = arcMdeg(baseHeadingMdeg, orderedHeadingMdeg);
    return wrap360Mdeg(baseHeadingMdeg + Math.sign(d) * Math.min(Math.abs(d), step));
  }

  $effect(() => {
    heading = headingDeg;
    const rawCockpit = app.picture.cockpitData as { heading_mdeg?: unknown } | null;
    const rawMdeg = wrap360Mdeg(
      typeof rawCockpit?.heading_mdeg === "number" ? rawCockpit.heading_mdeg : headingDeg * 1_000,
    );
    if (frame === null) {
      frameHeadingMdeg = rawMdeg;
      baseHeadingMdeg = rawMdeg;
      orderedHeadingMdeg = null;
      prevFrameTMs = null;
      prevFrameHeadingMdeg = null;
      frameArrivedAtMs = 0;
      livePulseStream = false;
      return;
    }
    const f = frame;
    untrack(() => {
      if (f.t_ms === prevFrameTMs && rawMdeg === prevFrameHeadingMdeg) return;
      const now = Date.now();
      const currentPred = predictedHeadingMdeg(now);
      if (prevFrameTMs !== null && f.t_ms !== null) {
        const pulseDt = f.t_ms - prevFrameTMs;
        livePulseStream = pulseDt > 0 && pulseDt <= 10_000;
      }
      frameArrivedAtMs = now;
      if (
        prevFrameTMs !== null &&
        f.t_ms !== null &&
        f.t_ms > prevFrameTMs &&
        prevFrameHeadingMdeg !== null &&
        orderedHeadingMdeg !== null
      ) {
        const dt = f.t_ms - prevFrameTMs;
        const turned = Math.abs(arcMdeg(prevFrameHeadingMdeg, rawMdeg));
        if (dt > 0 && turned > 0 && Math.abs(arcMdeg(rawMdeg, orderedHeadingMdeg)) > 1_000) {
          turnRateMdegPerS = Math.min(180_000, Math.max(5_000, Math.round((turned * 1_000) / dt)));
        }
      }
      frameHeadingMdeg = rawMdeg;
      prevFrameTMs = f.t_ms;
      prevFrameHeadingMdeg = rawMdeg;
      if (hullStuck()) {
        baseHeadingMdeg = rawMdeg;
        baseAtMs = now;
        orderedHeadingMdeg = null;
        return;
      }
      if (orderedHeadingMdeg !== null) {
        if (turning === 0 && Math.abs(arcMdeg(rawMdeg, orderedHeadingMdeg)) <= 1_000) {
          baseHeadingMdeg = rawMdeg;
          baseAtMs = now;
          orderedHeadingMdeg = null;
          return;
        }
        if (turning === 0 && now - orderedAtMs > 2_500) {
          baseHeadingMdeg = rawMdeg;
          baseAtMs = now;
          orderedHeadingMdeg = null;
          return;
        }
        const toOrderFromServer = arcMdeg(rawMdeg, orderedHeadingMdeg);
        const toOrderFromPred = arcMdeg(currentPred, orderedHeadingMdeg);
        const serverLagMdeg = arcMdeg(rawMdeg, currentPred);
        if (
          Math.sign(toOrderFromServer) === Math.sign(serverLagMdeg) &&
          Math.abs(serverLagMdeg) <= 25_000 &&
          Math.abs(toOrderFromPred) <= Math.abs(toOrderFromServer)
        ) {
          baseHeadingMdeg = currentPred;
          baseAtMs = now;
          return;
        }
      }
      baseHeadingMdeg = rawMdeg;
      baseAtMs = now;
    });
  });

  function toggle(): void {
    wanted = !wanted;
    storageSet(PICTURE_PREF_KEY, wanted ? "on" : "off");
  }

  // Keep the store's `on` and `lookMdeg` aligned with whether the picture is
  // active, and reset them when the picture hides or the view is torn down.
  $effect(() => {
    const on = active;
    untrack(() => {
      app.picture.on = on;
      if (!on) app.picture.lookMdeg = 0;
    });
    return () =>
      untrack(() => {
        app.picture.on = false;
        app.picture.lookMdeg = 0;
      });
  });

  // The page's visibility, and the two ways a key's release can go unseen:
  // the page hidden (the keys' events stop: the tab switched, the window
  // minimised) and the window blurred (another window took the keys).
  // And the view torn down (the screen changed, the seat went) with a key
  // still held: let go as a blur would — not every browser blurs an element
  // it removes.
  // Also listen in the capture phase on `window` so flight keys work anywhere
  // on the cockpit screen without requiring the user to click inside `.frame`
  // first, matching the TUI's screen 0 behaviour.
  $effect(() => () => release());
  $effect(() => {
    const onVisibility = (): void => {
      visible = document.visibilityState === "visible";
      if (!visible) release();
    };
    const onFocusIn = (ev: FocusEvent): void => {
      if (isTextField(ev.target)) release();
    };
    const onHelp = (ev: Event): void => {
      ev.preventDefault();
    };
    document.addEventListener("visibilitychange", onVisibility);
    window.addEventListener("blur", release);
    window.addEventListener("keydown", onWindowKeyDown, true);
    window.addEventListener("keyup", onWindowKeyUp, true);
    window.addEventListener("focusin", onFocusIn, true);
    window.addEventListener("help", onHelp, true);
    return () => {
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener("blur", release);
      window.removeEventListener("keydown", onWindowKeyDown, true);
      window.removeEventListener("keyup", onWindowKeyUp, true);
      window.removeEventListener("focusin", onFocusIn, true);
      window.removeEventListener("help", onHelp, true);
    };
  });

  // Drawn here: the module loaded once, while wanted.
  $effect(() => {
    if (!active || local !== null || !localCapable()) return;
    let live = true;
    pixelModule().then(
      (m) => {
        if (live) local = new LocalPicture(m);
      },
      (err: unknown) => {
        if (live) fail(err);
      },
    );
    return () => {
      live = false;
    };
  });

  // Drawn here: a frame at most every `LOCAL_FRAME_MS` while wanted and
  // visible, put on the canvas only when it changed — or when the canvas is
  // one nothing has been painted on yet.
  $effect(() => {
    const l = local;
    const c = canvas;
    if (!l || !c || !active) return;
    if (untrack(() => painted) !== c) l.forget();
    let raf = 0;
    let last = -Infinity;
    const tick = (t: number): void => {
      raf = requestAnimationFrame(tick);
      if (t - last < LOCAL_FRAME_MS) return;
      last = t;
      try {
        const now = Date.now();
        const predMdeg = predictedHeadingMdeg(now);
        const hullOffsetMdeg = frame !== null ? arcMdeg(frameHeadingMdeg, predMdeg) : 0;
        const orderedMdeg =
          frame !== null &&
          orderedHeadingMdeg !== null &&
          Math.abs(arcMdeg(predMdeg, orderedHeadingMdeg)) > 1_000
            ? orderedHeadingMdeg
            : null;
        const rawCockpit = app.picture.cockpitData as {
          speed_cms?: unknown;
          blocked?: unknown;
        } | null;
        const speedCms =
          typeof rawCockpit?.speed_cms === "number"
            ? rawCockpit.speed_cms
            : Math.round(((frame?.speed_kph ?? 0) * 1_000) / 36);
        const canDeadReckon =
          frame !== null &&
          !hullStuck() &&
          rawCockpit?.blocked !== true &&
          speedCms !== 0 &&
          frameArrivedAtMs > 0 &&
          (livePulseStream || orderedHeadingMdeg !== null || turning !== 0 || orderedKph !== null);
        const { dxCm, dyCm } = canDeadReckon
          ? predictedOffsetCm(speedCms, predMdeg, now - frameArrivedAtMs)
          : { dxCm: 0, dyCm: 0 };
        const targetW = hdMode ? FRAME_W_HD : FRAME_W;
        const targetH = hdMode ? FRAME_H_HD : FRAME_H;
        if (
          l.draw(
            app.picture.cockpitData,
            app.picture.mapData,
            app.picture.lookMdeg,
            app.picture.kind,
            hullOffsetMdeg,
            orderedMdeg,
            dxCm,
            dyCm,
            effectiveZoomPermille,
            targetW,
            targetH,
          )
        ) {
          if (c.width !== targetW || c.height !== targetH) {
            c.width = targetW;
            c.height = targetH;
          }
          c.getContext("2d")?.putImageData(l.image(), 0, 0);
          painted = c;
        }
      } catch (err) {
        cancelAnimationFrame(raf);
        fail(err);
      }
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  });

  // The look swings while `lookTurn` is set, sent again each step.
  $effect(() => {
    const dir = lookTurn;
    if (dir === 0) return;
    const timer = setInterval(() => {
      lookTo(swingLook(app.picture.lookMdeg, dir, LOOK_STEP_MS));
    }, LOOK_STEP_MS);
    return () => clearInterval(timer);
  });

  function startTurn(dir: -1 | 1): void {
    const now = Date.now();
    if (frame !== null && !hullStuck()) {
      const p = predictedHeadingMdeg(now);
      turnStartAtMs = now;
      turnTapBaseMdeg =
        orderedHeadingMdeg !== null && Math.sign(arcMdeg(p, orderedHeadingMdeg)) === dir
          ? orderedHeadingMdeg
          : p;
    }
    turning = dir;
  }

  // A held turn's order is extended while the key is down.
  $effect(() => {
    const dir = turning;
    if (dir === 0) return;
    const sendTurnOrder = (): void => {
      const now = Date.now();
      if (frame !== null && !hullStuck()) {
        const p = predictedHeadingMdeg(now);
        baseHeadingMdeg = p;
        baseAtMs = now;
        const deg = (((Math.round(p / 1_000 + dir * HELM_LEAD_DEG) % 360) + 360) % 360);
        orderedHeadingMdeg = deg * 1_000;
        orderedAtMs = now;
        app.helm(helmLine(p / 1_000, dir));
      } else {
        app.helm(helmLine(heading, dir));
      }
    };
    untrack(sendTurnOrder);
    const timer = setInterval(sendTurnOrder, HELM_REPEAT_MS);
    return () => clearInterval(timer);
  });

  function stopTurn(): void {
    if (turning !== 0) {
      const dir = turning;
      turning = 0;
      if (frame !== null && !hullStuck()) {
        const now = Date.now();
        const p = predictedHeadingMdeg(now);
        const dt = now - turnStartAtMs;
        const targetMdeg =
          dt > 0 && dt < 170
            ? wrap360Mdeg(turnTapBaseMdeg + dir * MIN_TAP_TURN_MDEG)
            : p;
        baseHeadingMdeg = p;
        baseAtMs = now;
        const deg = (((Math.round(targetMdeg / 1_000) % 360) + 360) % 360);
        orderedHeadingMdeg = deg * 1_000;
        orderedAtMs = now;
        app.helm(helmLine(deg, 0));
      } else {
        app.helm(helmLine(heading, 0));
      }
    }
  }

  /** Let go of everything a key holds: the look's swing, and the held turn
   * with its stop order (where one was held). */
  function release(): void {
    lookTurn = 0;
    stopTurn();
  }

  // -- the speed (doc 21 cut G.12c) ------------------------------------------

  /** The speed the pilot last ordered, until the frame reaches it. */
  let orderedKph = $state<number | null>(null);
  /** The step waiting for the coalescing window to close, and its timer. */
  let pendingKph: number | null = null;
  let speedTimer: ReturnType<typeof setTimeout> | null = null;

  const frameKph = $derived(frame?.speed_kph ?? 0);
  const maxKph = $derived(frame?.max_speed_kph ?? DEFAULT_MAX_SPEED_KPH);
  // The ceiling astern as a positive number, whichever sign a server sends.
  const reverseKph = $derived(Math.abs(frame?.reverse_kph ?? DEFAULT_REVERSE_KPH));
  /** The ordered speed while the frame has not reached it — what the header
   * shows beside the frame's, and what the next step goes from. */
  const outstandingKph = $derived(speedOutstanding(orderedKph, frameKph) ? orderedKph : null);

  // Reached: the order is forgotten, so a later drift of the frame's speed is
  // not shown as an order the pilot gave.
  $effect(() => {
    if (orderedKph !== null && !speedOutstanding(orderedKph, frameKph)) orderedKph = null;
  });

  /** Send the step waiting in the window, if one is. */
  function flushSpeed(): void {
    if (speedTimer !== null) clearTimeout(speedTimer);
    speedTimer = null;
    if (pendingKph !== null) {
      app.helm(speedLine(pendingKph));
      pendingKph = null;
    }
  }

  /** One step `dir` from the outstanding order, else from the frame's speed;
   * sent when the window closes, the last value winning. */
  function stepSpeed(dir: -1 | 1): void {
    const from = speedOutstanding(orderedKph, frameKph) ? orderedKph : frameKph;
    const next = steppedSpeed(from, dir, maxKph, reverseKph);
    orderedKph = next;
    pendingKph = next;
    speedTimer ??= setTimeout(flushSpeed, SPEED_COALESCE_MS);
  }

  /** All stop: `SPEED 0` at once (a step still waiting is dropped), and the
   * helm's stop whether or not a key holds a turn — all stop cancels a typed
   * `HEADING` as well as a held arrow. */
  function allStop(): void {
    if (speedTimer !== null) clearTimeout(speedTimer);
    speedTimer = null;
    pendingKph = null;
    orderedKph = 0;
    app.helm(speedLine(0));
    turning = 0;
    if (frame !== null && !hullStuck()) {
      const now = Date.now();
      const p = predictedHeadingMdeg(now);
      baseHeadingMdeg = p;
      baseAtMs = now;
      const deg = (((Math.round(p / 1_000) % 360) + 360) % 360);
      orderedHeadingMdeg = deg * 1_000;
      orderedAtMs = now;
      app.helm(helmLine(deg, 0));
    } else {
      app.helm(helmLine(heading, 0));
    }
  }

  /** A step still waiting when the view goes (the screen changed) is sent, not
   * lost: the speed stands across screens, only the turn and the look stop. */
  $effect(() => () => flushSpeed());

  // -- weapons free auto-fire rule controls ----------------------------------

  let fireRuleWeapon = $state("ALL");
  let fireRuleOdds = $state(60);
  let fireRuleArmed = $state(false);

  $effect(() => {
    const f = frame;
    if (!f) {
      fireRuleArmed = false;
      return;
    }
    if (fireRuleWeapon !== "ALL" && !f.weapons.some((w) => w.id === fireRuleWeapon)) {
      fireRuleWeapon = "ALL";
    }
  });

  function sendFireRule(): void {
    fireRuleArmed = true;
    if (frame?.weapons_free === false) {
      app.helm("FIRE FREE");
    }
    app.helm(`FIRE ${fireRuleWeapon} WHEN odds >= ${fireRuleOdds}`);
  }

  function onFireWeaponChange(ev: Event): void {
    fireRuleWeapon = (ev.currentTarget as HTMLSelectElement).value || "ALL";
    sendFireRule();
  }

  function onFireOddsChange(ev: Event): void {
    const raw = Number((ev.currentTarget as HTMLInputElement).value);
    if (Number.isFinite(raw)) {
      fireRuleOdds = Math.max(10, Math.min(100, Math.round(raw)));
    }
    sendFireRule();
  }

  function toggleFireRule(): void {
    if (fireRuleArmed) {
      fireRuleArmed = false;
      app.helm(`FIRE ${fireRuleWeapon} OFF`);
    } else {
      sendFireRule();
    }
  }

  // -- HUD speed bar pointer drag --------------------------------------------

  let hudSpeedDragging = $state(false);

  function hudSpeedFromNormY(normY: number): number {
    // In `overlay.rs`, `draw_speed` places the bar at y = 0.30..0.70 of canvas height.
    // Above 0.70 (0.30..0.70) maps 0..maxKph; below 0.70 (0.70..0.78) maps 0..-reverseKph.
    if (normY <= 0.7) {
      const frac = Math.max(0, Math.min(1, (0.7 - normY) / 0.4));
      return Math.round(frac * maxKph);
    }
    const revFrac = Math.max(0, Math.min(1, (normY - 0.7) / 0.08));
    return -Math.round(revFrac * reverseKph);
  }

  function applyHudSpeed(normY: number): void {
    const next = hudSpeedFromNormY(normY);
    orderedKph = next;
    pendingKph = next;
    speedTimer ??= setTimeout(flushSpeed, SPEED_COALESCE_MS);
  }

  function onCanvasPointerDown(ev: PointerEvent): void {
    if (viewKind !== "cockpit" || !active || frame === null) return;
    const coords = canvasNormCoords(ev);
    if (!coords) return;
    if (coords.normX >= 0.88 && coords.normY >= 0.24 && coords.normY <= 0.78) {
      ev.preventDefault();
      hudSpeedDragging = true;
      (ev.currentTarget as HTMLElement | null)?.setPointerCapture?.(ev.pointerId);
      applyHudSpeed(coords.normY);
    }
  }

  function onCanvasPointerMove(ev: PointerEvent): void {
    if (!hudSpeedDragging) return;
    const coords = canvasNormCoords(ev);
    if (!coords) return;
    applyHudSpeed(coords.normY);
  }

  function onCanvasPointerUp(ev: PointerEvent): void {
    if (!hudSpeedDragging) return;
    hudSpeedDragging = false;
    (ev.currentTarget as HTMLElement | null)?.releasePointerCapture?.(ev.pointerId);
    flushSpeed();
  }

  // -- the lock cycle (doc 21 cut G.12c) -------------------------------------

  /** The lock last asked for, until the frame shows it or it goes stale, so
   * `t` pressed again before the next frame walks on rather than asking for
   * the same contact twice. */
  let lockAsked: { designator: string; at: number } | null = null;

  function cycleLock(dir: -1 | 1): void {
    if (frame === null) return;
    const now = Date.now();
    const current =
      lockAsked !== null && now - lockAsked.at < LOCK_PENDING_MS
        ? lockAsked.designator
        : frame.lock_designator;
    const designator = nextLock(frame.contacts, current, dir);
    if (designator === null) return;
    lockAsked = { designator, at: now };
    app.helm(`LOCK ${designator}`);
  }

  function onWindowKeyDown(ev: KeyboardEvent): void {
    if (ev.defaultPrevented || ev.isComposing) return;
    if (typeof document !== "undefined" && document.querySelector('[role="dialog"]') !== null) return;
    if (ev.ctrlKey || ev.altKey || ev.metaKey) return;
    const key = pictureKey(ev.key);
    if (key === null) return;
    if (key.kind !== "order" && isTextField(ev.target)) return;
    onKeyDown(ev);
  }

  function onWindowKeyUp(ev: KeyboardEvent): void {
    if (ev.defaultPrevented) return;
    onKeyUp(ev);
  }

  function onKeyDown(ev: KeyboardEvent): void {
    if (ev.defaultPrevented) return;
    // A chord is the browser's (Ctrl-W, Ctrl-T, Alt-F …), never the picture's.
    if (ev.ctrlKey || ev.altKey || ev.metaKey) return;
    const key = pictureKey(ev.key);
    if (key === null) return;
    if (key.kind === "order") {
      // Always taken on the cockpit screen, so the browser's F1 help,
      // F5 reload and F6 address bar never fire from the cockpit.
      ev.preventDefault();
      if (!ev.repeat) {
        const line = detachmentKeyPrefill(frame?.detachment, key.slot === "ALL" ? 1 : key.slot);
        if (line !== null) prefill?.(line);
      }
      return;
    }
    if (!active) return;
    ev.preventDefault();
    switch (key.kind) {
      case "turn":
        if (!ev.repeat) startTurn(key.dir);
        break;
      case "look":
        if (!ev.repeat) lookTurn = lookTurn === key.dir ? 0 : key.dir;
        break;
      case "centre":
        lookTurn = 0;
        lookTo(0);
        break;
      case "steady":
        lookTurn = 0;
        stopTurn();
        break;
      case "speed":
        // A held key's repeats step on, coalesced by the window.
        stepSpeed(key.dir);
        break;
      case "allStop":
        if (!ev.repeat) allStop();
        break;
      case "view":
        if (!ev.repeat) app.picture.kind = nextPictureKind(app.picture.kind);
        break;
      case "gate":
        if (!ev.repeat) app.helm(gateLine(frame?.weapons_free ?? null));
        break;
      case "volley":
        if (!ev.repeat) app.helm("FIRE ALL");
        break;
      case "fireSlot":
        if (!ev.repeat) {
          const w = frame?.weapons[key.slot];
          if (w?.id) app.helm(`FIRE ${w.id}`);
        }
        break;
      case "lock":
        if (!ev.repeat) cycleLock(key.dir);
        break;
      case "zoom":
        zoomPermille = stepZoomPermille(zoomPermille, key.dir, viewKind);
        break;
    }
  }

  function canvasNormCoords(ev: MouseEvent): { normX: number; normY: number } | null {
    const el = canvas;
    if (!el) return null;
    const rect = el.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) return null;
    return {
      normX: (ev.clientX - rect.left) / rect.width,
      normY: (ev.clientY - rect.top) / rect.height,
    };
  }

  function onCanvasWheel(ev: WheelEvent): void {
    if (!active) return;
    ev.preventDefault();
    const dir = ev.deltaY < 0 ? 1 : -1;
    zoomPermille = stepZoomPermille(zoomPermille, dir, viewKind);
  }

  function onCanvasClick(ev: MouseEvent): void {
    if (!active || frame === null) return;
    const coords = canvasNormCoords(ev);
    if (!coords) return;
    if (viewKind === "cockpit") {
      // HUD heading tape (`draw_tape` in `overlay.rs`): x = 0.20..0.80, y = 0.01..0.15, ±30° reach.
      if (
        coords.normX >= 0.2 &&
        coords.normX <= 0.8 &&
        coords.normY >= 0.01 &&
        coords.normY <= 0.15
      ) {
        const now = Date.now();
        const camMdeg = wrap360Mdeg(predictedHeadingMdeg(now) + app.picture.lookMdeg);
        const rel = Math.max(-1, Math.min(1, (coords.normX - 0.5) / 0.3));
        const targetMdeg = wrap360Mdeg(camMdeg + Math.round(rel * 30_000));
        const deg = (((Math.round(targetMdeg / 1_000) % 360) + 360) % 360);
        if (!hullStuck()) {
          baseHeadingMdeg = predictedHeadingMdeg(now);
          baseAtMs = now;
          orderedHeadingMdeg = deg * 1_000;
          orderedAtMs = now;
        }
        app.helm(helmLine(deg, 0));
        return;
      }
      if (coords.normX >= 0.88 && coords.normY >= 0.24 && coords.normY <= 0.78) {
        return;
      }
    }
    const rawCockpit = app.picture.cockpitData as { sight_m?: unknown } | null;
    const sightM = typeof rawCockpit?.sight_m === "number" ? rawCockpit.sight_m : 600;
    const camDeg = (predictedHeadingMdeg(Date.now()) + app.picture.lookMdeg) / 1_000;
    const desig = pickCanvasContact(
      frame.contacts,
      viewKind,
      coords.normX,
      coords.normY,
      effectiveZoomPermille,
      sightM,
      camDeg,
    );
    if (desig !== null) {
      lockAsked = { designator: desig, at: Date.now() };
      app.helm(`LOCK ${desig}`);
    }
  }

  function onCanvasDblClick(ev: MouseEvent): void {
    if ((viewKind !== "iso25d" && viewKind !== "overhead") || !active) return;
    const coords = canvasNormCoords(ev);
    if (!coords) return;
    if (frame !== null) {
      const rawCockpit = app.picture.cockpitData as { sight_m?: unknown } | null;
      const sightM = typeof rawCockpit?.sight_m === "number" ? rawCockpit.sight_m : 600;
      if (
        pickCanvasContact(
          frame.contacts,
          viewKind,
          coords.normX,
          coords.normY,
          zoomPermille,
          sightM,
        ) !== null
      ) {
        return;
      }
    }
    const deg = canvasClickHeadingDeg(viewKind, coords.normX, coords.normY);
    if (deg === null) return;
    const now = Date.now();
    if (frame !== null && !hullStuck()) {
      baseHeadingMdeg = predictedHeadingMdeg(now);
      baseAtMs = now;
      orderedHeadingMdeg = deg * 1_000;
      orderedAtMs = now;
    }
    app.helm(helmLine(deg, 0));
  }

  function onKeyUp(ev: KeyboardEvent): void {
    if (ev.defaultPrevented) return;
    const key = pictureKey(ev.key);
    if (key?.kind === "turn" && key.dir === turning) {
      ev.preventDefault();
      stopTurn();
    }
  }
</script>

<section
  class="picturebox"
  aria-label="picture"
  style={wanted ? `width: min(100%, ${frameWidth}px)` : undefined}
>
  <header>
    <strong>picture</strong>
    <span class="grow"></span>
    {#if wanted && active}
      {#if outstandingKph !== null}
        <span class="readout" aria-label="speed ordered">spd {frameKph}→{outstandingKph}</span>
      {/if}
      {#if frame !== null}
        <span class="wf-controls" aria-label="picture weapons free controls">
          <button
            type="button"
            class="toggle wf-btn"
            class:armed={fireRuleArmed}
            aria-pressed={fireRuleArmed}
            title={fireRuleArmed
              ? `FIRE ${fireRuleWeapon} OFF`
              : `FIRE ${fireRuleWeapon} WHEN odds >= ${fireRuleOdds}`}
            onclick={toggleFireRule}
          >{gateWord(frame.weapons_free)}</button>
          {#if frame.weapons.length > 0}
            <select
              class="wf-select"
              aria-label="picture auto-fire weapon"
              value={fireRuleWeapon}
              onchange={onFireWeaponChange}
            >
              <option value="ALL">ALL</option>
              {#each frame.weapons as w (w.id)}
                <option value={w.id}>{w.id}</option>
              {/each}
            </select>
            <input
              type="range"
              class="wf-slider"
              aria-label="picture auto-fire odds"
              min="10"
              max="100"
              step="5"
              value={fireRuleOdds}
              oninput={onFireOddsChange}
              onchange={onFireOddsChange}
            />
            <span class="readout wf-odds">&gt;={fireRuleOdds}%</span>
          {/if}
        </span>
        {#if frame.weapons.length > 0}
          <span class="wpn-slots" aria-label="weapon quick-fire slots">
            {#each frame.weapons.slice(0, 4) as w, idx (w.id)}
              <button
                type="button"
                class="toggle zoom-btn wpn-slot-btn"
                class:armed={w.ready}
                title={`Fire ${w.id} (Shift+${idx + 1})`}
                onclick={() => app.helm(`FIRE ${w.id}`)}>{idx + 1}:{w.id}</button
              >
            {/each}
          </span>
        {/if}
        {#if frame.mode === "range"}
          <!-- 16c Q-PT1: a range says so beside the gate, as the TUI's top line does. -->
          <span class="readout" aria-label="match mode">RANGE</span>
        {/if}
      {/if}
      <span class="zoom-controls" aria-label="camera zoom">
        <button
          type="button"
          class="toggle zoom-btn"
          title="Zoom out (-)"
          onclick={() => (zoomPermille = stepZoomPermille(zoomPermille, -1, viewKind))}>−</button
        >
        <button
          type="button"
          class="toggle zoom-btn"
          title="Reset zoom"
          onclick={() => (zoomPermille = stepZoomPermille(zoomPermille, 0, viewKind))}
          >{Math.round(effectiveZoomPermille / 10)}%</button
        >
        <button
          type="button"
          class="toggle zoom-btn"
          title="Zoom in (+)"
          onclick={() => (zoomPermille = stepZoomPermille(zoomPermille, 1, viewKind))}>+</button
        >
      </span>
      <button
        type="button"
        class="toggle wf-btn"
        class:armed={hdMode}
        aria-pressed={hdMode}
        aria-label="HD resolution toggle"
        title={hdMode ? "Switch to 640×480 SD" : "Switch to 1280×720 HD"}
        onclick={toggleHd}>{hdMode ? "1280p" : "640p"}</button
      >
      <span class="camera-pills" role="group" aria-label="camera view">
        <button
          type="button"
          class="toggle cam-btn"
          class:active={viewKind === "cockpit"}
          aria-pressed={viewKind === "cockpit"}
          title="First-person view (v)"
          onclick={() => (app.picture.kind = "cockpit")}>1st</button
        >
        <button
          type="button"
          class="toggle cam-btn"
          class:active={viewKind === "overhead"}
          aria-pressed={viewKind === "overhead"}
          title="Overhead view (v)"
          onclick={() => (app.picture.kind = "overhead")}>Overhead</button
        >
        <button
          type="button"
          class="toggle cam-btn"
          class:active={viewKind === "iso25d"}
          aria-pressed={viewKind === "iso25d"}
          title="2.5D tactical view (v)"
          onclick={() => (app.picture.kind = "iso25d")}>2.5D</button
        >
        {#if viewKind === "cockpit"}
          <button
            type="button"
            class="toggle cam-btn"
            class:active={glassOn}
            aria-label="cockpit glass"
            aria-pressed={glassOn}
            title={glassOn ? "Hide cockpit canopy glass" : "Show cockpit canopy glass"}
            onclick={toggleGlass}>Glass</button
          >
        {/if}
        {#if app.picture.lookMdeg !== 0}
          <button
            type="button"
            class="toggle cam-btn"
            title="Centre look (c)"
            onclick={() => {
              lookTurn = 0;
              lookTo(0);
            }}>Centre</button
          >
        {/if}
      </span>
      <span class="dim legend"
        >a d turn · w s speed · q e look · c centre · x stop · v view · f free/hold · g volley · t lock ·
        F1–F6 order</span
      >
    {/if}
    <button type="button" class="toggle" onclick={toggle}>
      {wanted ? "Hide the picture" : "Show the picture"}
    </button>
  </header>
  {#if wanted}
    <!-- An application region: it takes keys while focused. -->
    <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
    <div
      class="frame"
      tabindex="0"
      role="application"
      aria-label={viewKind === "overhead"
        ? "overhead view; the cockpit's flight keys work here, v switches to 2.5D tactical"
        : viewKind === "iso25d"
          ? "2.5D tactical view; the cockpit's flight keys work here, v returns to first person"
          : "first-person view; a and d or the arrows turn, w and s set the speed, q and e swing the look, v switches view"}
      onkeydown={onKeyDown}
      onkeyup={onKeyUp}
    >
      <canvas
        bind:this={canvas}
        aria-hidden="true"
        style={hdMode ? "aspect-ratio: 16 / 9" : undefined}
        onwheel={onCanvasWheel}
        onclick={onCanvasClick}
        ondblclick={onCanvasDblClick}
        onpointerdown={onCanvasPointerDown}
        onpointermove={onCanvasPointerMove}
        onpointerup={onCanvasPointerUp}
        onpointercancel={onCanvasPointerUp}
      ></canvas>
      {#if viewKind === "cockpit" && glassOn}
        <div
          class="cockpit-glass-overlay"
          class:power-off={frame?.power === "off"}
          data-damage={glassDamageLevel}
          aria-label="cockpit glass overlay"
          style="--look-shift: {glassLookShift}px"
        >
          <div class="glass-pane-optics"></div>
          <svg
            class="cockpit-glass-svg"
            viewBox="0 0 640 480"
            preserveAspectRatio="none"
            aria-hidden="true"
          >
            <defs>
              <linearGradient id="cg-strut-grad" x1="0" y1="0" x2="1" y2="1">
                <stop offset="0%" stop-color="#1b222c" />
                <stop offset="50%" stop-color="#11161d" />
                <stop offset="100%" stop-color="#090c10" />
              </linearGradient>
              <linearGradient id="cg-sill-grad" x1="0" y1="0" x2="0" y2="1">
                <stop offset="0%" stop-color="#1e2631" />
                <stop offset="35%" stop-color="#12171f" />
                <stop offset="100%" stop-color="#080b0f" />
              </linearGradient>
              <linearGradient id="cg-sheen-grad" x1="0" y1="0" x2="1" y2="1">
                <stop offset="0%" stop-color="#b8e4ff" stop-opacity="0.07" />
                <stop offset="35%" stop-color="#b8e4ff" stop-opacity="0.015" />
                <stop offset="65%" stop-color="#90d0ff" stop-opacity="0" />
                <stop offset="100%" stop-color="#b8e4ff" stop-opacity="0.045" />
              </linearGradient>
            </defs>

            <!-- Specular glass pane reflections & wiper sweep boundary -->
            <g transform="translate({glassLookShift * 0.4}, 0)">
              <polygon
                points="145,10 310,10 190,448 55,448"
                fill="url(#cg-sheen-grad)"
              />
              <polygon
                points="390,10 495,10 385,448 295,448"
                fill="url(#cg-sheen-grad)"
                opacity="0.65"
              />
              <!-- Subtle wiper arc & outer corner condensation/grime boundary -->
              <path
                d="M 128 450 A 340 340 0 0 1 512 450"
                fill="none"
                stroke="#9ad4ff"
                stroke-opacity="0.07"
                stroke-width="1"
                stroke-dasharray="6 4"
              />
              <!-- Corner micro-scratches / dust stipple outside the main sightline -->
              <g stroke="#c8e6ff" stroke-opacity="0.11" stroke-width="0.8">
                <line x1="28" y1="220" x2="54" y2="232" />
                <line x1="34" y1="228" x2="50" y2="235" />
                <line x1="560" y1="78" x2="594" y2="96" />
                <line x1="572" y1="88" x2="598" y2="102" />
                <line x1="44" y1="392" x2="82" y2="404" />
              </g>
            </g>

            <!-- Etched HUD collimator glass brackets & horizon ticks -->
            <g
              class="glass-collimator"
              stroke={frame?.power === "off" ? "#d0843c" : "#6ee7ff"}
              stroke-opacity={frame?.power === "off" ? "0.14" : "0.22"}
              stroke-width="1"
              fill="none"
            >
              <!-- Left and right collimator framing brackets -->
              <path d="M 218 178 L 206 178 L 206 302 L 218 302" />
              <path d="M 422 178 L 434 178 L 434 302 L 422 302" />
              <!-- Subtle pitch ladder ticks -->
              <line x1="210" y1="210" x2="218" y2="210" />
              <line x1="210" y1="240" x2="222" y2="240" />
              <line x1="210" y1="270" x2="218" y2="270" />
              <line x1="430" y1="210" x2="422" y2="210" />
              <line x1="430" y1="240" x2="418" y2="240" />
              <line x1="430" y1="270" x2="422" y2="270" />
            </g>

            <!-- Ballistic glass cracks when hull/armour takes damage -->
            {#if glassDamageLevel >= 1}
              <g class="glass-cracks" stroke="#d8f0ff" fill="none" stroke-linecap="round">
                <!-- Upper-right quarter-pane ballistic star fracture -->
                <circle
                  cx="562"
                  cy="144"
                  r="4"
                  fill="#ffffff"
                  fill-opacity="0.22"
                  stroke-opacity="0.55"
                  stroke-width="1.1"
                />
                <path
                  d="M 562 144 L 528 118 M 562 144 L 598 122 M 562 144 L 610 158 M 562 144 L 574 196 M 562 144 L 532 174 M 544 130 L 535 148 M 580 133 L 586 151 M 547 160 L 568 171"
                  stroke-opacity="0.48"
                  stroke-width="1"
                />
                <path
                  d="M 528 118 L 512 112 M 574 196 L 582 222 M 532 174 L 514 184"
                  stroke-opacity="0.28"
                  stroke-width="0.75"
                />
              </g>
            {/if}
            {#if glassDamageLevel >= 2}
              <g class="glass-cracks-heavy" stroke="#e4f5ff" fill="none" stroke-linecap="round">
                <!-- Lower-left quarter-pane spiderweb fracture -->
                <circle
                  cx="76"
                  cy="354"
                  r="5.5"
                  fill="#ffffff"
                  fill-opacity="0.28"
                  stroke-opacity="0.65"
                  stroke-width="1.2"
                />
                <path
                  d="M 76 354 L 32 316 M 76 354 L 116 308 M 76 354 L 138 352 M 76 354 L 118 408 M 76 354 L 58 424 M 76 354 L 24 378"
                  stroke-opacity="0.55"
                  stroke-width="1.15"
                />
                <path
                  d="M 54 335 L 96 331 L 107 353 L 97 381 L 67 389 L 50 366 Z M 39 322 L 108 317 L 126 352 L 110 396 L 61 410 L 32 374 Z"
                  stroke-opacity="0.32"
                  stroke-width="0.8"
                />
              </g>
            {/if}

            <!-- Armored Canopy Struts, Mullions & Lower Cowling -->
            <g transform="translate({glassLookShift * 0.15}, 0)">
              <!-- Angled left and right interior canopy mullions (clear of scope, tape, reticle, and KPH bar) -->
              <path
                d="M 14 176 L 23 176 L 136 452 L 124 452 Z"
                fill="url(#cg-strut-grad)"
                stroke="#2c3848"
                stroke-width="1"
              />
              <path
                d="M 626 102 L 617 102 L 504 452 L 516 452 Z"
                fill="url(#cg-strut-grad)"
                stroke="#2c3848"
                stroke-width="1"
              />
              <!-- Inner rubber gasket highlight along the mullions -->
              <line
                x1="24"
                y1="176"
                x2="137"
                y2="452"
                stroke="#7ec8ff"
                stroke-opacity="0.16"
                stroke-width="1"
              />
              <line
                x1="616"
                y1="102"
                x2="503"
                y2="452"
                stroke="#7ec8ff"
                stroke-opacity="0.16"
                stroke-width="1"
              />

              <!-- Outer perimeter armored frame & upper-right structural gusset -->
              <path
                d="M 0 0 L 640 0 L 640 480 L 0 480 Z
                   M 6 6 L 6 164 L 18 176 L 18 430 L 130 452 L 510 452 L 622 430 L 622 102 L 524 6 Z"
                fill="url(#cg-strut-grad)"
                fill-rule="evenodd"
                stroke="#2a3544"
                stroke-width="1.2"
              />

              <!-- Lower dashboard glacis / cowling details -->
              <path
                d="M 0 432 L 128 454 L 512 454 L 640 432 L 640 480 L 0 480 Z"
                fill="url(#cg-sill-grad)"
                stroke="#313f52"
                stroke-width="1"
              />
              <!-- Defroster vent louvers on the lower cowling flanks -->
              <g stroke="#253040" stroke-width="1.5">
                <line x1="36" y1="456" x2="72" y2="462" />
                <line x1="32" y1="462" x2="68" y2="468" />
                <line x1="28" y1="468" x2="64" y2="474" />
                <line x1="604" y1="456" x2="568" y2="462" />
                <line x1="608" y1="462" x2="572" y2="468" />
                <line x1="612" y1="468" x2="576" y2="474" />
              </g>

              <!-- Structural rivets along the canopy struts -->
              <g fill="#38475a" stroke="#0b0e13" stroke-width="0.6">
                <circle cx="542" cy="14" r="2.2" />
                <circle cx="566" cy="32" r="2.2" />
                <circle cx="590" cy="52" r="2.2" />
                <circle cx="612" cy="72" r="2.2" />
                <circle cx="130" cy="462" r="2.2" />
                <circle cx="210" cy="464" r="2" />
                <circle cx="430" cy="464" r="2" />
                <circle cx="510" cy="462" r="2.2" />
              </g>

              <!-- Upper-right canopy strut status LEDs (PWR / WPN / LOCK) -->
              <g font-family="monospace" font-size="7.5" font-weight="bold">
                <circle
                  cx="572"
                  cy="16"
                  r="2.8"
                  fill={frame?.power === "off" ? "#d97724" : "#38d976"}
                />
                <text x="579" y="19" fill="#9fb0c6">PWR</text>

                <circle
                  cx="604"
                  cy="16"
                  r="2.8"
                  fill={frame?.weapons_free === false ? "#d99b26" : "#49b6ff"}
                />
                <text x="611" y="19" fill="#9fb0c6">WPN</text>

                <circle
                  cx="592"
                  cy="32"
                  r="2.8"
                  fill={frame?.lock_designator ? "#f05252" : "#2b3544"}
                />
                <text
                  x="599"
                  y="35"
                  fill={frame?.lock_designator ? "#ffb4b4" : "#68778c"}
                >{frame?.lock_designator ? `LCK:${frame.lock_designator}` : "NO LCK"}</text>
              </g>

              <!-- Center lower cowling look-angle indicator bar -->
              <g>
                <rect
                  x="276"
                  y="461"
                  width="88"
                  height="8"
                  rx="2"
                  fill="#0b0f14"
                  stroke="#273242"
                  stroke-width="0.9"
                />
                <line x1="320" y1="461" x2="320" y2="469" stroke="#4c607a" stroke-width="1" />
                <polygon
                  points="{320 - glassLookShift * 1.8},461 {316 - glassLookShift * 1.8},468 {324 - glassLookShift * 1.8},468"
                  fill={app.picture.lookMdeg === 0 ? "#5ec2ff" : "#f0b44c"}
                />
              </g>
            </g>
          </svg>
        </div>
      {/if}
      {#if !drawnHere}
        <p class="muted waiting">
          {localFailed !== null ? "Could not draw the picture." : "Waiting for the first picture…"}
        </p>
      {/if}
      <div
        class="resizer"
        class:dragging
        role="separator"
        aria-orientation="vertical"
        aria-label="resize picture"
        title="Drag to resize picture"
        onpointerdown={onResizeDown}
        onpointermove={onResizeMove}
        onpointerup={onResizeUp}
        onpointercancel={onResizeUp}
      ></div>
    </div>
  {/if}
</section>

<style>
  .picturebox {
    flex: 0 0 auto;
    max-width: 100%;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-panel);
    padding: 0.4rem 0.6rem;
  }

  header {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.4rem;
    font-family: var(--font-mono);
    font-size: 0.82rem;
  }

  .grow {
    flex: 1;
  }

  .dim {
    color: var(--fg-muted);
  }

  /* The legend is long (doc 21 cut G.12c): it wraps under the readouts on a
     narrow window rather than pushing the buttons off it. */
  .legend {
    font-size: 0.75rem;
    width: 100%;
    order: 10;
  }

  .readout {
    color: var(--fg);
  }

  .toggle:disabled {
    cursor: default;
  }

  .toggle {
    background: none;
    border: 1px solid var(--border);
    border-radius: 4px;
    color: var(--fg-muted);
    cursor: pointer;
    font: inherit;
    padding: 0.1rem 0.5rem;
  }

  .toggle:hover {
    color: var(--fg);
  }

  .wf-controls {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
  }

  .wf-btn.armed {
    border-color: var(--accent, #6aa0ff);
    color: var(--fg);
    background: var(--bg-input);
  }

  .wf-select {
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 4px;
    color: var(--fg);
    font: inherit;
    font-size: 0.75rem;
    padding: 0.05rem 0.25rem;
  }

  .wf-slider {
    width: 4.5rem;
    accent-color: var(--accent, #6aa0ff);
    cursor: pointer;
  }

  .wf-odds {
    font-size: 0.75rem;
    min-width: 2.8rem;
  }

  .zoom-controls,
  .wpn-slots {
    display: inline-flex;
    align-items: center;
    gap: 0.15rem;
  }

  .zoom-btn {
    padding: 0.05rem 0.35rem;
    font-size: 0.75rem;
  }

  .camera-pills {
    display: inline-flex;
    align-items: center;
    gap: 0.15rem;
  }

  .cam-btn {
    padding: 0.05rem 0.4rem;
    font-size: 0.74rem;
  }

  .cam-btn.active {
    border-color: var(--accent, #6aa0ff);
    color: var(--fg);
    background: var(--bg-input);
  }

  .frame {
    position: relative;
    margin-top: 0.35rem;
    width: 100%;
    outline: none;
  }

  .frame:focus-visible {
    box-shadow: 0 0 0 2px var(--accent, #6aa0ff);
  }

  canvas {
    display: block;
    width: 100%;
    height: auto;
    aspect-ratio: 4 / 3;
    image-rendering: pixelated;
    background: var(--bg);
  }

  .cockpit-glass-overlay {
    position: absolute;
    inset: 0;
    pointer-events: none;
    overflow: hidden;
    user-select: none;
  }

  .glass-pane-optics {
    position: absolute;
    inset: 0;
    background:
      radial-gradient(
        ellipse 82% 76% at 50% 48%,
        transparent 58%,
        rgba(88, 175, 235, 0.045) 84%,
        rgba(6, 12, 20, 0.42) 100%
      ),
      linear-gradient(
        126deg,
        rgba(185, 230, 255, 0.055) 0%,
        transparent 24%,
        rgba(160, 220, 255, 0.025) 48%,
        transparent 68%,
        rgba(140, 210, 255, 0.04) 100%
      ),
      repeating-linear-gradient(
        0deg,
        transparent 0px,
        transparent 3px,
        rgba(120, 200, 255, 0.012) 3px,
        rgba(120, 200, 255, 0.012) 4px
      );
    box-shadow:
      inset 0 0 28px rgba(0, 0, 0, 0.55),
      inset 0 0 2px rgba(125, 205, 255, 0.22);
  }

  .cockpit-glass-overlay.power-off .glass-pane-optics {
    background:
      radial-gradient(
        ellipse 80% 74% at 50% 48%,
        rgba(18, 12, 8, 0.14) 50%,
        rgba(180, 95, 35, 0.05) 82%,
        rgba(5, 6, 10, 0.62) 100%
      );
  }

  .cockpit-glass-svg {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    display: block;
  }

  .resizer {
    position: absolute;
    top: 0;
    right: -4px;
    bottom: 0;
    width: 10px;
    cursor: ew-resize;
    touch-action: none;
    user-select: none;
  }

  .resizer::after {
    content: "";
    position: absolute;
    right: 3px;
    bottom: 6px;
    width: 4px;
    height: 24px;
    border-right: 2px solid var(--fg-muted);
    border-left: 2px solid var(--fg-muted);
    opacity: 0.45;
    transition: opacity 0.15s ease;
  }

  .resizer:hover::after,
  .resizer.dragging::after {
    opacity: 1;
    border-color: var(--accent, #6aa0ff);
  }

  .waiting {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    margin: 0;
  }
</style>
