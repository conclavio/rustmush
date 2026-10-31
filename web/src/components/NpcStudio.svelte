<script lang="ts">
  // Cut CS.1 — NPC Studio (Station Cast, Crew, Passengers, and Lore Pools)
  // (doc 34 §3, D-34-3, D-34-4, D-34-8, D-34-9).
  import type { JsonValue } from "$lib/proto";
  import type { ContentStudioRpc } from "$lib/content/rpc";
  import type { CatalogData } from "$lib/content/store";
  import {
    applyAssistPatchToNpc,
    applyLorePoolToDoc,
    applyNpcModelToDoc,
    CREW_DUTIES,
    type CrewDuty,
    extractLorePools,
    extractNpcModels,
    type LorePoolEditorModel,
    NPC_BOUNDS,
    type NpcEditorModel,
    type NpcRoleFilter,
    relocateNpcToPort,
    utf8ByteLength,
  } from "$lib/content/npc-editor";

  let {
    documentJson,
    catalog,
    rpc,
    relocateNpcs = $bindable<string[]>([]),
    onChange,
    onStatus,
    onError,
  }: {
    documentJson: JsonValue | null;
    catalog: CatalogData;
    rpc: ContentStudioRpc | null;
    relocateNpcs?: string[];
    onChange: (nextDoc: JsonValue) => void;
    onStatus: (msg: string) => void;
    onError: (msg: string) => void;
  } = $props();

  let roleFilter = $state<NpcRoleFilter>("all");
  let selectedEntryId = $state<string>("");
  let assistBrief = $state<string>(
    "Create a new crew member who specializes in maintenance/repair, based on ceres, heavily experienced",
  );
  let assistingScope = $state<string | null>(null);

  const npcs = $derived<NpcEditorModel[]>(extractNpcModels(documentJson));
  const lorePools = $derived<LorePoolEditorModel[]>(extractLorePools(documentJson));

  const filteredNpcs = $derived<NpcEditorModel[]>(
    npcs.filter((n) => {
      if (roleFilter === "all") return true;
      if (roleFilter === "lore") return false;
      return n.role === roleFilter;
    }),
  );

  const activeNpc = $derived<NpcEditorModel | null>(
    npcs.find((n) => n.entryId === selectedEntryId) ?? npcs[0] ?? null,
  );

  const activeLorePool = $derived<LorePoolEditorModel | null>(
    lorePools.find((l) => l.entryId === selectedEntryId) ??
      (roleFilter === "lore" ? (lorePools[0] ?? null) : null),
  );

  const activeCatalogMeta = $derived(
    activeNpc
      ? catalog.npcs.find((c) => c.entry_id === activeNpc.entryId) ?? null
      : null,
  );

  const availableRooms = $derived(
    activeNpc
      ? (catalog.stations.find((s) => s.pack_id === activeNpc.roomPack)?.rooms ?? [])
      : [],
  );

  function commitNpc(updated: NpcEditorModel): void {
    if (!documentJson) return;
    selectedEntryId = updated.entryId;
    onChange(applyNpcModelToDoc(documentJson, updated));
  }

  function commitLorePool(updated: LorePoolEditorModel): void {
    if (!documentJson) return;
    selectedEntryId = updated.entryId;
    onChange(applyLorePoolToDoc(documentJson, updated));
  }

  function addNewNpc(role: "station" | "crew" | "passenger" = "crew"): void {
    const slug = `new_npc_${npcs.length + 1}`;
    const entryId = `npc.${slug}`;
    const model: NpcEditorModel = {
      slug,
      entryId,
      name: "New Corridor Specialist",
      tier: "minor",
      roomPack: "neighborhood.ceres_freeport",
      room: "room.c10_slack_line",
      enabled: true,
      cooldownS: null,
      dailyTokens: null,
      persona: "Practical corridor hand with steady nerves and clean logbooks.",
      background: "Worked Freeport drydocks and Belt hauler maintenance watches.",
      speechStyle: "Clipped, direct, and unhurried.",
      goals: ["Keep the hull sealed and the watch bill square."],
      knows: ["Ceres Freeport berth routines and repair procedures."],
      doesNotKnow: ["Classified fleet cipher keys."],
      fallback: [
        "The log's open on the bench; give me a moment to check the seal.",
        "Ask me again once the watch bell clears.",
      ],
      watch: ["hull", "repair"],
      interfaces:
        role === "crew"
          ? ["hire"]
          : role === "passenger"
            ? ["passenger"]
            : ["npc.lore"],
      commandRefs:
        role === "crew"
          ? ["+crew"]
          : role === "passenger"
            ? ["+passage", "+ship"]
            : [],
      tags: ["corridor_trade", "port_ceres"],
      lore: [],
      role,
      crewTerms:
        role === "crew"
          ? {
              schema: 1,
              npc: entryId,
              port: "ceres_port",
              duties: ["recovery"],
              fee_minor: 50_000,
              daily_wage_minor: 2_500,
            }
          : null,
      passengerTerms:
        role === "passenger"
          ? {
              npc: entryId,
              origin: "ceres_port",
              destination: "stickney_port",
              fare_minor: 45_000,
              deadline_days: 14,
              opening: "Looking for a berth to Stickney Yards before the next shift.",
              settled: "Obliged for the lift, Captain.",
            }
          : null,
    };
    commitNpc(model);
  }

  function addNewLorePool(): void {
    const tag = `pool_${lorePools.length + 1}`;
    const pool: LorePoolEditorModel = {
      entryId: `lore.${tag}`,
      tag,
      entries: [
        {
          tier: "public",
          subject: "Corridor trade",
          fact: "Every Dray on the Ceres-Stickney lane logs its mass at the berth.",
          source: "",
        },
      ],
    };
    roleFilter = "lore";
    commitLorePool(pool);
  }

  function switchNpcRole(
    npc: NpcEditorModel,
    nextRole: "station" | "crew" | "passenger",
  ): void {
    const next: NpcEditorModel = { ...npc, role: nextRole };
    if (nextRole === "crew") {
      next.crewTerms = next.crewTerms ?? {
        schema: 1,
        npc: next.entryId,
        port: "ceres_port",
        duties: ["recovery"],
        fee_minor: 50_000,
        daily_wage_minor: 2_500,
      };
      if (!next.interfaces.includes("hire")) {
        next.interfaces = [...next.interfaces.filter((i) => i !== "passenger"), "hire"];
      }
      if (!next.commandRefs.includes("+crew")) {
        next.commandRefs = ["+crew"];
      }
    } else if (nextRole === "passenger") {
      next.passengerTerms = next.passengerTerms ?? {
        npc: next.entryId,
        origin: "ceres_port",
        destination: "stickney_port",
        fare_minor: 45_000,
        deadline_days: 14,
        opening: "Looking for passage down the corridor.",
        settled: "Safe berth and fair accounts, Captain.",
      };
      if (!next.interfaces.includes("passenger")) {
        next.interfaces = [...next.interfaces.filter((i) => i !== "hire"), "passenger"];
      }
      if (!next.commandRefs.includes("+passage")) {
        next.commandRefs = ["+passage", "+ship"];
      }
    } else {
      next.interfaces = next.interfaces.filter((i) => i !== "hire" && i !== "passenger");
    }
    commitNpc(next);
  }

  function toggleCrewDuty(npc: NpcEditorModel, duty: CrewDuty): void {
    if (!npc.crewTerms) return;
    const has = npc.crewTerms.duties.includes(duty);
    const nextDuties = has
      ? npc.crewTerms.duties.filter((d) => d !== duty)
      : [...npc.crewTerms.duties, duty];
    if (nextDuties.length === 0) return;
    commitNpc({
      ...npc,
      crewTerms: {
        ...npc.crewTerms,
        duties: nextDuties,
      },
    });
  }

  function toggleChip(list: string[], item: string): string[] {
    return list.includes(item) ? list.filter((x) => x !== item) : [...list, item];
  }

  function toggleRelocateLive(entryId: string, checked: boolean): void {
    if (checked) {
      if (!relocateNpcs.includes(entryId)) {
        relocateNpcs = [...relocateNpcs, entryId];
      }
    } else {
      relocateNpcs = relocateNpcs.filter((id) => id !== entryId);
    }
  }

  async function runAssist(scope: string, createNew = false): Promise<void> {
    if (!rpc) return;
    assistingScope = scope;
    const res = await rpc.request({
      op: "assist",
      family: "npc",
      scope,
      brief: assistBrief,
      context: (activeNpc ?? {}) as unknown as JsonValue,
    });
    assistingScope = null;
    if (res.status === "error") {
      onError(res.message);
      return;
    }
    if (res.status === "assisted") {
      onStatus(
        `Assisted ${scope} (${res.model}, prompt ${res.prompt_hash.slice(0, 8)}…)`,
      );
      if (createNew || !activeNpc) {
        const patchObj = res.patch as Record<string, unknown>;
        const slug =
          typeof patchObj.slug === "string"
            ? patchObj.slug
            : `assisted_${npcs.length + 1}`;
        const base: NpcEditorModel = {
          slug,
          entryId: `npc.${slug}`,
          name: slug,
          tier: "minor",
          roomPack: "neighborhood.ceres_freeport",
          room: "room.c10_slack_line",
          enabled: true,
          cooldownS: null,
          dailyTokens: null,
          persona: "",
          background: "",
          speechStyle: "",
          goals: [],
          knows: [],
          doesNotKnow: [],
          fallback: [],
          watch: [],
          interfaces: ["hire"],
          commandRefs: ["+crew"],
          tags: ["corridor_trade"],
          lore: [],
          role: "crew",
          crewTerms: null,
          passengerTerms: null,
        };
        commitNpc(applyAssistPatchToNpc(base, "full", res.patch));
      } else {
        commitNpc(applyAssistPatchToNpc(activeNpc, scope, res.patch));
      }
    }
  }
</script>

<div class="npc-studio">
  <!-- LLM Co-Authoring Bar (D-34-8) -->
  <div class="assist-bar">
    <input
      type="text"
      class="assist-input"
      placeholder="Describe an NPC (e.g. 'create a new crew member who specializes in maintenance/repair, based on ceres, heavily experienced')…"
      bind:value={assistBrief}
    />
    <button
      type="button"
      class="primary"
      disabled={assistingScope !== null}
      onclick={() => void runAssist("full", true)}
    >
      {assistingScope === "full" ? "Synthesizing…" : "LLM Draft New NPC"}
    </button>
    {#if activeNpc}
      <button
        type="button"
        disabled={assistingScope !== null}
        onclick={() => void runAssist("full", false)}
      >
        Apply Brief to {activeNpc.name}
      </button>
    {/if}
  </div>

  <!-- Filter & Action Bar -->
  <div class="filter-bar">
    <div class="role-pills">
      <button
        type="button"
        class="chip"
        class:active={roleFilter === "all"}
        onclick={() => (roleFilter = "all")}
      >
        All NPCs ({npcs.length})
      </button>
      <button
        type="button"
        class="chip"
        class:active={roleFilter === "station"}
        onclick={() => (roleFilter = "station")}
      >
        Station Cast ({npcs.filter((n) => n.role === "station").length})
      </button>
      <button
        type="button"
        class="chip"
        class:active={roleFilter === "crew"}
        onclick={() => (roleFilter = "crew")}
      >
        Crew ({npcs.filter((n) => n.role === "crew").length})
      </button>
      <button
        type="button"
        class="chip"
        class:active={roleFilter === "passenger"}
        onclick={() => (roleFilter = "passenger")}
      >
        Passengers ({npcs.filter((n) => n.role === "passenger").length})
      </button>
      <button
        type="button"
        class="chip"
        class:active={roleFilter === "lore"}
        onclick={() => (roleFilter = "lore")}
      >
        Lore Pools ({lorePools.length})
      </button>
    </div>
    <div class="add-buttons">
      <button type="button" onclick={() => addNewNpc("crew")}>+ New Crew</button>
      <button type="button" onclick={() => addNewNpc("passenger")}>+ New Passenger</button>
      <button type="button" onclick={() => addNewNpc("station")}>+ New Station NPC</button>
      <button type="button" onclick={addNewLorePool}>+ New Lore Pool</button>
    </div>
  </div>

  <div class="npc-workspace">
    <!-- Left Card Grid -->
    <div class="card-grid">
      {#if roleFilter === "lore"}
        {#each lorePools as pool (pool.entryId)}
          <button
            type="button"
            class="npc-card"
            class:selected={activeLorePool?.entryId === pool.entryId}
            onclick={() => (selectedEntryId = pool.entryId)}
          >
            <div class="card-top">
              <strong>#{pool.tag}</strong>
              <span class="tag-pill">lore ({pool.entries.length})</span>
            </div>
            <div class="card-sub">
              <code>{pool.entryId}</code>
            </div>
          </button>
        {/each}
      {:else}
        {#each filteredNpcs as npc (npc.entryId)}
          {@const meta = catalog.npcs.find((c) => c.entry_id === npc.entryId)}
          <button
            type="button"
            class="npc-card"
            class:selected={activeNpc?.entryId === npc.entryId}
            onclick={() => (selectedEntryId = npc.entryId)}
          >
            <div class="card-top">
              <strong>{npc.name}</strong>
              <span class="tier-badge {npc.tier}">{npc.tier}</span>
            </div>
            <div class="card-sub">
              <code>{npc.entryId}</code> · <span>{npc.room}</span>
            </div>
            <div class="card-badges">
              <span class="role-badge">{npc.role}</span>
              {#if !npc.enabled}
                <span class="warn-badge" title="Principal NPC — never wakes on LLM stimulus">
                  PRINCIPAL (OFF)
                </span>
              {/if}
              {#if meta?.code_bound}
                <span
                  class="code-bound-badge"
                  title={`Code-bound: ${meta.code_bindings.join("; ")}`}
                >
                  CODE-BOUND
                </span>
              {/if}
              {#if meta?.latched}
                <span class="latched-badge" title="Latched on live world (slug locked)">
                  LATCHED
                </span>
              {/if}
            </div>
          </button>
        {/each}
      {/if}
    </div>

    <!-- Right Inspector -->
    <div class="inspector-pane">
      {#if roleFilter === "lore" && activeLorePool}
        <h4>Shared Lore Pool: <code>{activeLorePool.entryId}</code></h4>
        <label class="field-label">
          <span>Pool Tag</span>
          <input
            type="text"
            value={activeLorePool.tag}
            oninput={(e) =>
              commitLorePool({
                ...activeLorePool,
                tag: (e.currentTarget as HTMLInputElement).value,
              })}
          />
        </label>
        <div class="list-section">
          <div class="section-head">
            <span>Lore Facts ({activeLorePool.entries.length})</span>
            <button
              type="button"
              onclick={() =>
                commitLorePool({
                  ...activeLorePool,
                  entries: [
                    ...activeLorePool.entries,
                    { tier: "public", subject: "", fact: "", source: "" },
                  ],
                })}
            >
              + Add Fact
            </button>
          </div>
          {#each activeLorePool.entries as item, idx (idx)}
            <div class="lore-row">
              <select
                value={item.tier}
                onchange={(e) => {
                  const next = [...activeLorePool.entries];
                  next[idx] = {
                    ...item,
                    tier: (e.currentTarget as HTMLSelectElement).value as
                      | "public"
                      | "guarded"
                      | "secret",
                  };
                  commitLorePool({ ...activeLorePool, entries: next });
                }}
              >
                <option value="public">public</option>
                <option value="guarded">guarded</option>
                <option value="secret">secret</option>
              </select>
              <input
                type="text"
                placeholder="Subject…"
                value={item.subject}
                oninput={(e) => {
                  const next = [...activeLorePool.entries];
                  next[idx] = {
                    ...item,
                    subject: (e.currentTarget as HTMLInputElement).value,
                  };
                  commitLorePool({ ...activeLorePool, entries: next });
                }}
              />
              <input
                type="text"
                placeholder="Fact…"
                value={item.fact}
                oninput={(e) => {
                  const next = [...activeLorePool.entries];
                  next[idx] = {
                    ...item,
                    fact: (e.currentTarget as HTMLInputElement).value,
                  };
                  commitLorePool({ ...activeLorePool, entries: next });
                }}
              />
            </div>
          {/each}
        </div>
      {:else if activeNpc}
        <div class="inspector-head">
          <h4>{activeNpc.name} (<code>{activeNpc.entryId}</code>)</h4>
          {#if activeCatalogMeta?.code_bound}
            <div class="guardrail-alert">
              <strong>Code-Bound Entry (D-34-9):</strong> Referenced by
              <code>{activeCatalogMeta.code_bindings.join(", ")}</code>. Entry ID cannot be renamed or retired.
            </div>
          {/if}
        </div>

        <div class="field-grid">
          <label class="field-label">
            <span>Display Name (≤ {NPC_BOUNDS.maxNameBytes} B, no `/=;` )</span>
            <input
              type="text"
              value={activeNpc.name}
              oninput={(e) =>
                commitNpc({
                  ...activeNpc,
                  name: (e.currentTarget as HTMLInputElement).value,
                })}
            />
          </label>

          <label class="field-label">
            <span>Tier</span>
            <select
              value={activeNpc.tier}
              onchange={(e) =>
                commitNpc({
                  ...activeNpc,
                  tier: (e.currentTarget as HTMLSelectElement).value as
                    | "major"
                    | "minor"
                    | "extra",
                })}
            >
              <option value="major">major (30s cooldown / 24k tokens)</option>
              <option value="minor">minor (120s cooldown / 8k tokens)</option>
              <option value="extra">extra (300s cooldown / 2k tokens)</option>
            </select>
          </label>

          <label class="field-label">
            <span>Role Pairing</span>
            <select
              value={activeNpc.role}
              disabled={Boolean(activeCatalogMeta?.latched)}
              onchange={(e) =>
                switchNpcRole(
                  activeNpc,
                  (e.currentTarget as HTMLSelectElement).value as
                    | "station"
                    | "crew"
                    | "passenger",
                )}
            >
              <option value="station">Station Cast (npc only)</option>
              <option value="crew">Crew for Hire (npc + crew_for_hire, schema 1)</option>
              <option value="passenger">Passenger (npc + passenger)</option>
            </select>
          </label>

          <label class="checkbox-field">
            <input
              type="checkbox"
              checked={activeNpc.enabled}
              onchange={(e) =>
                commitNpc({
                  ...activeNpc,
                  enabled: (e.currentTarget as HTMLInputElement).checked,
                })}
            />
            <span>Agent Enabled (uncheck for non-waking Principals)</span>
          </label>
        </div>

        <!-- 11-Port Relocation Cascade & Room Selector (D-34-9) -->
        <div class="location-box">
          <div class="field-grid">
            <label class="field-label">
              <span>Quick Move to Corridor Port (11 Ports)</span>
              <select
                value={
                  catalog.ports.find((p) => p.area === activeNpc.roomPack)?.id ?? ""
                }
                onchange={(e) =>
                  commitNpc(
                    relocateNpcToPort(
                      activeNpc,
                      (e.currentTarget as HTMLSelectElement).value,
                      catalog,
                    ),
                  )}
              >
                {#each catalog.ports as port (port.id)}
                  <option value={port.id}>{port.venue} ({port.id} → {port.area})</option>
                {/each}
              </select>
            </label>

            <label class="field-label">
              <span>Authored Room (<code>{activeNpc.roomPack}</code>)</span>
              <select
                value={activeNpc.room}
                onchange={(e) =>
                  commitNpc({
                    ...activeNpc,
                    room: (e.currentTarget as HTMLSelectElement).value,
                  })}
              >
                {#if availableRooms.length === 0}
                  <option value={activeNpc.room}>{activeNpc.room}</option>
                {:else}
                  {#each availableRooms as room (room.id)}
                    <option value={room.id}>{room.id} — {room.name}</option>
                  {/each}
                {/if}
              </select>
            </label>
          </div>
          <label class="checkbox-field">
            <input
              type="checkbox"
              checked={relocateNpcs.includes(activeNpc.entryId)}
              onchange={(e) =>
                toggleRelocateLive(
                  activeNpc.entryId,
                  (e.currentTarget as HTMLInputElement).checked,
                )}
            />
            <span>
              Also relocate live NPC object in running world on Publish (D-34-9:
              <code>NpcApplier::update</code> preserves live location unless checked)
            </span>
          </label>
        </div>

        <!-- Crew Terms (D-34-4: schema 1, fixed duties enum) -->
        {#if activeNpc.role === "crew" && activeNpc.crewTerms}
          {@const terms = activeNpc.crewTerms}
          <div class="terms-box">
            <h5>Crew Hire Terms (<code>crew_for_hire.{activeNpc.slug}</code>, schema: 1)</h5>
            <div class="duty-row">
              <span>Duties (D-34-4 fixed enum):</span>
              {#each CREW_DUTIES as duty (duty)}
                <label class="checkbox-field">
                  <input
                    type="checkbox"
                    checked={terms.duties.includes(duty)}
                    onchange={() => toggleCrewDuty(activeNpc, duty)}
                  />
                  <code>{duty}</code>
                </label>
              {/each}
            </div>
            <div class="field-grid">
              <label class="field-label">
                <span>Sign-On Fee Minor ({(terms.fee_minor / 100).toFixed(0)} cr)</span>
                <input
                  type="number"
                  min="1"
                  max="1000000000"
                  value={terms.fee_minor}
                  oninput={(e) =>
                    commitNpc({
                      ...activeNpc,
                      crewTerms: {
                        ...terms,
                        fee_minor: Number((e.currentTarget as HTMLInputElement).value) || 1,
                      },
                    })}
                />
              </label>
              <label class="field-label">
                <span>Daily Wage Minor ({(terms.daily_wage_minor / 100).toFixed(0)} cr/day)</span>
                <input
                  type="number"
                  min="1"
                  max="100000000"
                  value={terms.daily_wage_minor}
                  oninput={(e) =>
                    commitNpc({
                      ...activeNpc,
                      crewTerms: {
                        ...terms,
                        daily_wage_minor:
                          Number((e.currentTarget as HTMLInputElement).value) || 1,
                      },
                    })}
                />
              </label>
            </div>
          </div>
        {/if}

        <!-- Passenger Terms -->
        {#if activeNpc.role === "passenger" && activeNpc.passengerTerms}
          {@const pterms = activeNpc.passengerTerms}
          <div class="terms-box">
            <h5>Passenger Terms (<code>passenger.{activeNpc.slug}</code>)</h5>
            <div class="field-grid">
              <label class="field-label">
                <span>Origin Port</span>
                <select
                  value={pterms.origin}
                  onchange={(e) =>
                    commitNpc({
                      ...activeNpc,
                      passengerTerms: {
                        ...pterms,
                        origin: (e.currentTarget as HTMLSelectElement).value,
                      },
                    })}
                >
                  {#each catalog.ports as port (port.id)}
                    <option value={port.id}>{port.venue} ({port.id})</option>
                  {/each}
                </select>
              </label>
              <label class="field-label">
                <span>Destination Port (≠ Origin)</span>
                <select
                  value={pterms.destination}
                  onchange={(e) =>
                    commitNpc({
                      ...activeNpc,
                      passengerTerms: {
                        ...pterms,
                        destination: (e.currentTarget as HTMLSelectElement).value,
                      },
                    })}
                >
                  {#each catalog.ports as port (port.id)}
                    <option value={port.id}>{port.venue} ({port.id})</option>
                  {/each}
                </select>
              </label>
              <label class="field-label">
                <span>Fare Minor ({(pterms.fare_minor / 100).toFixed(0)} cr)</span>
                <input
                  type="number"
                  min="1"
                  value={pterms.fare_minor}
                  oninput={(e) =>
                    commitNpc({
                      ...activeNpc,
                      passengerTerms: {
                        ...pterms,
                        fare_minor:
                          Number((e.currentTarget as HTMLInputElement).value) || 1,
                      },
                    })}
                />
              </label>
              <label class="field-label">
                <span>Deadline Days (1–365)</span>
                <input
                  type="number"
                  min="1"
                  max="365"
                  value={pterms.deadline_days}
                  oninput={(e) =>
                    commitNpc({
                      ...activeNpc,
                      passengerTerms: {
                        ...pterms,
                        deadline_days:
                          Number((e.currentTarget as HTMLInputElement).value) || 14,
                      },
                    })}
                />
              </label>
            </div>
          </div>
        {/if}

        <!-- Profile Prose Fields with Live Byte Counters & Per-Field Regenerate -->
        <div class="prose-section">
          <div class="section-head">
            <span>
              Persona ({utf8ByteLength(activeNpc.persona)} / {NPC_BOUNDS.maxPersonaBytes} B)
            </span>
            <button
              type="button"
              class="small-btn"
              onclick={() => void runAssist("persona")}
            >
              Regenerate Persona
            </button>
          </div>
          <textarea
            rows="3"
            value={activeNpc.persona}
            oninput={(e) =>
              commitNpc({
                ...activeNpc,
                persona: (e.currentTarget as HTMLTextAreaElement).value,
              })}
          ></textarea>

          <div class="section-head">
            <span>
              Background ({utf8ByteLength(activeNpc.background)} / {NPC_BOUNDS.maxBackgroundBytes} B)
            </span>
            <button
              type="button"
              class="small-btn"
              onclick={() => void runAssist("background")}
            >
              Regenerate Background
            </button>
          </div>
          <textarea
            rows="3"
            value={activeNpc.background}
            oninput={(e) =>
              commitNpc({
                ...activeNpc,
                background: (e.currentTarget as HTMLTextAreaElement).value,
              })}
          ></textarea>

          <div class="section-head">
            <span>
              Speech Style ({utf8ByteLength(activeNpc.speechStyle)} / {NPC_BOUNDS.maxSpeechBytes} B)
            </span>
            <button
              type="button"
              class="small-btn"
              onclick={() => void runAssist("speech_style")}
            >
              Regenerate Speech
            </button>
          </div>
          <textarea
            rows="2"
            value={activeNpc.speechStyle}
            oninput={(e) =>
              commitNpc({
                ...activeNpc,
                speechStyle: (e.currentTarget as HTMLTextAreaElement).value,
              })}
          ></textarea>

          <div class="section-head">
            <span>
              Goals ({activeNpc.goals.length} / {NPC_BOUNDS.maxListItems}, one per line)
            </span>
            <button
              type="button"
              class="small-btn"
              onclick={() => void runAssist("goals")}
            >
              Regenerate Goals
            </button>
          </div>
          <textarea
            rows="3"
            value={activeNpc.goals.join("\n")}
            oninput={(e) =>
              commitNpc({
                ...activeNpc,
                goals: (e.currentTarget as HTMLTextAreaElement).value
                  .split("\n")
                  .map((s) => s.trim())
                  .filter(Boolean),
              })}
          ></textarea>

          <div class="section-head">
            <span>
              Knows ({activeNpc.knows.length} / {NPC_BOUNDS.maxListItems}, one per line)
            </span>
            <button
              type="button"
              class="small-btn"
              onclick={() => void runAssist("knows")}
            >
              Regenerate Knows
            </button>
          </div>
          <textarea
            rows="3"
            value={activeNpc.knows.join("\n")}
            oninput={(e) =>
              commitNpc({
                ...activeNpc,
                knows: (e.currentTarget as HTMLTextAreaElement).value
                  .split("\n")
                  .map((s) => s.trim())
                  .filter(Boolean),
              })}
          ></textarea>

          <div class="section-head">
            <span>
              Fallback Lines ({activeNpc.fallback.length} / {NPC_BOUNDS.maxFallbackItems}, one per line)
            </span>
            <button
              type="button"
              class="small-btn"
              onclick={() => void runAssist("fallback")}
            >
              Regenerate Fallback
            </button>
          </div>
          <textarea
            rows="3"
            value={activeNpc.fallback.join("\n")}
            oninput={(e) =>
              commitNpc({
                ...activeNpc,
                fallback: (e.currentTarget as HTMLTextAreaElement).value
                  .split("\n")
                  .map((s) => s.trim())
                  .filter(Boolean),
              })}
          ></textarea>
        </div>

        <!-- Registered Interfaces, Command Refs, and Lore Pool Tags -->
        <div class="chip-section">
          <span>Interfaces:</span>
          {#each catalog.interfaces as iface (iface)}
            <button
              type="button"
              class="chip"
              class:active={activeNpc.interfaces.includes(iface)}
              onclick={() =>
                commitNpc({
                  ...activeNpc,
                  interfaces: toggleChip(activeNpc.interfaces, iface),
                })}
            >
              {iface}
            </button>
          {/each}
        </div>

        <div class="chip-section">
          <span>Command Guidance Refs:</span>
          {#each catalog.command_refs as cmd (cmd)}
            <button
              type="button"
              class="chip"
              class:active={activeNpc.commandRefs.includes(cmd)}
              onclick={() =>
                commitNpc({
                  ...activeNpc,
                  commandRefs: toggleChip(activeNpc.commandRefs, cmd),
                })}
            >
              {cmd}
            </button>
          {/each}
        </div>
      {/if}
    </div>
  </div>
</div>

<style>
  .npc-studio {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .assist-bar,
  .filter-bar {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: center;
    justify-content: space-between;
  }
  .assist-input {
    flex: 1;
    min-width: 260px;
    background: #161b22;
    color: #c9d1d9;
    border: 1px solid #30363d;
    border-radius: 4px;
    padding: 5px 8px;
    font-size: 12px;
  }
  .role-pills,
  .add-buttons,
  .chip-section,
  .duty-row {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
  }
  .npc-workspace {
    display: grid;
    grid-template-columns: 270px 1fr;
    gap: 10px;
    min-height: 420px;
  }
  .card-grid {
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-height: 520px;
    overflow-y: auto;
  }
  .npc-card {
    text-align: left;
    background: #161b22;
    border: 1px solid #30363d;
    border-radius: 6px;
    padding: 8px;
    color: #c9d1d9;
    cursor: pointer;
  }
  .npc-card.selected {
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
    margin-top: 2px;
  }
  .card-badges {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin-top: 4px;
  }
  .tier-badge,
  .role-badge,
  .warn-badge,
  .code-bound-badge,
  .latched-badge,
  .tag-pill {
    font-size: 10px;
    padding: 1px 5px;
    border-radius: 999px;
    background: #21262d;
  }
  .code-bound-badge {
    background: rgba(210, 153, 34, 0.25);
    color: #e3b341;
  }
  .latched-badge {
    background: rgba(56, 139, 253, 0.2);
    color: #58a6ff;
  }
  .warn-badge {
    background: rgba(248, 81, 73, 0.2);
    color: #ff7b72;
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
  .lore-row input,
  .lore-row select {
    background: #0d1117;
    color: #c9d1d9;
    border: 1px solid #30363d;
    border-radius: 4px;
    padding: 4px 6px;
    font-size: 12px;
  }
  .location-box,
  .terms-box {
    border: 1px solid #30363d;
    border-radius: 4px;
    padding: 8px;
    background: #0d1117;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .terms-box h5,
  .inspector-head h4 {
    margin: 0;
  }
  .section-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: 11px;
    color: #8b949e;
    margin-top: 4px;
  }
  .chip,
  button {
    background: #21262d;
    color: #c9d1d9;
    border: 1px solid #30363d;
    border-radius: 4px;
    padding: 3px 8px;
    font-size: 11px;
    cursor: pointer;
  }
  .chip.active,
  button.primary {
    background: #238636;
    border-color: #2ea043;
    color: #ffffff;
  }
  .guardrail-alert {
    font-size: 11px;
    padding: 6px;
    border-radius: 4px;
    background: rgba(210, 153, 34, 0.15);
    border: 1px solid rgba(210, 153, 34, 0.4);
    color: #e3b341;
  }
  .lore-row {
    display: grid;
    grid-template-columns: 90px 140px 1fr;
    gap: 6px;
    margin-top: 4px;
  }
</style>
