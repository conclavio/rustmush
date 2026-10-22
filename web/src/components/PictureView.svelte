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
  import type { GameApp } from "$lib/game.svelte";
  import {
    HELM_REPEAT_MS,
    LOOK_STEP_MS,
    PICTURE_PREF_KEY,
    helmLine,
    pictureKey,
    swingLook,
  } from "$lib/picture";
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

  // The latest heading, for the helm's repeats (a prop read inside a timer).
  let heading = 0;
  $effect(() => {
    heading = headingDeg;
  });

  function toggle(): void {
    wanted = !wanted;
    storageSet(PICTURE_PREF_KEY, wanted ? "on" : "off");
  }

  // Asked for while wanted and visible; stopped when either goes, and when
  // the view is torn down.
  $effect(() => {
    const want = wanted && visible;
    untrack(() => app.setPicture(want, want ? app.picture.lookMdeg : 0));
    return () => untrack(() => app.setPicture(false, 0));
  });

  $effect(() => {
    const onVisibility = (): void => {
      visible = document.visibilityState === "visible";
    };
    document.addEventListener("visibilitychange", onVisibility);
    return () => document.removeEventListener("visibilitychange", onVisibility);
  });

  // Each frame drawn as it arrives, at its own size; CSS scales the canvas.
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
      app.setPicture(true, swingLook(app.picture.lookMdeg, dir, LOOK_STEP_MS));
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

  function onKeyDown(ev: KeyboardEvent): void {
    const key = pictureKey(ev.key);
    if (key === null || !app.picture.on) return;
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
        app.setPicture(true, 0);
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
    {#if wanted && app.picture.on}
      <span class="dim">arrows turn · , . look · c centre · space stop</span>
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
      onblur={() => {
        lookTurn = 0;
        stopTurn();
      }}
    >
      <canvas bind:this={canvas} aria-hidden="true"></canvas>
      {#if app.picture.frame === null}
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
