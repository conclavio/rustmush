<script lang="ts">
  import type { ActionRequestsStore } from "$lib/stores/action-requests.svelte";
  let { requests }: { requests: ActionRequestsStore } = $props();
  const attempts = $derived(Object.entries(requests.entries));
  const unresolved = $derived(attempts.some(([, value]) => value.blocked && value.response?.state !== "committed"));
</script>

{#if attempts.length > 0}
  <section aria-label="Recent transactions">
    <strong>Recent transactions</strong>
    {#if unresolved}
      <p role="status">Some transactions are unresolved. Check their status before repeating those actions, even if the available offers change.</p>
    {/if}
    <ul>
      {#each attempts as [offer, attempt] (offer)}
        <li>
          <strong>{attempt.label}</strong>
          <p>{attempt.message}</p>
          {#if attempt.reference}
            <button type="button" disabled={attempt.busy} onclick={() => void requests.status(offer)}>Check transaction status</button>
            <small>Transaction: {attempt.reference}</small>
          {/if}
        </li>
      {/each}
    </ul>
  </section>
{/if}

<style>
  section { border: 1px solid var(--border); border-radius: 8px; padding: 0.65rem; margin-bottom: 0.7rem; }
  ul { display: grid; gap: 0.6rem; list-style: none; padding: 0; margin: 0.5rem 0 0; }
  li { display: grid; gap: 0.25rem; }
  p, small { margin: 0; font-size: 0.85rem; color: var(--fg-muted); overflow-wrap: anywhere; }
  button { font: inherit; color: var(--fg); background: var(--bg); border: 1px solid var(--border); border-radius: 4px; padding: 0.4rem; cursor: pointer; }
  button:disabled { cursor: default; opacity: 0.6; }
</style>
