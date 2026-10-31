<script lang="ts">
  // The `worldgen.draft` panel feed: the builder-facing preview for an
  // `@gen`-generated room draft (design 03 §5.2 / mush-worldgen's `@gen`
  // command surface). Nothing here ever writes the world directly — every
  // button sends the existing `@gen/*` command as a plain Line, exactly
  // like the mail panel's `@mail/quick`; the server is the only thing that
  // ever commits a draft (`@gen/accept`), and only through the ordinary
  // attribute-set path. See $lib/worldgen.ts for the payload contract this
  // renders against and web/README.md for the full writeup.
  import type { GameApp } from "$lib/game.svelte";
  import ContentStudio from "./ContentStudio.svelte";
  import MapEditorStudio from "./MapEditorStudio.svelte";
  import UnitDesignerStudio from "./UnitDesignerStudio.svelte";

  let { app }: { app: GameApp } = $props();

  const wg = $derived(app.worldgen);
  let buildTab = $state<"studio" | "units" | "content" | "drafts">("studio");

  let retrying = $state(false);
  let guidance = $state("");

  function startRetry(): void {
    guidance = "";
    retrying = true;
  }

  function sendRetry(): void {
    app.worldgenRetry(guidance);
    retrying = false;
    guidance = "";
  }

  function cancelRetry(): void {
    retrying = false;
    guidance = "";
  }

  function accept(): void {
    app.worldgenAccept();
  }

  function discard(): void {
    retrying = false;
    app.worldgenDiscard();
  }
</script>

<div class="build-shell">
  <div class="build-subnav" role="tablist" aria-label="Builder tools">
    <button
      type="button"
      role="tab"
      aria-selected={buildTab === "studio"}
      class="subnav-tab"
      class:active={buildTab === "studio"}
      onclick={() => (buildTab = "studio")}
    >
      Terrain &amp; Arena Studio
    </button>
    <button
      type="button"
      role="tab"
      aria-selected={buildTab === "units"}
      class="subnav-tab"
      class:active={buildTab === "units"}
      onclick={() => (buildTab = "units")}
    >
      Unit Designer
    </button>
    <button
      type="button"
      role="tab"
      aria-selected={buildTab === "content"}
      class="subnav-tab"
      class:active={buildTab === "content"}
      onclick={() => (buildTab = "content")}
    >
      Content Studio
    </button>
    <button
      type="button"
      role="tab"
      aria-selected={buildTab === "drafts"}
      class="subnav-tab"
      class:active={buildTab === "drafts"}
      onclick={() => (buildTab = "drafts")}
    >
      Room Drafts (@gen)
      {#if wg.draft !== null}
        <span class="draftbadge">DRAFT</span>
      {/if}
    </button>
  </div>

  {#if buildTab === "studio"}
    <div class="studio-wrap">
      <MapEditorStudio {app} />
    </div>
  {:else if buildTab === "units"}
    <!-- Doc 31 cut UD.2: staff only — the server refuses anyone else's check. -->
    <div class="studio-wrap">
      <UnitDesignerStudio {app} />
    </div>
  {:else if buildTab === "content"}
    <!-- Doc 34 cut CS.0a: staff Content Studio — server-checked packs and tables. -->
    <div class="studio-wrap">
      <ContentStudio {app} />
    </div>
  {:else}
<div class="worldgen">
  {#if wg.draft === null}
    <div class="empty">
      <p><strong>Worldgen drafts</strong></p>
      <p class="muted">
        No pending draft. In the terminal, describe a room you can edit with
        <code>@gen here=&lt;prose&gt;</code> — the generated draft appears here for
        review before anything is written. Requires the <code>BUILDER_LLM</code> power.
      </p>
    </div>
  {:else}
    {@const d = wg.draft}
    <div class="head">
      <span class="roomname">{d.roomName || d.room}</span>
      <span class="draftbadge">DRAFT</span>
      <span class="grow"></span>
      {#if d.model}<span class="model muted" title="model that produced this draft">{d.model}</span>{/if}
    </div>

    {#if wg.pending}
      <p class="pending">Regenerating… the previous draft stays below until the new one arrives.</p>
    {/if}

    <div class="body">
      <section class="block">
        <h3>Description</h3>
        <div class="compare">
          <div class="col">
            <h4>Current</h4>
            {#if d.currentDesc.trim().length > 0}
              <p class="desc">{d.currentDesc}</p>
            {:else}
              <p class="desc muted">(empty)</p>
            {/if}
          </div>
          <div class="col proposed">
            <h4>Proposed</h4>
            {#if d.proposed.desc.trim().length > 0}
              <p class="desc">{d.proposed.desc}</p>
            {:else}
              <p class="desc muted">(empty)</p>
            {/if}
          </div>
        </div>
      </section>

      {#if d.proposed.shortDesc.trim().length > 0}
        <section class="block">
          <h3>Short desc</h3>
          <p class="desc">{d.proposed.shortDesc}</p>
        </section>
      {/if}

      {#if d.proposed.details.length > 0}
        <section class="block">
          <h3>Details</h3>
          {#each d.proposed.details as det, i (i)}
            <div class="entry">
              <span class="entrylabel">{det.keyword || "(no keyword)"}</span>
              <p class="desc">{det.desc}</p>
            </div>
          {/each}
        </section>
      {/if}

      {#if d.proposed.exits.length > 0}
        <section class="block">
          <h3>Exits</h3>
          {#each d.proposed.exits as ex, i (i)}
            <div class="entry">
              <span class="entrylabel">{ex.name || ex.direction || "(exit)"}</span>
              {#if ex.desc.trim().length > 0}<p class="desc">{ex.desc}</p>{/if}
              {#if ex.transitMsg.trim().length > 0}
                <p class="desc muted small">Transit: {ex.transitMsg}</p>
              {/if}
            </div>
          {/each}
        </section>
      {/if}
    </div>

    <p class="hint muted">
      Nothing here is written until you accept it — <code>@gen/accept</code> writes the
      full draft through the normal edit path.
    </p>

    {#if retrying}
      <form class="retryform" onsubmit={(ev) => { ev.preventDefault(); sendRetry(); }}>
        <label>
          Extra guidance (optional)
          <input bind:value={guidance} placeholder="e.g. make it darker, mention the smell of rain" />
        </label>
        <div class="row">
          <button type="button" class="ghost" onclick={cancelRetry}>Cancel</button>
          <button type="submit" class="primary">Regenerate</button>
        </div>
      </form>
    {:else}
      <div class="actions">
        <button type="button" class="ghost" onclick={discard}>Discard</button>
        <button type="button" class="ghost" onclick={startRetry}>Retry…</button>
        <button type="button" class="primary" onclick={accept}>Accept</button>
      </div>
    {/if}
  {/if}
</div>
  {/if}
</div>

<style>
  .build-shell {
    height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow: hidden;
  }

  .build-subnav {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.3rem 0.6rem;
    border-bottom: 1px solid var(--border);
    background: var(--bg-panel, #0d111a);
    flex-shrink: 0;
  }

  .subnav-tab {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    background: var(--bg-input, #121722);
    color: var(--fg-muted);
    border: 1px solid var(--border);
    border-radius: 5px;
    padding: 0.22rem 0.65rem;
    font-size: 0.76rem;
    font-weight: 600;
    cursor: pointer;
  }

  .subnav-tab.active {
    background: rgba(74, 158, 255, 0.16);
    color: var(--fg);
    border-color: var(--accent, #4a9eff);
  }

  .studio-wrap {
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }

  .worldgen {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 0.6rem 0.75rem 1rem;
    font-size: 0.85rem;
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
  }

  .empty {
    padding: 0.5rem 0.1rem;
  }

  .empty p {
    margin: 0 0 0.4rem;
  }

  .muted {
    color: var(--fg-muted);
  }

  .small {
    font-size: 0.78rem;
  }

  code {
    font-family: var(--font-mono);
    font-size: 0.9em;
  }

  .head {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .roomname {
    font-weight: 700;
    color: var(--fg);
  }

  .draftbadge {
    display: inline-flex;
    align-items: center;
    border-radius: 999px;
    padding: 0.05rem 0.55rem;
    font-size: 0.68rem;
    font-weight: 700;
    letter-spacing: 0.04em;
    color: var(--warn);
    border: 1px solid var(--warn);
  }

  .grow {
    flex: 1;
  }

  .model {
    font-size: 0.72rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 8rem;
  }

  .pending {
    margin: 0;
    font-size: 0.78rem;
    color: var(--warn);
  }

  .body {
    display: flex;
    flex-direction: column;
    gap: 0.7rem;
  }

  .block h3 {
    margin: 0 0 0.35rem;
    font-size: 0.78rem;
    letter-spacing: 0.03em;
    text-transform: uppercase;
    color: var(--fg-muted);
  }

  .compare {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 0.5rem;
  }

  .col h4 {
    margin: 0 0 0.2rem;
    font-size: 0.72rem;
    color: var(--fg-faint);
    font-weight: 600;
    text-transform: uppercase;
  }

  .col.proposed h4 {
    color: var(--accent);
  }

  .col.proposed .desc {
    border-color: var(--accent-dim);
  }

  .desc {
    margin: 0;
    padding: 0.4rem 0.5rem;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 6px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    line-height: 1.4;
  }

  .desc.muted {
    font-style: italic;
  }

  .entry {
    margin-bottom: 0.5rem;
  }

  .entry:last-child {
    margin-bottom: 0;
  }

  .entrylabel {
    display: block;
    font-weight: 600;
    color: var(--fg);
    font-size: 0.8rem;
    margin-bottom: 0.15rem;
  }

  .hint {
    margin: 0;
    font-size: 0.75rem;
  }

  .actions,
  .row {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
  }

  .retryform {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    border-top: 1px solid var(--border);
    padding-top: 0.6rem;
  }

  .retryform label {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    font-size: 0.78rem;
    color: var(--fg-muted);
  }

  .retryform input {
    background: var(--bg-input);
    color: var(--fg);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 0.35rem 0.5rem;
    font: inherit;
    font-size: 0.85rem;
  }

  .retryform input:focus {
    outline: none;
    border-color: var(--accent-dim);
  }

  button {
    border-radius: 6px;
    padding: 0.3rem 0.8rem;
    font: inherit;
    font-size: 0.82rem;
    cursor: pointer;
    border: 1px solid var(--border);
  }

  .ghost {
    background: none;
    color: var(--fg-muted);
  }

  .ghost:hover {
    color: var(--fg);
  }

  .primary {
    background: var(--accent-dim);
    color: var(--fg);
    border-color: transparent;
  }

  .primary:hover {
    filter: brightness(1.1);
  }
</style>
