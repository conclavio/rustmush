<script lang="ts">
  // The cockpit's picture in the web client (doc 21 cut G.3c): the
  // first-person view the gateway draws from this connection's own feeds —
  // the flight HUD over it, the other machines in their team's colours — sent
  // as one PNG a binary message while asked for (`app.setPicture`), and drawn
  // here as it arrives.
  //
  // It is asked for only while wanted AND the page is visible, and stopped
  // when this view goes (the tab changes, the pilot leaves the seat): a
  // picture nobody is looking at costs the gateway a render and 100 KB/s.
  // Whether the viewer wants it is remembered in this browser alone.
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
  // **Drawn here** (doc 21 cut G.3b): where the browser can (WebAssembly and
  // `ImageData`), the picture is drawn in it, by the engine's own renderer
  // built to wasm, from the keyframes this connection already holds — the
  // gateway is not asked and draws nothing. The server's picture is the
  // fallback (a module that will not load or that traps) and a choice the
  // header offers, remembered in this browser. Drawn here, the look is the
  // store's alone and nothing is sent for it.
  import type { GameApp } from "$lib/game.svelte";
  import {
    HELM_REPEAT_MS,
    LOOK_STEP_MS,
    PICTURE_PREF_KEY,
    helmLine,
    pictureKey,
    swingLook,
  } from "$lib/picture";
  import {
    FRAME_H,
    FRAME_W,
    LOCAL_FRAME_MS,
    LocalPicture,
    PICTURE_SOURCE_KEY,
    localCapable,
    pixelModule,
  } from "$lib/pixel/local";
  import { storageGet, storageSet } from "$lib/storage";
  import { untrack } from "svelte";

  let { app, headingDeg }: { app: GameApp; headingDeg: number } = $props();

  let wanted = $state(storageGet(PICTURE_PREF_KEY) === "on");
  let visible = $state(typeof document === "undefined" || document.visibilityState === "visible");
  let canvas: HTMLCanvasElement | undefined = $state();
  /** The look's swing: `-1` left, `0` still, `1` right. */
  let lookTurn = $state<-1 | 0 | 1>(0);
  /** The held turn: `-1` left, `0` none, `1` right. */
  let turning = $state<-1 | 0 | 1>(0);

  /** Drawn here or by the server, as this browser chose (here by default,
   * where it can be). */
  let source = $state<"here" | "server">(
    storageGet(PICTURE_SOURCE_KEY) === "server" || !localCapable() ? "server" : "here",
  );
  /** Why the picture could not be drawn here, once it could not. */
  let localFailed = $state<string | null>(null);
  let local = $state.raw<LocalPicture | null>(null);
  /** A frame has been drawn here. */
  let drawnHere = $state(false);
  const here = $derived(source === "here" && localFailed === null);
  /** Whether the picture is up: drawn here while wanted and visible, or the
   * server's while asked for. */
  const active = $derived(here ? wanted && visible : app.picture.on);

  function toggleSource(): void {
    source = source === "here" ? "server" : "here";
    storageSet(PICTURE_SOURCE_KEY, source);
    localFailed = null;
  }

  /** The look, in millidegrees: the store's alone while drawn here, asked of
   * the server otherwise. */
  function lookTo(mdeg: number): void {
    if (here) {
      app.picture.lookMdeg = mdeg;
    } else {
      app.setPicture(true, mdeg);
    }
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

  // The server's picture: asked for while wanted and visible and not drawn
  // here; stopped when any of those goes, and when the view is torn down.
  $effect(() => {
    const want = wanted && visible && !here;
    untrack(() => app.setPicture(want, want ? app.picture.lookMdeg : 0));
    return () => untrack(() => app.setPicture(false, 0));
  });

  // The page's visibility, and the two ways a key's release can go unseen:
  // the page hidden (the keys' events stop: the tab switched, the window
  // minimised) and the window blurred (another window took the keys).
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
    if (!(wanted && here) || local !== null) return;
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
  // visible, put on the canvas only when it changed.
  $effect(() => {
    const l = local;
    const c = canvas;
    if (!l || !c || !(wanted && visible && here)) return;
    let raf = 0;
    let last = -Infinity;
    const tick = (t: number): void => {
      raf = requestAnimationFrame(tick);
      if (t - last < LOCAL_FRAME_MS) return;
      last = t;
      try {
        if (l.draw(app.picture.cockpitData, app.picture.mapData, app.picture.lookMdeg)) {
          if (c.width !== FRAME_W || c.height !== FRAME_H) {
            c.width = FRAME_W;
            c.height = FRAME_H;
          }
          c.getContext("2d")?.putImageData(l.image(), 0, 0);
          drawnHere = true;
        }
      } catch (err) {
        cancelAnimationFrame(raf);
        fail(err);
      }
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  });

  // The server's frames drawn as they arrive, at their own size; CSS scales
  // the canvas.
  $effect(() => {
    const blob = app.picture.frame;
    const c = canvas;
    if (!blob || !c || typeof createImageBitmap === "undefined") return;
    let live = true;
    void createImageBitmap(blob).then((bmp) => {
      if (live) {
        c.width = bmp.width;
        c.height = bmp.height;
        c.getContext("2d")?.drawImage(bmp, 0, 0);
      }
      bmp.close();
    });
    return () => {
      live = false;
    };
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

  function onKeyDown(ev: KeyboardEvent): void {
    const key = pictureKey(ev.key);
    if (key === null || !active) return;
    ev.preventDefault();
    switch (key.kind) {
      case "turn":
        if (!ev.repeat) turning = key.dir;
        break;
      case "look":
        lookTurn = lookTurn === key.dir ? 0 : key.dir;
        break;
      case "centre":
        lookTurn = 0;
        lookTo(0);
        break;
      case "stop":
        lookTurn = 0;
        stopTurn();
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
      <span class="dim">arrows turn · , . look · c centre · space stop</span>
    {/if}
    {#if wanted}
      <button
        type="button"
        class="toggle"
        onclick={toggleSource}
        title={localFailed ?? (here ? "drawn by this browser" : "drawn by the server")}
      >
        {here ? "drawn here" : localFailed ? "drawn by the server (could not draw here)" : "drawn by the server"}
      </button>
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
      aria-label="first-person view; arrows turn the tank, comma and full stop swing the look"
      onkeydown={onKeyDown}
      onkeyup={onKeyUp}
      onblur={release}
    >
      <canvas bind:this={canvas} aria-hidden="true"></canvas>
      {#if here ? !drawnHere : app.picture.frame === null}
        <p class="muted waiting">Waiting for the first picture…</p>
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
