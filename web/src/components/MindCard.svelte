<script lang="ts">
  import { cardValueText, type MindCard, type CardValue } from "$lib/mind-card";
  let { card, readonly = true, stations = ["helm", "sensors", "fire"], onedit }:
    { card: MindCard; readonly?: boolean; stations?: string[]; onedit?: (field: string, value: CardValue) => void } = $props();
  function setMember(field: string, member: string, checked: boolean) {
    const old = card.values[field];
    const next = new Set(Array.isArray(old) ? old : []);
    if (checked) next.add(member); else next.delete(member);
    onedit?.(field, [...next].sort());
  }
</script>

<div class="mind-card">
  {#each card.sections as section (section.id)}
    <fieldset disabled={readonly || !stations.includes(section.station)}>
      <legend>{section.label}{!stations.includes(section.station) ? " (no station; inert)" : ""}</legend>
      {#each section.fields as field (field.id)}
        {@const value = card.values[field.id]!}
        {@const widget = field.widget}
        <div class="field">
          {#if readonly}
            <span>{field.label}</span><output>{cardValueText(widget, value)}</output>
            {#if widget.kind === "slider" && widget.min < widget.max}<meter aria-label={field.label} min={widget.min} max={widget.max} value={Number(value)}></meter>{/if}
          {:else if widget.kind === "slider"}
            <label>{field.label}<input type="range" min={widget.min} max={widget.max} step={widget.step}
              value={Number(value)} onchange={event => onedit?.(field.id, Number(event.currentTarget.value))} /></label>
            <output>{cardValueText(widget, value)}</output>
          {:else if widget.kind === "choice"}
            <label>{field.label}<select value={String(value)} onchange={event => onedit?.(field.id, event.currentTarget.value)}>
              {#each widget.choices as choice (choice)}<option value={choice}>{choice}</option>{/each}
            </select></label>
          {:else if widget.kind === "toggle"}
            <label><input type="checkbox" checked={value === true} onchange={event => onedit?.(field.id, event.currentTarget.checked)} />{field.label}</label>
          {:else if widget.kind === "bearing"}
            <label>{field.label}<input type="number" min="0" max="359" step="1" value={Number(value)}
              onchange={event => onedit?.(field.id, Number(event.currentTarget.value))} />°</label>
          {:else}
            <fieldset class="set"><legend>{field.label}</legend>
              {#each widget.kind === "facet_set" ? ["front", "left", "rear", "right", "top"] : ["friendly", "hostile", "unknown"] as member (member)}
                <label><input type="checkbox" checked={Array.isArray(value) && value.includes(member)}
                  onchange={event => setMember(field.id, member, event.currentTarget.checked)} />{member}</label>
              {/each}
            </fieldset>
          {/if}
          {#if widget.help}<small>{widget.help}</small>{/if}
        </div>
      {/each}
    </fieldset>
  {/each}
</div>

<style>
  .mind-card { display: grid; gap: .7rem; }
  fieldset { border: 1px solid var(--border, #555); padding: .6rem; min-width: 0; }
  legend { padding: 0 .25rem; }
  .field { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: .35rem; padding: .3rem 0; }
  label { display: flex; align-items: center; gap: .5rem; flex-wrap: wrap; }
  output { font-variant-numeric: tabular-nums; }
  small { flex-basis: 100%; opacity: .8; overflow-wrap: anywhere; }
  input[type="range"] { max-width: 100%; }
  .set { display: flex; flex-wrap: wrap; gap: .5rem; width: 100%; }
</style>
