<script lang="ts">
  // Cut CS.2 — Quest Studio (doc 34 §4, D-34-8, D-34-9):
  // Quest cards, intra-pack & cross-pack prerequisite/choice DAG visualizer,
  // Giver NPC picker with MAX_QUESTS_PER_NPC = 8 & "quest" interface check,
  // 1-8 checkpoint builder with live room & atlas:detail:<kw> dropdowns,
  // all 8 SourceObjective forms, and Quest LLM Assist.
  import type { JsonValue } from "$lib/proto";
  import type { ContentStudioRpc } from "$lib/content/rpc";
  import type { CatalogData } from "$lib/content/store";
  import {
    applyAssistPatchToQuest,
    applyQuestModelToDoc,
    buildQuestDagEdges,
    extractQuestModels,
    OBJECTIVE_KINDS,
    type ObjectiveKind,
    QUEST_BOUNDS,
    type QuestCheckpointModel,
    type QuestEditorModel,
    utf8ByteLength,
  } from "$lib/content/quest-editor";

  let {
    packId,
    latchedVersion,
    documentJson,
    catalog,
    rpc,
    onChange,
    onStatus,
    onError,
  }: {
    packId: string;
    latchedVersion: number | null;
    documentJson: JsonValue | null;
    catalog: CatalogData;
    rpc: ContentStudioRpc | null;
    onChange: (nextDoc: JsonValue) => void;
    onStatus: (msg: string) => void;
    onError: (msg: string) => void;
  } = $props();

  let selectedEntryId = $state<string>("");
  let assistBrief = $state<string>(
    "Draft a follow-up invitation for a Stickney-to-Ceres fittings delivery with Dov Akintola",
  );
  let assistingScope = $state<string | null>(null);

  const quests = $derived<QuestEditorModel[]>(extractQuestModels(documentJson));
  const dagEdges = $derived(buildQuestDagEdges(packId, quests));
  const activeQuest = $derived<QuestEditorModel | null>(
    quests.find((q) => q.entryId === selectedEntryId) ?? quests[0] ?? null,
  );

  const giverMeta = $derived(
    activeQuest
      ? catalog.npcs.find((n) => n.entry_id === activeQuest.npc) ?? null
      : null,
  );

  function commitQuest(updated: QuestEditorModel): void {
    if (!documentJson) return;
    selectedEntryId = updated.entryId;
    onChange(applyQuestModelToDoc(documentJson, updated));
  }

  function addNewQuest(): void {
    const slug = `new_quest_${quests.length + 1}`;
    const model: QuestEditorModel = {
      entryId: `quest.${slug}`,
      npc: "npc.oduya",
      npcPack: "neighborhood.ceres_freeport",
      prerequisiteKind: "open",
      prerequisiteQuest: "",
      choiceGroup: "",
      choicePath: "",
      title: "New Optional Invitation",
      destination: "ceres",
      introduction:
        "If this piece of corridor work suits your plans, try it and bring the record back to my desk.",
      completion:
        "The captain completed the work and returned to my desk to account for it.",
      checkpoints: [
        {
          id: "step_1",
          title: "Complete the first step",
          roomPack: "neighborhood.ceres_freeport",
          roomEntry: "room.c05_registry",
          detail: "hull registry board",
          instructions:
            "Visit the Registry & Co-op Desk and use +quest/check to verify.",
          objective: {
            kind: "coop_enrolment",
            origin: "stickney",
            destination: "ceres",
            commodity: "industrial_parts",
            quantity: 1,
            npcPack: "neighborhood.ceres_freeport",
            npc: "npc.akintola",
            mode: "scan",
          },
        },
      ],
    };
    commitQuest(model);
  }

  function addCheckpoint(quest: QuestEditorModel): void {
    if (quest.checkpoints.length >= QUEST_BOUNDS.maxCheckpoints) return;
    const idx = quest.checkpoints.length + 1;
    const cp: QuestCheckpointModel = {
      id: `step_${idx}`,
      title: `Step ${idx}`,
      roomPack: "neighborhood.ceres_freeport",
      roomEntry: "room.c05_registry",
      detail: "",
      instructions: "Complete this step and run +quest/check here.",
      objective: {
        kind: "coop_enrolment",
        origin: "stickney",
        destination: "ceres",
        commodity: "industrial_parts",
        quantity: 1,
        npcPack: "neighborhood.ceres_freeport",
        npc: "npc.akintola",
        mode: "scan",
      },
    };
    commitQuest({
      ...quest,
      checkpoints: [...quest.checkpoints, cp],
    });
  }

  function updateCheckpoint(
    quest: QuestEditorModel,
    index: number,
    nextCp: QuestCheckpointModel,
  ): void {
    const next = [...quest.checkpoints];
    next[index] = nextCp;
    commitQuest({ ...quest, checkpoints: next });
  }

  function removeCheckpoint(quest: QuestEditorModel, index: number): void {
    if (quest.checkpoints.length <= 1) return;
    commitQuest({
      ...quest,
      checkpoints: quest.checkpoints.filter((_, i) => i !== index),
    });
  }

  async function runQuestAssist(scope: string): Promise<void> {
    if (!rpc || !activeQuest) return;
    assistingScope = scope;
    const res = await rpc.request({
      op: "assist",
      family: "quest",
      scope,
      brief: assistBrief,
      context: activeQuest as unknown as JsonValue,
    });
    assistingScope = null;
    if (res.status === "error") {
      onError(res.message);
      return;
    }
    if (res.status === "assisted") {
      onStatus(res.summary);
      commitQuest(applyAssistPatchToQuest(activeQuest, scope, res.patch));
    }
  }
</script>

<div class="quest-studio">
  <!-- Quest LLM Co-Authoring Bar (D-34-8) -->
  <div class="assist-bar">
    <input
      type="text"
      class="assist-input"
      placeholder="Describe a quest invitation in the giver NPC's voice…"
      bind:value={assistBrief}
    />
    <button
      type="button"
      class="primary"
      disabled={!activeQuest || assistingScope !== null}
      onclick={() => void runQuestAssist("full")}
    >
      {assistingScope === "full" ? "Drafting…" : "LLM Draft into Active Quest"}
    </button>
    <button type="button" onclick={addNewQuest}>+ New Quest</button>
  </div>

  <!-- Prerequisite & Choice DAG Banner -->
  <div class="dag-box">
    <div class="dag-head">
      <strong>Quest Chain &amp; Choice DAG ({quests.length} quests in <code>{packId}</code>)</strong>
    </div>
    <div class="dag-nodes">
      {#each quests as q (q.entryId)}
        <div class="dag-pill">
          <code>{q.entryId}</code>
          <span>({q.npc})</span>
          {#if q.prerequisiteKind === "completed"}
            <span class="edge-tag">← {q.prerequisiteQuest}</span>
          {:else}
            <span class="open-tag">open</span>
          {/if}
          {#if q.choiceGroup}
            <span class="choice-tag">choice: {q.choiceGroup}:{q.choicePath}</span>
          {/if}
        </div>
      {/each}
      {#each dagEdges.filter((e) => e.crossPack) as edge, i (i)}
        <span class="cross-pack-badge">
          Cross-pack pin: <code>{edge.from}</code> → <code>{edge.to}</code>
        </span>
      {/each}
    </div>
  </div>

  <div class="quest-workspace">
    <!-- Left Quest Card List -->
    <div class="card-grid">
      {#each quests as q (q.entryId)}
        <button
          type="button"
          class="quest-card"
          class:selected={activeQuest?.entryId === q.entryId}
          onclick={() => (selectedEntryId = q.entryId)}
        >
          <div class="card-top">
            <strong>{q.title}</strong>
            <span class="step-badge">{q.checkpoints.length} steps</span>
          </div>
          <div class="card-sub">
            <code>{q.entryId}</code> · Giver: <code>{q.npc}</code>
          </div>
        </button>
      {/each}
    </div>

    <!-- Right Quest Inspector -->
    <div class="inspector-pane">
      {#if activeQuest}
        <div class="inspector-head">
          <h4>{activeQuest.title} (<code>{activeQuest.entryId}</code>)</h4>
          {#if giverMeta && !giverMeta.interfaces.includes("quest")}
            <div class="guardrail-alert">
              <strong>Interface Warning (D-34-9):</strong> Giver
              <code>{giverMeta.name}</code> ({giverMeta.entry_id}) does not declare
              <code>"quest"</code> in <code>profile.interfaces</code>.
            </div>
          {/if}
          {#if giverMeta && giverMeta.quest_count >= QUEST_BOUNDS.maxQuestsPerNpc}
            <div class="guardrail-alert">
              <strong>Capacity Warning:</strong> Giver <code>{giverMeta.name}</code> is at
              <code>{giverMeta.quest_count}/{QUEST_BOUNDS.maxQuestsPerNpc}</code> quests (`MAX_QUESTS_PER_NPC`).
            </div>
          {/if}
        </div>

        <div class="field-grid">
          <label class="field-label">
            <span>
              Giver NPC {latchedVersion !== null ? "(Locked on latched pack — D-34-9)" : ""}
            </span>
            <select
              value={activeQuest.npc}
              disabled={latchedVersion !== null}
              onchange={(e) => {
                const nextNpcId = (e.currentTarget as HTMLSelectElement).value;
                const found = catalog.npcs.find((n) => n.entry_id === nextNpcId);
                const nextPack = found
                  ? found.pack_id.startsWith("tacsim.cast.")
                    ? found.room_pack
                    : found.pack_id
                  : activeQuest.npcPack;
                commitQuest({
                  ...activeQuest,
                  npc: nextNpcId,
                  npcPack: nextPack,
                });
              }}
            >
              {#each catalog.npcs as n (`${n.pack_id}:${n.entry_id}`)}
                <option value={n.entry_id}>
                  {n.name} ({n.entry_id} · {n.quest_count}/{QUEST_BOUNDS.maxQuestsPerNpc} quests{n.interfaces.includes("quest") ? " · +quest" : ""})
                </option>
              {/each}
            </select>
          </label>

          <label class="field-label">
            <span>Title ({utf8ByteLength(activeQuest.title)} / {QUEST_BOUNDS.maxTitleBytes} B)</span>
            <input
              type="text"
              value={activeQuest.title}
              oninput={(e) =>
                commitQuest({
                  ...activeQuest,
                  title: (e.currentTarget as HTMLInputElement).value,
                })}
            />
          </label>

          <label class="field-label">
            <span>
              Destination ({utf8ByteLength(activeQuest.destination)} / {QUEST_BOUNDS.maxDestinationBytes} B)
            </span>
            <input
              type="text"
              value={activeQuest.destination}
              oninput={(e) =>
                commitQuest({
                  ...activeQuest,
                  destination: (e.currentTarget as HTMLInputElement).value,
                })}
            />
          </label>

          <label class="field-label">
            <span>Prerequisite Mode</span>
            <select
              value={activeQuest.prerequisiteKind}
              onchange={(e) =>
                commitQuest({
                  ...activeQuest,
                  prerequisiteKind: (e.currentTarget as HTMLSelectElement).value as
                    | "open"
                    | "completed",
                })}
            >
              <option value="open">open (always available)</option>
              <option value="completed">completed quest prerequisite</option>
            </select>
          </label>

          {#if activeQuest.prerequisiteKind === "completed"}
            <label class="field-label">
              <span>Required Quest Reference (`&lt;pack&gt;/quest.&lt;slug&gt;`)</span>
              <input
                type="text"
                placeholder="tacsim.quests.first_steps/quest.qureshi_first_readings"
                value={activeQuest.prerequisiteQuest}
                oninput={(e) =>
                  commitQuest({
                    ...activeQuest,
                    prerequisiteQuest: (e.currentTarget as HTMLInputElement).value,
                  })}
              />
            </label>
          {/if}

          <label class="field-label">
            <span>Mutually Exclusive Choice Group (optional)</span>
            <input
              type="text"
              placeholder="tacsim.leila_next_work"
              value={activeQuest.choiceGroup}
              oninput={(e) =>
                commitQuest({
                  ...activeQuest,
                  choiceGroup: (e.currentTarget as HTMLInputElement).value,
                })}
            />
          </label>

          <label class="field-label">
            <span>Choice Path Slug (optional)</span>
            <input
              type="text"
              placeholder="trade"
              value={activeQuest.choicePath}
              oninput={(e) =>
                commitQuest({
                  ...activeQuest,
                  choicePath: (e.currentTarget as HTMLInputElement).value,
                })}
            />
          </label>
        </div>

        <!-- Introduction & Completion Prose -->
        <div class="prose-section">
          <div class="section-head">
            <span>
              Introduction ({utf8ByteLength(activeQuest.introduction)} / {QUEST_BOUNDS.maxIntroductionBytes} B)
            </span>
            <button
              type="button"
              class="small-btn"
              onclick={() => void runQuestAssist("introduction")}
            >
              Regenerate Introduction
            </button>
          </div>
          <textarea
            rows="3"
            value={activeQuest.introduction}
            oninput={(e) =>
              commitQuest({
                ...activeQuest,
                introduction: (e.currentTarget as HTMLTextAreaElement).value,
              })}
          ></textarea>

          <div class="section-head">
            <span>
              Completion Memory ({utf8ByteLength(activeQuest.completion)} / {QUEST_BOUNDS.maxCompletionBytes} B)
            </span>
            <button
              type="button"
              class="small-btn"
              onclick={() => void runQuestAssist("completion")}
            >
              Regenerate Completion
            </button>
          </div>
          <textarea
            rows="2"
            value={activeQuest.completion}
            oninput={(e) =>
              commitQuest({
                ...activeQuest,
                completion: (e.currentTarget as HTMLTextAreaElement).value,
              })}
          ></textarea>
        </div>

        <!-- Ordered Checkpoints (1-8) & All 8 SourceObjective Forms -->
        <div class="checkpoints-box">
          <div class="section-head">
            <strong>
              Checkpoints ({activeQuest.checkpoints.length} / {QUEST_BOUNDS.maxCheckpoints})
            </strong>
            <button
              type="button"
              disabled={activeQuest.checkpoints.length >= QUEST_BOUNDS.maxCheckpoints}
              onclick={() => addCheckpoint(activeQuest)}
            >
              + Add Checkpoint
            </button>
          </div>

          {#each activeQuest.checkpoints as cp, idx (idx)}
            {@const station = catalog.stations.find((s) => s.pack_id === cp.roomPack)}
            {@const roomObj = station?.rooms.find((r) => r.id === cp.roomEntry)}
            <div class="cp-card">
              <div class="section-head">
                <strong>#{idx + 1}: <code>{cp.id}</code> — {cp.title}</strong>
                {#if activeQuest.checkpoints.length > 1}
                  <button
                    type="button"
                    class="small-btn"
                    onclick={() => removeCheckpoint(activeQuest, idx)}
                  >
                    Remove
                  </button>
                {/if}
              </div>

              <div class="field-grid">
                <label class="field-label">
                  <span>Step ID (≤ {QUEST_BOUNDS.maxCheckpointIdBytes} B)</span>
                  <input
                    type="text"
                    value={cp.id}
                    oninput={(e) =>
                      updateCheckpoint(activeQuest, idx, {
                        ...cp,
                        id: (e.currentTarget as HTMLInputElement).value,
                      })}
                  />
                </label>

                <label class="field-label">
                  <span>Step Title (≤ {QUEST_BOUNDS.maxCheckpointTitleBytes} B)</span>
                  <input
                    type="text"
                    value={cp.title}
                    oninput={(e) =>
                      updateCheckpoint(activeQuest, idx, {
                        ...cp,
                        title: (e.currentTarget as HTMLInputElement).value,
                      })}
                  />
                </label>

                <label class="field-label">
                  <span>Station Pack (`room_pack`)</span>
                  <select
                    value={cp.roomPack}
                    onchange={(e) => {
                      const nextPack = (e.currentTarget as HTMLSelectElement).value;
                      const st = catalog.stations.find((s) => s.pack_id === nextPack);
                      updateCheckpoint(activeQuest, idx, {
                        ...cp,
                        roomPack: nextPack,
                        roomEntry: st?.rooms[0]?.id ?? cp.roomEntry,
                      });
                    }}
                  >
                    {#each catalog.stations as st (st.pack_id)}
                      <option value={st.pack_id}>{st.pack_id}</option>
                    {/each}
                  </select>
                </label>

                <label class="field-label">
                  <span>Room (`room_entry`)</span>
                  <select
                    value={cp.roomEntry}
                    onchange={(e) =>
                      updateCheckpoint(activeQuest, idx, {
                        ...cp,
                        roomEntry: (e.currentTarget as HTMLSelectElement).value,
                      })}
                  >
                    {#if !station || station.rooms.length === 0}
                      <option value={cp.roomEntry}>{cp.roomEntry}</option>
                    {:else}
                      {#each station.rooms as r (r.id)}
                        <option value={r.id}>{r.id} — {r.name}</option>
                      {/each}
                    {/if}
                  </select>
                </label>

                <label class="field-label">
                  <span>Objective Kind (8 SourceObjective variants)</span>
                  <select
                    value={cp.objective.kind}
                    onchange={(e) =>
                      updateCheckpoint(activeQuest, idx, {
                        ...cp,
                        objective: {
                          ...cp.objective,
                          kind: (e.currentTarget as HTMLSelectElement)
                            .value as ObjectiveKind,
                        },
                      })}
                  >
                    {#each OBJECTIVE_KINDS as kind (kind)}
                      <option value={kind}>{kind}</option>
                    {/each}
                  </select>
                </label>

                {#if cp.objective.kind === "observation"}
                  <label class="field-label">
                    <span>Room Detail (`atlas:detail:&lt;kw&gt;`)</span>
                    <select
                      value={cp.detail}
                      onchange={(e) =>
                        updateCheckpoint(activeQuest, idx, {
                          ...cp,
                          detail: (e.currentTarget as HTMLSelectElement).value,
                        })}
                    >
                      {#if !roomObj || roomObj.details.length === 0}
                        <option value={cp.detail}>{cp.detail || "(enter detail key)"}</option>
                      {:else}
                        {#each roomObj.details as det (det)}
                          <option value={det}>{det}</option>
                        {/each}
                      {/if}
                    </select>
                  </label>
                {:else if cp.objective.kind === "trade"}
                  <label class="field-label">
                    <span>Commodity (12 Shipped Goods)</span>
                    <select
                      value={cp.objective.commodity}
                      onchange={(e) =>
                        updateCheckpoint(activeQuest, idx, {
                          ...cp,
                          objective: {
                            ...cp.objective,
                            commodity: (e.currentTarget as HTMLSelectElement).value,
                          },
                        })}
                    >
                      {#each catalog.goods as g (g.id)}
                        <option value={g.id}>{g.id} ({g.label})</option>
                      {/each}
                    </select>
                  </label>
                  <label class="field-label">
                    <span>Quantity (t)</span>
                    <input
                      type="number"
                      min="1"
                      value={cp.objective.quantity}
                      oninput={(e) =>
                        updateCheckpoint(activeQuest, idx, {
                          ...cp,
                          objective: {
                            ...cp.objective,
                            quantity:
                              Number((e.currentTarget as HTMLInputElement).value) || 1,
                          },
                        })}
                    />
                  </label>
                {:else if cp.objective.kind === "survey"}
                  <label class="field-label">
                    <span>Survey Mode</span>
                    <select
                      value={cp.objective.mode}
                      onchange={(e) =>
                        updateCheckpoint(activeQuest, idx, {
                          ...cp,
                          objective: {
                            ...cp.objective,
                            mode: (e.currentTarget as HTMLSelectElement).value as
                              | "scan"
                              | "sound",
                          },
                        })}
                    >
                      <option value="scan">scan (+tv/1 scan)</option>
                      <option value="sound">sound (+tv/1 sound)</option>
                    </select>
                  </label>
                {:else if cp.objective.kind === "passenger" || cp.objective.kind === "contact" || cp.objective.kind === "paid_passage"}
                  <label class="field-label">
                    <span>Target NPC</span>
                    <select
                      value={cp.objective.npc}
                      onchange={(e) => {
                        const targetNpc = (e.currentTarget as HTMLSelectElement).value;
                        const meta = catalog.npcs.find((n) => n.entry_id === targetNpc);
                        updateCheckpoint(activeQuest, idx, {
                          ...cp,
                          objective: {
                            ...cp.objective,
                            npc: targetNpc,
                            npcPack: meta
                              ? meta.pack_id.startsWith("tacsim.cast.")
                                ? meta.room_pack
                                : meta.pack_id
                              : cp.objective.npcPack,
                          },
                        });
                      }}
                    >
                      {#each catalog.npcs as n (`${n.pack_id}:${n.entry_id}`)}
                        <option value={n.entry_id}>{n.name} ({n.entry_id})</option>
                      {/each}
                    </select>
                  </label>
                {/if}
              </div>

              <label class="field-label">
                <span>
                  Instructions ({utf8ByteLength(cp.instructions)} / {QUEST_BOUNDS.maxInstructionsBytes} B)
                </span>
                <textarea
                  rows="2"
                  value={cp.instructions}
                  oninput={(e) =>
                    updateCheckpoint(activeQuest, idx, {
                      ...cp,
                      instructions: (e.currentTarget as HTMLTextAreaElement).value,
                    })}
                ></textarea>
              </label>
            </div>
          {/each}
        </div>
      {/if}
    </div>
  </div>
</div>

<style>
  .quest-studio {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .assist-bar {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: center;
  }
  .assist-input {
    flex: 1;
    min-width: 240px;
    background: #161b22;
    color: #c9d1d9;
    border: 1px solid #30363d;
    border-radius: 4px;
    padding: 5px 8px;
    font-size: 12px;
  }
  .dag-box {
    background: #161b22;
    border: 1px solid #30363d;
    border-radius: 6px;
    padding: 8px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .dag-nodes {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .dag-pill {
    background: #0d1117;
    border: 1px solid #30363d;
    border-radius: 999px;
    padding: 2px 8px;
    font-size: 11px;
    display: flex;
    gap: 4px;
    align-items: center;
  }
  .edge-tag,
  .cross-pack-badge {
    color: #58a6ff;
    font-size: 10px;
  }
  .choice-tag {
    color: #d29922;
    font-size: 10px;
  }
  .open-tag {
    color: #3fb950;
    font-size: 10px;
  }
  .quest-workspace {
    display: grid;
    grid-template-columns: 260px 1fr;
    gap: 10px;
    min-height: 420px;
  }
  .card-grid {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .quest-card {
    text-align: left;
    background: #161b22;
    border: 1px solid #30363d;
    border-radius: 6px;
    padding: 8px;
    color: #c9d1d9;
    cursor: pointer;
  }
  .quest-card.selected {
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
  .step-badge {
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
    max-height: 520px;
    overflow-y: auto;
  }
  .field-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
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
  .prose-section textarea,
  .cp-card textarea {
    background: #0d1117;
    color: #c9d1d9;
    border: 1px solid #30363d;
    border-radius: 4px;
    padding: 4px 6px;
    font-size: 12px;
  }
  .cp-card {
    background: #0d1117;
    border: 1px solid #30363d;
    border-radius: 4px;
    padding: 8px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 6px;
  }
  .section-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: 11px;
    color: #8b949e;
  }
  .guardrail-alert {
    font-size: 11px;
    padding: 6px;
    border-radius: 4px;
    background: rgba(210, 153, 34, 0.15);
    border: 1px solid rgba(210, 153, 34, 0.4);
    color: #e3b341;
  }
  button {
    background: #21262d;
    color: #c9d1d9;
    border: 1px solid #30363d;
    border-radius: 4px;
    padding: 3px 8px;
    font-size: 11px;
    cursor: pointer;
  }
  button.primary {
    background: #238636;
    border-color: #2ea043;
    color: #ffffff;
  }
</style>
