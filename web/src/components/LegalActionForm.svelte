<script lang="ts">
  import type { LegalAction } from "$lib/stores/actions.svelte";
  import type { ActionRequestsStore } from "$lib/stores/action-requests.svelte";
  import type { ActionControl } from "$lib/legal-action-control";

  let { action, control, requests, submit }: {
    action: LegalAction;
    control: ActionControl;
    requests: ActionRequestsStore;
    submit: (values: Record<string, string>) => Promise<void>;
  } = $props();
  let values = $state<Record<string, string>>({});
  const attempt = $derived(requests.entries[action.offer]);
  const disabled = $derived(attempt?.busy || attempt?.blocked || false);
  function set(id: string, value: string): void { values = { ...values, [id]: value }; }
</script>

<form class="action" aria-label={action.presentation.label}
  onsubmit={event => { event.preventDefault(); void submit(values); }}>
  <strong>{action.presentation.label}</strong>
  {#if action.presentation.help}<p>{action.presentation.help}</p>{/if}
  {#each control.fields as field (field.id)}
    <label>
      {field.id.replaceAll("_", " ")}{field.required ? "" : " (optional)"}
      {#if field.kind === "boolean"}
        <select required={field.required} {disabled} value={values[field.id] ?? ""}
          onchange={event => set(field.id, event.currentTarget.value)}>
          <option value="">Choose…</option><option value="true">Yes</option><option value="false">No</option>
        </select>
      {:else if field.kind === "choice"}
        <select required={field.required} {disabled} value={values[field.id] ?? ""}
          onchange={event => set(field.id, event.currentTarget.value)}>
          <option value="">Choose…</option>
          {#each field.choices as choice (choice)}<option value={choice}>{choice}</option>{/each}
        </select>
      {:else}
        <input type="text" inputmode={field.kind === "integer" ? "numeric" : "text"}
          required={field.required && field.kind === "integer"} maxlength={field.kind === "integer" ? 20 : field.maximumBytes}
          {disabled} value={values[field.id] ?? ""} autocomplete="off"
          oninput={event => set(field.id, event.currentTarget.value)} />
      {/if}
    </label>
  {/each}
  <button type="submit" {disabled}>{attempt?.busy ? "Waiting…" : action.presentation.label}</button>
  {#if attempt}
    <p role="status">{attempt.message}</p>
    {#if attempt.reference}
      <button type="button" disabled={attempt.busy} onclick={() => void requests.status(action.offer)}>Check transaction status</button>
      <small>Transaction: {attempt.reference}</small>
    {/if}
  {/if}
</form>

<style>
  .action { display: grid; gap: 0.5rem; border: 1px solid var(--border); border-radius: 8px; padding: 0.65rem; background: var(--bg-raised); }
  label { display: grid; gap: 0.2rem; font-size: 0.85rem; }
  input, select, button { font: inherit; color: var(--fg); background: var(--bg); border: 1px solid var(--border); border-radius: 4px; padding: 0.4rem; min-width: 0; }
  button { cursor: pointer; } button:disabled { cursor: default; opacity: 0.6; }
  p { margin: 0; color: var(--fg-muted); font-size: 0.85rem; }
  small { overflow-wrap: anywhere; color: var(--fg-muted); }
</style>
