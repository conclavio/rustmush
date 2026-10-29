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
  import {
    DEFAULT_MAX_SPEED_KPH,
    DEFAULT_REVERSE_KPH,
    HELM_REPEAT_MS,
    LOCK_PENDING_MS,
    LOOK_STEP_MS,
    PICTURE_PREF_KEY,
    SPEED_COALESCE_MS,
    gateLine,
    gateWord,
    helmLine,
    nextLock,
    pictureKey,
    speedLine,
    speedOutstanding,
    steppedSpeed,
    swingLook,
  } from "$lib/picture";
  import {
    FRAME_H,
    FRAME_W,
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
  /** The overhead is chosen (doc 21 cut G.12c; doc 28 P2). */
  const overhead = $derived(app.picture.kind === "overhead");
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

  // The latest heading, for the helm's repeats (a prop read inside a timer).
  let heading = 0;
  $effect(() => {
    heading = headingDeg;
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
  $effect(() => () => release());
  $effect(() => {
    const onVisibility = (): void => {
      visible = document.visibilityState === "visible";
      if (!visible) release();
    };
    document.addEventListener("visibilitychange", onVisibility);
    window.addEventListener("blur", release);
    return () => {
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener("blur", release);
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
        if (
          l.draw(
            app.picture.cockpitData,
            app.picture.mapData,
            app.picture.lookMdeg,
            app.picture.kind,
          )
        ) {
          if (c.width !== FRAME_W || c.height !== FRAME_H) {
            c.width = FRAME_W;
            c.height = FRAME_H;
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

  // A held turn's order is extended while the key is down.
  $effect(() => {
    const dir = turning;
    if (dir === 0) return;
    app.helm(helmLine(untrack(() => heading), dir));
    const timer = setInterval(() => app.helm(helmLine(heading, dir)), HELM_REPEAT_MS);
    return () => clearInterval(timer);
  });

  function stopTurn(): void {
    if (turning !== 0) {
      turning = 0;
      app.helm(helmLine(heading, 0));
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
    app.helm(helmLine(heading, 0));
  }

  // A step still waiting when the view goes (the screen changed) is sent, not
  // lost: the speed stands across screens, only the turn and the look stop.
  $effect(() => () => flushSpeed());

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

  function onKeyDown(ev: KeyboardEvent): void {
    // A chord is the browser's (Ctrl-W, Ctrl-T, Alt-F …), never the picture's.
    if (ev.ctrlKey || ev.altKey || ev.metaKey) return;
    const key = pictureKey(ev.key);
    if (key === null) return;
    if (key.kind === "order") {
      // Always taken while the picture has focus, so the browser's F1 help,
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
        if (!ev.repeat) turning = key.dir;
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
        if (!ev.repeat) app.picture.kind = overhead ? "cockpit" : "overhead";
        break;
      case "gate":
        if (!ev.repeat) app.helm(gateLine(frame?.weapons_free ?? null));
        break;
      case "volley":
        if (!ev.repeat) app.helm("FIRE ALL");
        break;
      case "lock":
        if (!ev.repeat) cycleLock(key.dir);
        break;
    }
  }

  function onKeyUp(ev: KeyboardEvent): void {
    const key = pictureKey(ev.key);
    if (key?.kind === "turn" && key.dir === turning) {
      ev.preventDefault();
      stopTurn();
    }
  }
</script>

<section class="picturebox" aria-label="picture">
  <header>
    <strong>picture</strong>
    <span class="grow"></span>
    {#if wanted && active}
      {#if outstandingKph !== null}
        <span class="readout" aria-label="speed ordered">spd {frameKph}→{outstandingKph}</span>
      {/if}
      {#if frame !== null}
        <span class="readout">{gateWord(frame.weapons_free)}</span>
        {#if frame.mode === "range"}
          <!-- 16c Q-PT1: a range says so beside the gate, as the TUI's top line does. -->
          <span class="readout" aria-label="match mode">RANGE</span>
        {/if}
      {/if}
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
      aria-label={overhead
        ? "overhead view; the cockpit's flight keys work here, v returns to first person"
        : "first-person view; a and d or the arrows turn, w and s set the speed, q and e swing the look, v the overhead"}
      onkeydown={onKeyDown}
      onkeyup={onKeyUp}
      onblur={release}
    >
      <canvas bind:this={canvas} aria-hidden="true"></canvas>
      {#if !drawnHere}
        <p class="muted waiting">
          {localFailed !== null ? "Could not draw the picture." : "Waiting for the first picture…"}
        </p>
      {/if}
    </div>
  {/if}
</section>

<style>
  .picturebox {
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-panel);
    padding: 0.4rem 0.6rem;
  }

  header {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.5rem;
    font-family: var(--font-mono);
    font-size: 0.85rem;
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
    font-size: 0.8rem;
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

  .frame {
    position: relative;
    margin-top: 0.4rem;
    max-width: 960px;
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

  .waiting {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    margin: 0;
  }
</style>
