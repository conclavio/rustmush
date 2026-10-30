<script lang="ts">
  // The staff unit designer (doc 31 cut UD.2), beside the map studio: one
  // screen, four panes — chassis and model (with the model's baked preview),
  // rules, look (with the test hit), compare — over the DOM-free engine in
  // $lib/designer/unit-designer. The rules are checked by the server (the
  // `unit_design` invoke, staff only); the look by the picture module; the
  // design exports as the two files a person commits.
  import { onMount } from "svelte";
  import type { GameApp } from "$lib/game.svelte";
  import {
    type Draft,
    draftFrom,
    exportFiles,
    figures,
    History,
    loadDraft,
    models,
    rename,
    saveDraft,
    sections,
    setDollRows,
    setFacetArmor,
    setInternals,
    setLabel,
    setMaxSpeed,
    setModel,
    setShort,
    setWeapon,
    setWeaponName,
    templates,
    testHit,
    weaponComponents,
    weapons,
    type Json,
  } from "$lib/designer/unit-designer";
  import { UnitDesignRpc } from "$lib/designer/rpc";
  import { localCapable, pixelModule } from "$lib/pixel/local";
  import type { LookCheck, PixelModule } from "$lib/pixel/wasm";

  let { app }: { app: GameApp } = $props();

  const TEMPLATES = templates();
  const WEAPONS = weapons();
  const MODELS = models();

  const storage = typeof localStorage === "undefined" ? null : localStorage;
  const first = TEMPLATES.find((t) => t.id === "tacsim.unit.line_tank") ?? TEMPLATES[0];

  let templateId = $state(first?.id ?? "");
  let name = $state("New design");
  let draft = $state<Draft | null>(loadDraft(storage) ?? (first ? draftFrom(first, "New design") : null));
  const history = new History();
  let canUndo = $state(false);
  let canRedo = $state(false);

  let module = $state<PixelModule | null>(null);
  let rpc: UnitDesignRpc | null = null;
  let checking = $state(false);
  let refusals = $state<string[]>([]);
  let checkedPack = $state<Json | null>(null);
  let checkError = $state("");
  let hitSection = $state<string | null>(null);

  let facing = $state("front_right");
  let view = $state<"fp" | "iso">("fp");
  let pose = $state(0);
  let canvas: HTMLCanvasElement | null = $state(null);

  const template = $derived(TEMPLATES.find((t) => t.id === templateId) ?? null);
  const lookCheck = $derived<LookCheck>(
    module && draft ? module.checkLook(draft.look, draft.rules) : { refusals: [], notes: [] },
  );
  const mine = $derived(draft ? figures(draft) : null);
  const theirs = $derived(template ? figures(draftFrom(template, "t")) : null);
  const hit = $derived(draft && hitSection ? testHit(draft, hitSection) : []);
  const dollRows = $derived(
    draft && draft.look.doll && Array.isArray((draft.look.doll as Record<string, Json>).rows)
      ? ((draft.look.doll as Record<string, Json>).rows as string[])
      : [],
  );
  const legend = $derived(
    draft && draft.look.doll ? ((draft.look.doll as Record<string, Json>).legend as Record<string, Record<string, Json>>) ?? {} : {},
  );

  onMount(() => {
    rpc = new UnitDesignRpc(app.client);
    let cancelled = false;
    if (localCapable()) {
      void pixelModule()
        .then((m) => {
          if (!cancelled) module = m;
        })
        .catch(() => {
          // No picture module (a headless test): no preview, no look check.
        });
    }
    return () => {
      cancelled = true;
      rpc?.close();
    };
  });

  $effect(() => {
    if (!module || !canvas || !draft) return;
    const model = typeof draft.look.model === "string" ? draft.look.model : null;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    if (!model) return;
    const shot = module.preview({ model, view, size: 96, facing, pose, team: "red" });
    if (shot) {
      canvas.width = shot.side;
      canvas.height = shot.side;
      ctx.putImageData(new ImageData(shot.rgba, shot.side, shot.side), 0, 0);
    }
  });

  /** Apply an edit: remember the draft it replaces, forget the last check. */
  function edit(next: Draft): void {
    if (!draft) return;
    history.push(draft);
    draft = next;
    canUndo = history.canUndo;
    canRedo = history.canRedo;
    checkedPack = null;
    refusals = [];
    saveDraft(storage, next);
  }

  function undo(): void {
    if (!draft) return;
    const prev = history.undo(draft);
    if (prev) {
      draft = prev;
      saveDraft(storage, prev);
    }
    canUndo = history.canUndo;
    canRedo = history.canRedo;
    checkedPack = null;
  }

  function redo(): void {
    if (!draft) return;
    const next = history.redo(draft);
    if (next) {
      draft = next;
      saveDraft(storage, next);
    }
    canUndo = history.canUndo;
    canRedo = history.canRedo;
    checkedPack = null;
  }

  function fromTemplate(): void {
    if (template) edit(draftFrom(template, name));
  }

  async function check(): Promise<void> {
    if (!draft || !rpc) return;
    checking = true;
    checkError = "";
    const answer = await rpc.request({ op: "check", id: draft.id, version: draft.version, rules: draft.rules });
    checking = false;
    if (answer.status === "error") {
      checkError = answer.message;
      checkedPack = null;
      return;
    }
    refusals = answer.refusals;
    checkedPack = answer.pack;
  }

  function download(): void {
    if (!draft || checkedPack === null) return;
    for (const file of exportFiles(draft, checkedPack)) {
      const blob = new Blob([file.text], { type: "application/json" });
      const a = document.createElement("a");
      a.href = URL.createObjectURL(blob);
      a.download = file.path.split("/").pop() ?? "design.json";
      a.click();
      URL.revokeObjectURL(a.href);
    }
  }

  function num(e: Event): number {
    return Number((e.currentTarget as HTMLInputElement).value);
  }

  function partOf(ch: string): string | null {
    const part = legend[ch];
    return part && typeof part.section === "string" ? part.section : null;
  }
</script>

<div class="studio designer">
  <div class="studio-bar">
    <label class="field">From
      <select bind:value={templateId} aria-label="Template">
        {#each TEMPLATES as t (t.id)}
          <option value={t.id}>{t.id.replace("tacsim.unit.", "")}</option>
        {/each}
      </select>
    </label>
    <label class="field">Name <input bind:value={name} aria-label="Design name" /></label>
    <button type="button" onclick={fromTemplate}>New from template</button>
    <span class="sep"></span>
    <button type="button" onclick={undo} disabled={!canUndo}>Undo</button>
    <button type="button" onclick={redo} disabled={!canRedo}>Redo</button>
    <span class="grow"></span>
    <button type="button" onclick={check} disabled={!draft || checking}>{checking ? "Checking…" : "Check on server"}</button>
    <button
      type="button"
      onclick={download}
      disabled={checkedPack === null || refusals.length > 0 || lookCheck.refusals.length > 0}
    >Export</button>
  </div>

  {#if draft}
    <div class="panes">
      <section class="pane" aria-label="Chassis and model">
        <h3>Chassis &amp; model</h3>
        <label class="field">Id
          <input
            value={draft.id.replace("tacsim.unit.", "")}
            onchange={(e) => edit(rename(draft!, (e.currentTarget as HTMLInputElement).value))}
            aria-label="Unit id"
          />
        </label>
        <label class="field">Version
          <input class="num" type="number" min="1" value={draft.version}
            onchange={(e) => edit({ ...draft!, version: Math.max(1, num(e)) })} />
        </label>
        <label class="field">Model
          <select
            value={typeof draft.look.model === "string" ? draft.look.model : ""}
            onchange={(e) => {
              const m = MODELS.find((x) => x.id === (e.currentTarget as HTMLSelectElement).value);
              if (m) edit(setModel(draft!, m));
            }}
            aria-label="Model"
          >
            <option value="" disabled>—</option>
            {#each MODELS as m (m.id)}
              <option value={m.id}>{m.id} ({(m.size_cm[1] / 100).toFixed(1)} m)</option>
            {/each}
          </select>
        </label>
        <div class="preview">
          <canvas bind:this={canvas} width="96" height="96" aria-label="Model preview"></canvas>
          <div class="preview-ctl">
            <select bind:value={view} aria-label="View"><option value="fp">first person</option><option value="iso">2.5D</option></select>
            <select bind:value={facing} aria-label="Facing">
              {#each ["front", "front_right", "right", "rear_right", "rear", "rear_left", "left", "front_left"] as f (f)}
                <option value={f}>{f.replace("_", " ")}</option>
              {/each}
            </select>
            <select bind:value={pose} aria-label="Pose">
              <option value={0}>still</option><option value={1}>moving</option><option value={5}>wreck</option>
            </select>
          </div>
          {#if !module}<p class="muted">No picture module here: no preview, no look check.</p>{/if}
        </div>
      </section>

      <section class="pane" aria-label="Rules">
        <h3>Rules</h3>
        <label class="field">Label
          <input value={String(draft.rules.label ?? "")} onchange={(e) => edit(setLabel(draft!, (e.currentTarget as HTMLInputElement).value))} />
        </label>
        <label class="field">Top speed (cm/s)
          <input class="num" type="number" min="0" value={mine?.speedCms ?? 0} onchange={(e) => edit(setMaxSpeed(draft!, num(e)))} />
        </label>
        <table class="grid">
          <thead><tr><th>Section</th><th>Internals</th><th>Armour by facet</th></tr></thead>
          <tbody>
            {#each sections(draft) as s (s.id)}
              <tr>
                <td>{s.id}</td>
                <td><input class="num" type="number" min="1" value={s.internals} onchange={(e) => edit(setInternals(draft!, s.id, num(e)))} /></td>
                <td>
                  {#each s.facets as f (f.id)}
                    <label class="facet">{f.id}
                      <input class="num" type="number" min="1" value={f.armor} onchange={(e) => edit(setFacetArmor(draft!, s.id, f.id, num(e)))} />
                    </label>
                  {/each}
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
        <table class="grid">
          <thead><tr><th>Weapon</th><th>On</th><th>Fits</th></tr></thead>
          <tbody>
            {#each weaponComponents(draft) as w (w.id)}
              <tr>
                <td>{w.id}</td>
                <td>{w.section}</td>
                <td>
                  <select value={w.weapon} onchange={(e) => edit(setWeapon(draft!, w.id, (e.currentTarget as HTMLSelectElement).value))}>
                    {#each WEAPONS as x (x.id)}<option value={x.id}>{x.label}</option>{/each}
                  </select>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </section>

      <section class="pane" aria-label="Look">
        <h3>Look</h3>
        <label class="field">Short <input class="num" value={String(draft.look.short ?? "")} onchange={(e) => edit(setShort(draft!, (e.currentTarget as HTMLInputElement).value))} /></label>
        {#each weaponComponents(draft) as w (w.id)}
          <label class="field">{w.id}
            <input class="num" value={String(((draft.look.weapons as Record<string, Json>) ?? {})[w.id] ?? "")}
              onchange={(e) => edit(setWeaponName(draft!, w.id, (e.currentTarget as HTMLInputElement).value))} />
          </label>
        {/each}
        <p class="muted">Doll — click a part for the test hit.</p>
        <div class="doll" role="grid" aria-label="Doll">
          {#each dollRows as row, y (y)}
            <div class="doll-row">
              {#each [...row] as ch, x (x)}
                {@const section = partOf(ch)}
                <button
                  type="button"
                  class="cell"
                  class:part={section !== null}
                  class:hit={section !== null && section === hitSection}
                  disabled={section === null}
                  title={section ?? ""}
                  onclick={() => (hitSection = section === hitSection ? null : section)}
                >{ch === "." ? "" : ch}</button>
              {/each}
            </div>
          {/each}
        </div>
        <textarea rows={Math.max(3, dollRows.length)} value={dollRows.join("\n")} aria-label="Doll rows"
          onchange={(e) => edit(setDollRows(draft!, (e.currentTarget as HTMLTextAreaElement).value.split("\n")))}></textarea>
        {#if hitSection}
          <div class="hit-box">
            <strong>{hitSection} destroyed:</strong>
            {#if hit.length === 0}<span>nothing follows</span>{/if}
            <ul>{#each hit as h (h)}<li>{h}</li>{/each}</ul>
          </div>
        {/if}
      </section>

      <section class="pane" aria-label="Compare">
        <h3>Compare with {template?.id.replace("tacsim.unit.", "") ?? "—"}</h3>
        {#if mine && theirs}
          <table class="grid">
            <tbody>
              <tr><td>Armour (all facets)</td><td>{mine.armour}</td><td>{theirs.armour}</td></tr>
              <tr><td>Internals</td><td>{mine.internals}</td><td>{theirs.internals}</td></tr>
              <tr><td>Top speed (kph)</td><td>{mine.speedCms === null ? "—" : Math.round(mine.speedCms * 0.036)}</td><td>{theirs.speedCms === null ? "—" : Math.round(theirs.speedCms * 0.036)}</td></tr>
              <tr><td>Weapons</td><td>{mine.weapons.map((w) => w.replace("tacsim.weapon.", "")).join(", ")}</td><td>{theirs.weapons.map((w) => w.replace("tacsim.weapon.", "")).join(", ")}</td></tr>
            </tbody>
          </table>
        {/if}
        <h3>Checks</h3>
        {#if checkError}<p class="refusal">{checkError}</p>{/if}
        {#if checkedPack !== null && refusals.length === 0}<p class="ok">The server takes the rules.</p>{/if}
        {#each refusals as r (r)}<p class="refusal">rules: {r}</p>{/each}
        {#each lookCheck.refusals as r (r)}<p class="refusal">look: {r}</p>{/each}
        {#each lookCheck.notes as r (r)}<p class="note">look: {r}</p>{/each}
      </section>
    </div>
  {:else}
    <p class="muted">No shipped unit to start from.</p>
  {/if}
</div>

<style>
  .studio {
    display: flex;
    flex-direction: column;
    gap: 0.45rem;
    font-size: 0.8rem;
  }
  .studio-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.45rem;
    padding: 0.4rem 0.6rem;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-panel);
  }
  .field {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    color: var(--fg-muted);
  }
  input,
  select,
  textarea {
    background: var(--bg-input);
    color: var(--fg);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 0.2rem 0.4rem;
    font: inherit;
  }
  textarea {
    font-family: var(--font-mono, monospace);
    width: 100%;
  }
  .num {
    width: 4.5rem;
  }
  .sep {
    width: 1px;
    height: 1.2rem;
    background: var(--border);
  }
  .grow {
    flex: 1;
  }
  .panes {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(20rem, 1fr));
    gap: 0.45rem;
  }
  .pane {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    padding: 0.5rem 0.6rem;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-panel);
    min-width: 0;
  }
  h3 {
    margin: 0;
    font-size: 0.85rem;
  }
  .preview {
    display: flex;
    gap: 0.5rem;
    align-items: flex-start;
  }
  canvas {
    width: 192px;
    height: 192px;
    image-rendering: pixelated;
    background: var(--bg-input);
    border: 1px solid var(--border);
  }
  .preview-ctl {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
  }
  .grid {
    border-collapse: collapse;
    width: 100%;
  }
  .grid td,
  .grid th {
    border-bottom: 1px solid var(--border);
    padding: 0.15rem 0.3rem;
    text-align: left;
    vertical-align: top;
  }
  .facet {
    display: inline-flex;
    gap: 0.2rem;
    align-items: center;
    margin-right: 0.3rem;
    color: var(--fg-muted);
  }
  .doll {
    font-family: var(--font-mono, monospace);
    line-height: 1;
  }
  .doll-row {
    display: flex;
  }
  .cell {
    width: 1rem;
    height: 1rem;
    padding: 0;
    border: none;
    background: transparent;
    color: var(--fg-muted);
    font: inherit;
    font-size: 0.7rem;
  }
  .cell.part {
    background: var(--bg-input);
    cursor: pointer;
  }
  .cell.hit {
    background: var(--danger, #a33);
    color: var(--fg);
  }
  .hit-box ul {
    margin: 0.2rem 0 0 1rem;
    padding: 0;
  }
  .muted {
    color: var(--fg-muted);
    margin: 0;
  }
  .refusal {
    color: var(--danger, #d55);
    margin: 0;
  }
  .note {
    color: var(--warn, #ca5);
    margin: 0;
  }
  .ok {
    color: var(--ok, #5a5);
    margin: 0;
  }
</style>
