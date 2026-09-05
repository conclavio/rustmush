<script lang="ts">
  // The board (interface plan I6): every `.vitals` frame as widgets — the
  // tracks as grouped boxes, the pools as gauges, the readouts as a
  // key/value list with condition glyphs — with the roller's recent rolls
  // in a table beneath. Composed from the frame alone: this file names no
  // trait, no pool and no ruleset (the probe in vitalsboard.test.ts greps
  // for exactly that).
  import type { GameApp } from "$lib/game.svelte";
  import { classifyResult, youOrId } from "$lib/roll";
  import { fmtClock } from "$lib/render";
  import { orderedVitals } from "$lib/stores/vitals.svelte";
  import type { TableColumn } from "$lib/widgets";

  import Gauge from "./widgets/Gauge.svelte";
  import KeyValue from "./widgets/KeyValue.svelte";
  import Table from "./widgets/Table.svelte";
  import TrackBoxes from "./widgets/TrackBoxes.svelte";

  let { app }: { app: GameApp } = $props();

  const ROLL_COLUMNS: TableColumn[] = [
    { key: "who", label: "who" },
    { key: "result", label: "result" },
    { key: "dice", label: "dice" },
    { key: "when", label: "when", align: "right" },
  ];

  const recentRolls = $derived(
    app.rolls.entries.slice(-8).map((r) => ({
      who: youOrId(r.who, app.session.character?.id ?? null),
      result: classifyResult(r.outcome.result).label,
      dice: r.outcome.dice.map((d) => d.face).join(" "),
      when: fmtClock(r.ts),
    })),
  );
</script>

<div class="board">
  {#if app.vitals.list.length === 0}
    <p class="muted">No vitals feed for this character.</p>
  {/if}
  {#each app.vitals.list as group (group.feed)}
    {@const vitals = orderedVitals(group.frame)}
    {@const readouts = vitals.filter((v) => v.value.kind === "text")}
    <section class="frame" aria-label={group.frame.subject.name}>
      <h3>
        {group.frame.subject.name}
        <span class="rev">rev {group.frame.revision}</span>
      </h3>
      {#each vitals as v (v.id)}
        {#if v.value.kind === "track"}
          <TrackBoxes label={v.label} boxes={v.value.boxes} level={v.value.level} state={v.state} detail={v.detail} />
        {:else if v.value.kind === "bounded"}
          <Gauge
            label={v.label}
            current={v.value.current}
            min={v.value.min}
            max={v.value.max}
            state={v.state}
            detail={v.detail}
          />
        {/if}
      {/each}
      {#if readouts.length > 0}
        <KeyValue
          rows={readouts.map((v) => ({
            key: v.label,
            value: v.value.kind === "text" ? v.value.text : "",
            condition: v.state,
            detail: v.detail,
          }))}
        />
      {/if}
    </section>
  {/each}
  {#if recentRolls.length > 0}
    <Table columns={ROLL_COLUMNS} rows={recentRolls} caption="recent rolls" />
  {/if}
</div>

<style>
  .board {
    display: flex;
    flex-direction: column;
    gap: 0.8rem;
  }

  .frame {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    padding: 0.5rem 0.65rem;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-panel);
  }

  h3 {
    margin: 0 0 0.2rem;
    font-size: 0.95rem;
    display: flex;
    justify-content: space-between;
    align-items: baseline;
  }

  .rev {
    font-size: 0.7rem;
    color: var(--fg-faint);
    font-family: var(--font-mono);
  }

  .muted {
    color: var(--fg-muted);
    font-size: 0.85rem;
  }
</style>
