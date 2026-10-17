<script lang="ts">
  import { onDestroy } from "svelte";
  import type { MindsStore } from "$lib/stores/minds.svelte";
  import type { CardValue } from "$lib/mind-card";
  import MindCard from "./MindCard.svelte";
  import MindEditor from "./MindEditor.svelte";
  import MindBulkControls from "./MindBulkControls.svelte";

  let { minds, prefill }: { minds: MindsStore; prefill: (line: string) => void } = $props();
  let dialog = $state<HTMLDialogElement>();
  let returnFocus: HTMLElement | null = null;
  let name = $state("");
  let from = $state("standard");
  let revision = $state("");
  let profileName = $state("");
  let preset = $state("");
  let slot = $state(1);
  let rule = $state("");
  let notes = $state("");
  let version = $state("");
  let definition = $state("");
  let newName = $state("");
  let oldVersion = $state("");
  let newVersion = $state("");
  let removeConfirmed = $state(false);
  const card = $derived(minds.card);
  const unsupportedRule = $derived(minds.rulesSource[minds.profile]?.[slot - 1] === null);
  const presets = $derived(minds.draft?.presets && typeof minds.draft.presets === "object"
    && !Array.isArray(minds.draft.presets) ? Object.keys(minds.draft.presets) : []);

  $effect(() => {
    const d = dialog;
    if (!d) return;
    if (minds.open && !d.open) {
      returnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      d.showModal();
    } else if (!minds.open && d.open) {
      d.close();
      if (returnFocus?.isConnected) returnFocus.focus();
      returnFocus = null;
    }
  });
  onDestroy(() => { dialog?.close(); if (returnFocus?.isConnected) returnFocus.focus(); });
  $effect(() => {
    const draft = minds.draft;
    if (draft) {
      const { source: _source, ...envelope } = $state.snapshot(draft);
      definition = JSON.stringify(envelope, null, 2);
    } else definition = "";
  });
  $effect(() => { rule = minds.rulesSource[minds.profile]?.[slot - 1] ?? ""; });
  $effect(() => { void minds.name; removeConfirmed = false; });
  function setValue(field: string, value: CardValue): void {
    void minds.edit({ kind: "set", profile: minds.profile, field,
      value: Array.isArray(value) ? value.join(",") : String(value) });
  }
  function command(line: string): void { minds.open = false; prefill(line); }
</script>

<dialog bind:this={dialog} aria-label="Mind workshop" oncancel={event => { event.preventDefault(); minds.open = false; }}>
  <header><h2>Mind workshop</h2><button type="button" onclick={() => minds.open = false}>Close</button></header>
  <p class="hint">Working copies belong to this character. Saving creates an immutable revision; existing seals keep their selected revision.</p>
  <div class="layout">
    <aside aria-label="Mind library">
      <h3>Your minds</h3>
      <button type="button" disabled={minds.busy} onclick={() => void minds.show()}>Refresh library</button>
      <button type="button" disabled={minds.busy} onclick={() => void minds.prune()}>Clean unreferenced revisions</button>
      {#each minds.names as item (item)}
        <button type="button" class:selected={minds.name === item} disabled={minds.busy}
          onclick={() => { name = item; void minds.load(item); }}>{item}</button>
      {/each}
      <form onsubmit={event => { event.preventDefault(); void minds.create(name, from.trim() || null); }}>
        <label>Mind name<input bind:value={name} maxlength="64" required disabled={minds.busy} /></label>
        <label>Copy from<input bind:value={from} placeholder="standard or an owned revision" disabled={minds.busy} /></label>
        <button disabled={minds.busy || !name}>Create working copy</button>
      </form>
      <form onsubmit={event => { event.preventDefault(); void minds.load(name, revision.trim() || null); }}>
        <label>Revision<input bind:value={revision} placeholder="blank for working copy" disabled={minds.busy} /></label>
        <button disabled={minds.busy || !name}>Read revision</button>
      </form>
    </aside>
    <main>
      {#if minds.error}<p role="alert">{minds.error}</p>{/if}
      {#if minds.notice}<p role="status">{minds.notice}</p>{/if}
      {#if minds.busy}<p role="status">{minds.job === null ? "Waiting for the server…" : `Running job ${minds.job}…`}
        {#if minds.job !== null}<button type="button" onclick={() => void minds.cancel()}>Cancel job</button>{/if}</p>{/if}
      {#if minds.draft}
        <h3>{minds.name} · {minds.revision === null ? "working copy" : `revision ${minds.revision}`}{minds.dirty ? " · unsent changes" : ""}</h3>
        {#if minds.revision !== null}<p>Saved revisions are read-only. Create a working copy from this revision to change it.</p>{/if}
        <div class="toolbar">
          <button type="button" disabled={minds.busy} onclick={() => void minds.load(minds.name, minds.revision, true)}>{minds.dirty ? "Discard unsent changes and reload" : "Reload from server"}</button>
          <button type="button" disabled={minds.readonly} onclick={() => void minds.check()}>Check six pictures</button>
        </div>
        <MindEditor value={minds.draft.source} disabled={minds.readonly} diagnostics={minds.diagnostics}
          onchange={source => minds.setSource(source)} />
        {#if minds.diagnostics.length}<ul class="diagnostics" aria-label="Mind diagnostics">
          {#each minds.diagnostics as diagnostic, i (i)}<li>{diagnostic.field}{diagnostic.line === null ? "" : `:${diagnostic.line}`}{diagnostic.column === null ? "" : `:${diagnostic.column}`} — {diagnostic.message}</li>{/each}
        </ul>{/if}
        {#if minds.reports.length}<div class="reports"><table><caption>Isolated checks</caption>
          <thead><tr><th>Picture</th><th>Armings</th><th>Operations</th><th>Host fees</th><th>Result</th></tr></thead>
          <tbody>{#each minds.reports as report, i (i)}<tr><td>{report.picture}</td><td>{report.armed ? "existing" : "empty"}</td><td>{report.operations}</td><td>{report.fees}</td>
            <td>{[report.fault, ...report.class_violations, ...report.station_refusals].filter(Boolean).join("; ") || "passed"}
              {#if report.intents.length}<details><summary>Intents</summary><ul>{#each report.intents as intent, j (j)}<li>{intent}</li>{/each}</ul></details>{/if}</td></tr>{/each}</tbody>
        </table></div>{/if}
        <section aria-label="Profile card"><h3>Card and profiles</h3>
          <label>Profile<select bind:value={minds.profile} disabled={minds.busy}>{#each minds.profiles as p (p)}<option>{p}</option>{/each}</select></label>
          {#if card}<MindCard {card} readonly={minds.readonly} onedit={setValue} />{:else}<p>The card definition needs a valid schema and complete profile values.</p>{/if}
          <form class="toolbar" onsubmit={event => { event.preventDefault(); void minds.edit({ kind: "profile_add", name: profileName, from: minds.profile || null }); }}>
            <label>Profile name<input bind:value={profileName} disabled={minds.readonly} maxlength="64" required /></label>
            <button disabled={minds.readonly || minds.profiles.length >= 4}>Copy profile</button>
            <button type="button" disabled={minds.readonly || !profileName || !minds.profile}
              onclick={() => void minds.edit({ kind: "profile_rename", old: minds.profile, new: profileName })}>Rename current</button>
            <button type="button" disabled={minds.readonly || minds.profiles.length < 2}
              onclick={() => void minds.edit({ kind: "profile_delete", name: minds.profile })}>Remove current</button>
          </form>
          <form class="toolbar" onsubmit={event => { event.preventDefault(); void minds.edit({ kind: "preset_save", profile: minds.profile, preset }); }}>
            <label>Preset<input bind:value={preset} list="mind-presets" disabled={minds.readonly} maxlength="64" required /></label>
            <datalist id="mind-presets">{#each presets as p (p)}<option value={p}></option>{/each}</datalist>
            <button disabled={minds.readonly}>Save preset</button>
            <button type="button" disabled={minds.readonly || !preset} onclick={() => void minds.edit({ kind: "preset_apply", profile: minds.profile, preset })}>Apply preset</button>
            <button type="button" disabled={minds.readonly || !preset} onclick={() => void minds.edit({ kind: "preset_delete", preset })}>Remove preset</button>
          </form>
        </section>
        <section aria-label="Ordered rules"><h3>Ordered rules</h3>
          <ol>{#each minds.rulesText[minds.profile] ?? [] as text, i (i)}<li>{text}</li>{/each}</ol>
          <form onsubmit={event => { event.preventDefault(); void minds.edit({ kind: "rule", profile: minds.profile, slot, text: rule }); }}>
            <label>Slot<input type="number" min="1" max="6" step="1" bind:value={slot} disabled={minds.readonly} /></label>
            <label>Rule<input bind:value={rule} maxlength="2048" disabled={minds.readonly} placeholder="Empty removes this slot" /></label>
            {#if unsupportedRule}<p>This rule uses terms outside the text editor's grammar. Its exact definition is preserved. Enter a new rule to replace it, or edit its JSON below.</p>{/if}
            <button disabled={minds.readonly || (unsupportedRule && !rule.trim())}>Apply rule</button>
          </form>
        </section>
        <details><summary>Card definition</summary>
          <p>Exact integer schema, presets, profiles, rules, class and ABI. Check validates this definition with the source.</p>
          <form onsubmit={event => { event.preventDefault(); minds.setEnvelope(definition); }}>
            <label>Definition JSON<textarea bind:value={definition} rows="15" disabled={minds.readonly}></textarea></label>
            <button disabled={minds.readonly}>Use definition</button>
          </form>
        </details>
        <form class="save" onsubmit={event => { event.preventDefault(); void minds.save(notes, version.trim() || null); }}>
          <label>Revision notes<textarea bind:value={notes} rows="2" disabled={minds.readonly}></textarea></label>
          <label>Version label (optional)<input bind:value={version} maxlength="64" disabled={minds.readonly} /></label>
          <button disabled={minds.readonly}>Check and save revision</button>
        </form>
        <details><summary>Manage library entry</summary>
          <p>Names and version labels can change. Existing seals and retained revisions keep their exact identity.</p>
          <form onsubmit={event => { event.preventDefault(); void minds.lifecycle({ kind: "rename", new_name: newName }); }}>
            <label>New mind name<input bind:value={newName} maxlength="64" required disabled={minds.readonly || minds.dirty} /></label>
            <button disabled={minds.readonly || minds.dirty}>Rename mind</button>
          </form>
          <form onsubmit={event => { event.preventDefault(); void minds.lifecycle({ kind: "version_rename", old: oldVersion, new: newVersion }); }}>
            <label>Existing version label<input bind:value={oldVersion} maxlength="64" required disabled={minds.readonly || minds.dirty} /></label>
            <label>New version label<input bind:value={newVersion} maxlength="64" required disabled={minds.readonly || minds.dirty} /></label>
            <button disabled={minds.readonly || minds.dirty}>Rename version label</button>
            <button type="button" disabled={minds.readonly || minds.dirty || !oldVersion.trim()} onclick={() => void minds.lifecycle({ kind: "version_forget", version: oldVersion })}>Forget version label</button>
          </form>
          <p>Removing this entry discards its working copy and frees a library slot. Revisions still needed by seals, named versions or kept records are retained.</p>
          <label><input type="checkbox" bind:checked={removeConfirmed} disabled={minds.readonly || minds.dirty} /> Remove {minds.name} from my library</label>
          <button type="button" disabled={!removeConfirmed || minds.readonly || minds.dirty} onclick={() => void minds.lifecycle({ kind: "remove" })}>Remove library entry</button>
        </details>
      {:else}<p>Select a mind or create a working copy from Standard.</p>{/if}
      <details><summary>Proving ground and replay bench</summary><MindBulkControls {minds} prefill={command} /></details>
    </main>
  </div>
</dialog>

<style>
  dialog { width: min(1100px, 96vw); max-height: 94vh; color: var(--fg, #ddd); background: var(--bg, #181a1d); border: 1px solid var(--border, #666); border-radius: .5rem; padding: 1rem; }
  dialog::backdrop { background: #000a; }
  header, .toolbar { display: flex; align-items: center; gap: .5rem; flex-wrap: wrap; }
  header { justify-content: space-between; }
  h2, h3 { margin: .5rem 0; }
  .layout { display: grid; grid-template-columns: minmax(10rem, 15rem) minmax(0, 1fr); gap: 1rem; }
  aside, main { min-width: 0; }
  aside > button { display: block; width: 100%; text-align: left; overflow-wrap: anywhere; margin: .3rem 0; }
  button.selected { outline: 2px solid currentColor; }
  form, section, details { margin: .8rem 0; }
  label { display: grid; gap: .25rem; margin: .4rem 0; }
  input, select, textarea { max-width: 100%; min-width: 0; font: inherit; }
  textarea { width: 100%; box-sizing: border-box; font-family: monospace; }
  .hint { opacity: .8; }
  .reports { overflow-x: auto; }
  table { border-collapse: collapse; width: 100%; }
  td, th { text-align: left; vertical-align: top; padding: .35rem; border-bottom: 1px solid var(--border, #555); }
  .diagnostics, [role="alert"] { color: var(--danger, #ffa09a); overflow-wrap: anywhere; }
  .save { border-top: 1px solid var(--border, #555); padding-top: .6rem; }
  @media (max-width: 650px) { .layout { grid-template-columns: 1fr; } aside { border-bottom: 1px solid #666; padding-bottom: .6rem; } }
</style>
