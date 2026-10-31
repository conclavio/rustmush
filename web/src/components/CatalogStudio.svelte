<script lang="ts">
  // Cut CS.3 — Components, Tools, Hardware, Goods & Resources Studio (doc 34 §5.1):
  // Visual cards & structured inspectors for weapons, ammunition, sensors, cores,
  // governed hardware, resolution tables, mining tools, market/goods.json (with
  // Dray 100 t / 120 m³ and Dray-P 86 t / 103.2 m³ stowage calculator), and
  // resources.json.
  import type { JsonValue } from "$lib/proto";
  import {
    computeHoldStowage,
    computeWeaponBallistics,
    detectComponentDocShape,
    updateGoodRow,
    updateRulesPackEntryDef,
  } from "$lib/content/catalog-editor";

  let {
    documentJson,
    onChange,
  }: {
    documentJson: JsonValue | null;
    onChange: (nextDoc: JsonValue) => void;
  } = $props();

  let selectedId = $state<string>("");

  const shape = $derived(detectComponentDocShape(documentJson));

  const rulesEntries = $derived.by(() => {
    if (shape !== "rules_pack" || !documentJson || typeof documentJson !== "object") {
      return [];
    }
    const entries =
      ((documentJson as Record<string, unknown>).entries as Record<
        string,
        Record<string, unknown>
      >) ?? {};
    return Object.entries(entries).map(([id, raw]) => ({
      id,
      kind: String(raw.kind ?? "entry"),
      def: (raw.def ?? {}) as Record<string, unknown>,
    }));
  });

  const activeRuleEntry = $derived(
    rulesEntries.find((e) => e.id === selectedId) ?? rulesEntries[0] ?? null,
  );

  const goodsRows = $derived.by(() => {
    if (shape !== "goods_table" || !documentJson || typeof documentJson !== "object") {
      return [];
    }
    const list = (documentJson as Record<string, unknown>).goods;
    return Array.isArray(list) ? (list as Array<Record<string, unknown>>) : [];
  });

  const activeGood = $derived(
    goodsRows.find((g) => String(g.id) === selectedId) ?? goodsRows[0] ?? null,
  );

  const resourceRows = $derived.by(() => {
    if (
      shape !== "resource_table" ||
      !documentJson ||
      typeof documentJson !== "object"
    ) {
      return [];
    }
    const list = (documentJson as Record<string, unknown>).resources;
    return Array.isArray(list) ? (list as Array<Record<string, unknown>>) : [];
  });

  function patchRuleDef(entryId: string, patch: Record<string, unknown>): void {
    if (!documentJson || !activeRuleEntry) return;
    onChange(
      updateRulesPackEntryDef(documentJson, entryId, {
        ...activeRuleEntry.def,
        ...patch,
      } as JsonValue),
    );
  }
</script>

<div class="catalog-studio">
  {#if shape === "rules_pack"}
    <div class="catalog-workspace">
      <div class="card-grid">
        {#each rulesEntries as entry (entry.id)}
          <button
            type="button"
            class="item-card"
            class:selected={activeRuleEntry?.id === entry.id}
            onclick={() => (selectedId = entry.id)}
          >
            <div class="card-top">
              <strong>{String(entry.def.label ?? entry.id)}</strong>
              <span class="kind-pill">{entry.kind}</span>
            </div>
            <div class="card-sub">
              <code>{entry.id}</code>
            </div>
          </button>
        {/each}
      </div>

      <div class="inspector-pane">
        {#if activeRuleEntry}
          {@const def = activeRuleEntry.def}
          <h4>{String(def.label ?? activeRuleEntry.id)} (<code>{activeRuleEntry.id}</code>)</h4>

          {#if activeRuleEntry.kind === "weapon"}
            {@const ballistics = computeWeaponBallistics(def as JsonValue)}
            <div class="field-grid">
              <label class="field-label">
                <span>Label</span>
                <input
                  type="text"
                  value={String(def.label ?? "")}
                  oninput={(e) =>
                    patchRuleDef(activeRuleEntry.id, {
                      label: (e.currentTarget as HTMLInputElement).value,
                    })}
                />
              </label>
              <label class="field-label">
                <span>Delivery (`direct` | `beam`)</span>
                <select
                  value={String(def.delivery ?? "direct")}
                  onchange={(e) =>
                    patchRuleDef(activeRuleEntry.id, {
                      delivery: (e.currentTarget as HTMLSelectElement).value,
                    })}
                >
                  <option value="direct">direct (ballistic/missile)</option>
                  <option value="beam">beam (instantaneous)</option>
                </select>
              </label>
              <label class="field-label">
                <span>Cycle ms (≥ 1000 ms floor)</span>
                <input
                  type="number"
                  min="1000"
                  step="500"
                  value={Number(def.cycle_ms ?? 6000)}
                  oninput={(e) =>
                    patchRuleDef(activeRuleEntry.id, {
                      cycle_ms: Math.max(
                        1000,
                        Number((e.currentTarget as HTMLInputElement).value) || 1000,
                      ),
                    })}
                />
              </label>
              <label class="field-label">
                <span>Burst Rounds</span>
                <input
                  type="number"
                  min="1"
                  value={Number(def.burst_rounds ?? 1)}
                  oninput={(e) =>
                    patchRuleDef(activeRuleEntry.id, {
                      burst_rounds: Math.max(
                        1,
                        Number((e.currentTarget as HTMLInputElement).value) || 1,
                      ),
                    })}
                />
              </label>
              <label class="field-label">
                <span>Heat per Shot</span>
                <input
                  type="number"
                  min="0"
                  value={Number(def.heat_per_shot ?? 0)}
                  oninput={(e) =>
                    patchRuleDef(activeRuleEntry.id, {
                      heat_per_shot: Number((e.currentTarget as HTMLInputElement).value) || 0,
                    })}
                />
              </label>
              <label class="field-label">
                <span>Ammunition ID</span>
                <input
                  type="text"
                  value={String(def.ammunition ?? "")}
                  oninput={(e) =>
                    patchRuleDef(activeRuleEntry.id, {
                      ammunition: (e.currentTarget as HTMLInputElement).value,
                    })}
                />
              </label>
            </div>

            <div class="calc-box">
              <strong>
                Ballistics &amp; Dispersion Preview: Max Range {ballistics.maxRangeM} m · Rate of Fire {ballistics.roundsPerMinute} rds/min
              </strong>
              <div class="band-list">
                {#each ballistics.bands as b (b.bandIndex)}
                  <div class="band-pill">
                    Band #{b.bandIndex}: ≤ {b.maxRangeM} m · {b.dispersionMrad} mrad (1σ = {b.dispersionRadiusM} m) · Flight {b.flightTimeS} s
                  </div>
                {/each}
              </div>
            </div>
          {:else if activeRuleEntry.kind === "ammunition"}
            <div class="field-grid">
              <label class="field-label">
                <span>Label</span>
                <input
                  type="text"
                  value={String(def.label ?? "")}
                  oninput={(e) =>
                    patchRuleDef(activeRuleEntry.id, {
                      label: (e.currentTarget as HTMLInputElement).value,
                    })}
                />
              </label>
              <label class="field-label">
                <span>Damage per Hit</span>
                <input
                  type="number"
                  min="1"
                  value={Number(def.damage ?? 20)}
                  oninput={(e) =>
                    patchRuleDef(activeRuleEntry.id, {
                      damage: Number((e.currentTarget as HTMLInputElement).value) || 1,
                    })}
                />
              </label>
              <label class="field-label">
                <span>Penetration Class (`he` | `ap` | `beam`)</span>
                <select
                  value={String(def.penetration_class ?? "he")}
                  onchange={(e) =>
                    patchRuleDef(activeRuleEntry.id, {
                      penetration_class: (e.currentTarget as HTMLSelectElement).value,
                    })}
                >
                  <option value="he">he</option>
                  <option value="ap">ap</option>
                  <option value="beam">beam</option>
                </select>
              </label>
              <label class="field-label">
                <span>Count per Load</span>
                <input
                  type="number"
                  min="1"
                  value={Number(def.count_per_load ?? 40)}
                  oninput={(e) =>
                    patchRuleDef(activeRuleEntry.id, {
                      count_per_load:
                        Number((e.currentTarget as HTMLInputElement).value) || 1,
                    })}
                />
              </label>
            </div>
          {:else if activeRuleEntry.kind === "sensor"}
            <div class="field-grid">
              <label class="field-label">
                <span>Label</span>
                <input
                  type="text"
                  value={String(def.label ?? "")}
                  oninput={(e) =>
                    patchRuleDef(activeRuleEntry.id, {
                      label: (e.currentTarget as HTMLInputElement).value,
                    })}
                />
              </label>
              <label class="field-label">
                <span>Max Range Cells</span>
                <input
                  type="number"
                  min="1"
                  value={Number(def.max_range_cells ?? 60)}
                  oninput={(e) =>
                    patchRuleDef(activeRuleEntry.id, {
                      max_range_cells:
                        Number((e.currentTarget as HTMLInputElement).value) || 1,
                    })}
                />
              </label>
              <label class="field-label">
                <span>Max Attenuation</span>
                <input
                  type="number"
                  min="0"
                  value={Number(def.max_attenuation ?? 40)}
                  oninput={(e) =>
                    patchRuleDef(activeRuleEntry.id, {
                      max_attenuation:
                        Number((e.currentTarget as HTMLInputElement).value) || 0,
                    })}
                />
              </label>
              <label class="field-label">
                <span>Classify After (ms)</span>
                <input
                  type="number"
                  min="0"
                  value={Number(def.classify_after_ms ?? 3000)}
                  oninput={(e) =>
                    patchRuleDef(activeRuleEntry.id, {
                      classify_after_ms:
                        Number((e.currentTarget as HTMLInputElement).value) || 0,
                    })}
                />
              </label>
            </div>
          {:else if activeRuleEntry.kind === "core"}
            <div class="field-grid">
              <label class="field-label">
                <span>Label</span>
                <input
                  type="text"
                  value={String(def.label ?? "")}
                  oninput={(e) =>
                    patchRuleDef(activeRuleEntry.id, {
                      label: (e.currentTarget as HTMLInputElement).value,
                    })}
                />
              </label>
              <label class="field-label">
                <span>Core Class (`I` | `II` | `III`)</span>
                <select
                  value={String(def.class ?? "I")}
                  onchange={(e) =>
                    patchRuleDef(activeRuleEntry.id, {
                      class: (e.currentTarget as HTMLSelectElement).value,
                    })}
                >
                  <option value="I">Class I (Doctrine — 1,500 ops / 1.0s)</option>
                  <option value="II">Class II (Line — 3,000 ops / 0.5s)</option>
                  <option value="III">Class III (Command — 4,500 ops / 0.25s)</option>
                </select>
              </label>
              <label class="checkbox-field">
                <input
                  type="checkbox"
                  checked={Boolean(def.governed)}
                  onchange={(e) =>
                    patchRuleDef(activeRuleEntry.id, {
                      governed: (e.currentTarget as HTMLInputElement).checked,
                    })}
                />
                <span>Governed Core (`governed: true`)</span>
              </label>
            </div>
          {/if}

          {#if typeof def.note === "string"}
            <label class="field-label">
              <span>Designer Note</span>
              <textarea
                rows="3"
                value={String(def.note)}
                oninput={(e) =>
                  patchRuleDef(activeRuleEntry.id, {
                    note: (e.currentTarget as HTMLTextAreaElement).value,
                  })}
              ></textarea>
            </label>
          {/if}
        {/if}
      </div>
    </div>
  {:else if shape === "goods_table"}
    <div class="catalog-workspace">
      <div class="card-grid">
        {#each goodsRows as g (g.id)}
          <button
            type="button"
            class="item-card"
            class:selected={activeGood?.id === g.id}
            onclick={() => (selectedId = String(g.id))}
          >
            <div class="card-top">
              <strong>{String(g.label)}</strong>
              <span class="kind-pill">{String(g.stowage)}</span>
            </div>
            <div class="card-sub">
              <code>{String(g.id)}</code> · {String(g.reference_major)} cr/{String(g.quote_unit)}
            </div>
          </button>
        {/each}
      </div>

      <div class="inspector-pane">
        {#if activeGood}
          {@const measures = (activeGood.measures ?? {}) as Record<string, number>}
          {@const massKg = Number(measures["si.mass_kg"] ?? 1000)}
          {@const volL = Number(measures["si.volume_l"] ?? 1000)}
          {@const stowSummary = computeHoldStowage(
            massKg,
            volL,
            String(activeGood.stowage ?? "stow.bulk"),
          )}
          <h4>{String(activeGood.label)} (<code>{String(activeGood.id)}</code>)</h4>

          <div class="field-grid">
            <label class="field-label">
              <span>Label</span>
              <input
                type="text"
                value={String(activeGood.label ?? "")}
                oninput={(e) =>
                  documentJson &&
                  onChange(
                    updateGoodRow(documentJson, String(activeGood.id), {
                      label: (e.currentTarget as HTMLInputElement).value,
                    }),
                  )}
              />
            </label>

            <label class="field-label">
              <span>Reference Price Major (cr)</span>
              <input
                type="number"
                min="1"
                value={Number(activeGood.reference_major ?? 100)}
                oninput={(e) =>
                  documentJson &&
                  onChange(
                    updateGoodRow(documentJson, String(activeGood.id), {
                      reference_major:
                        Number((e.currentTarget as HTMLInputElement).value) || 1,
                    }),
                  )}
              />
            </label>

            <label class="field-label">
              <span>Unit Mass (`si.mass_kg`)</span>
              <input
                type="number"
                min="1"
                value={massKg}
                oninput={(e) =>
                  documentJson &&
                  onChange(
                    updateGoodRow(documentJson, String(activeGood.id), {
                      measures: {
                        ...measures,
                        "si.mass_kg":
                          Number((e.currentTarget as HTMLInputElement).value) || 1,
                      },
                    }),
                  )}
              />
            </label>

            <label class="field-label">
              <span>Packed Volume (`si.volume_l`)</span>
              <input
                type="number"
                min="1"
                value={volL}
                oninput={(e) =>
                  documentJson &&
                  onChange(
                    updateGoodRow(documentJson, String(activeGood.id), {
                      measures: {
                        ...measures,
                        "si.volume_l":
                          Number((e.currentTarget as HTMLInputElement).value) || 1,
                      },
                    }),
                  )}
              />
            </label>
          </div>

          <div class="calc-box">
            <strong>Dray &amp; Dray-P Stowage Density Calculator</strong>
            <div class="band-list">
              <div class="band-pill">
                Standard Dray (100 t / 120 m³, <code>stow.bulk</code>):
                <strong>{stowSummary.drayMaxUnits} {String(activeGood.ledger_unit)}</strong>
                ({stowSummary.drayLimitingFactor})
              </div>
              <div class="band-pill">
                Dray-P (86 t / 103.2 m³, <code>stow.bulk</code> + <code>stow.ore</code>):
                <strong>{stowSummary.drayPMaxUnits} {String(activeGood.ledger_unit)}</strong>
                ({stowSummary.drayPLimitingFactor})
              </div>
            </div>
          </div>
        {/if}
      </div>
    </div>
  {:else if shape === "resource_table"}
    <div class="resource-grid">
      {#each resourceRows as r (r.id)}
        <div class="item-card">
          <div class="card-top">
            <strong>{String(r.code)} — {String(r.label)}</strong>
            <span class="kind-pill">{String(r.family)}</span>
          </div>
          <div class="card-sub">
            <code>{String(r.id)}</code> · {String(r.note ?? "")}
          </div>
        </div>
      {/each}
    </div>
  {:else if shape === "tool_def" && documentJson && typeof documentJson === "object"}
    {@const tool = documentJson as Record<string, unknown>}
    <div class="inspector-pane">
      <h4>Mining Tool: {String(tool.label ?? tool.id)} (<code>{String(tool.id)}</code>)</h4>
      <p class="card-sub">{String(tool.note ?? "")}</p>
    </div>
  {/if}
</div>

<style>
  .catalog-studio {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .catalog-workspace {
    display: grid;
    grid-template-columns: 260px 1fr;
    gap: 10px;
    min-height: 360px;
  }
  .card-grid,
  .resource-grid {
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-height: 480px;
    overflow-y: auto;
  }
  .item-card {
    text-align: left;
    background: #161b22;
    border: 1px solid #30363d;
    border-radius: 6px;
    padding: 8px;
    color: #c9d1d9;
    cursor: pointer;
  }
  .item-card.selected {
    border-color: #58a6ff;
    background: #1c2433;
  }
  .card-top {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  .card-sub {
    font-size: 11px;
    color: #8b949e;
    margin-top: 3px;
  }
  .kind-pill {
    font-size: 10px;
    padding: 1px 6px;
    border-radius: 999px;
    background: #21262d;
  }
  .inspector-pane {
    background: #161b22;
    border: 1px solid #30363d;
    border-radius: 6px;
    padding: 10px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .field-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(190px, 1fr));
    gap: 8px;
  }
  .field-label {
    display: flex;
    flex-direction: column;
    gap: 3px;
    font-size: 11px;
    color: #8b949e;
  }
  .field-label input,
  .field-label select,
  .field-label textarea {
    background: #0d1117;
    color: #c9d1d9;
    border: 1px solid #30363d;
    border-radius: 4px;
    padding: 4px 6px;
    font-size: 12px;
  }
  .calc-box {
    background: #0d1117;
    border: 1px solid #30363d;
    border-radius: 4px;
    padding: 8px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .band-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .band-pill {
    font-size: 11px;
    color: #8b949e;
  }
</style>
