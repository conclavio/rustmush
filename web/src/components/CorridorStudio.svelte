<script lang="ts">
  // Cut CS.4 & CS.5b — Corridor (`lane/ceres_phobos.json`) & Relays (`relays/luna_ceres.json`,
  // `relays/manufacture-v1.json`) Studio (doc 34 §5.2, D-34-6).
  import type { JsonValue } from "$lib/proto";
  import type { CatalogData } from "$lib/content/store";
  import {
    type CorridorEntrySummary,
    type CorridorFilter,
    type PortOrgDraft,
    type PortSiteDraft,
    type PortVendorDraft,
    type RelayNodeDraft,
    type RelayPairDraft,
    type RelayRecipeDraft,
    addPortOrg,
    addPortVendor,
    extractCorridorEntries,
    extractPortOrgs,
    extractPortSite,
    extractPortVendors,
    extractRelayNodes,
    extractRelayPairs,
    extractRelayRecipe,
    removePortOrg,
    removePortVendor,
    updateCorridorEntryDef,
    updatePortOrg,
    updatePortSiteBlock,
    updatePortSiteMeta,
    updatePortVendor,
    updateRelayNode,
    updateRelayPair,
    updateRelayRecipe,
  } from "$lib/content/corridor-editor";

  let {
    documentJson,
    catalog = null,
    onChange,
  }: {
    documentJson: JsonValue | null;
    catalog?: CatalogData | null;
    onChange: (nextDoc: JsonValue) => void;
  } = $props();

  let filter = $state<CorridorFilter>("all");
  let selectedId = $state<string | null>(null);

  const entries = $derived<CorridorEntrySummary[]>(
    extractCorridorEntries(documentJson),
  );

  const filteredEntries = $derived<CorridorEntrySummary[]>(
    entries.filter((e) => {
      if (filter === "all") return true;
      return e.kind === filter;
    }),
  );

  $effect(() => {
    const first = filteredEntries[0];
    if (
      first &&
      (!selectedId || !filteredEntries.some((e) => e.id === selectedId))
    ) {
      selectedId = first.id;
    } else if (filteredEntries.length === 0) {
      selectedId = null;
    }
  });

  const activeEntry = $derived<CorridorEntrySummary | null>(
    entries.find((e) => e.id === selectedId) ?? null,
  );

  const portOrgs = $derived<PortOrgDraft[]>(
    activeEntry?.kind === "port" ? extractPortOrgs(activeEntry.def) : [],
  );

  const portVendors = $derived<PortVendorDraft[]>(
    activeEntry?.kind === "port" ? extractPortVendors(activeEntry.def) : [],
  );

  const portSite = $derived<PortSiteDraft | null>(
    activeEntry?.kind === "port" ? extractPortSite(activeEntry.def) : null,
  );

  const relayNodes = $derived<RelayNodeDraft[]>(
    activeEntry?.kind === "relay" ? extractRelayNodes(activeEntry.def) : [],
  );

  const relayPairs = $derived<RelayPairDraft[]>(
    activeEntry?.kind === "relay" ? extractRelayPairs(activeEntry.def) : [],
  );

  const relayRecipe = $derived<RelayRecipeDraft | null>(
    activeEntry?.kind === "relay" ? extractRelayRecipe(activeEntry.def) : null,
  );

  const catalogGoods = $derived<string[]>(
    catalog?.goods && catalog.goods.length > 0
      ? catalog.goods.map((g) => g.id)
      : [
          "metals",
          "volatiles",
          "propellant",
          "provisions",
          "structural_alloys",
          "industrial_parts",
          "instrument_boards",
          "carbon_feedstock",
          "metal_ore",
          "ice_ore",
          "carbon_ore",
          "strategic_ore",
        ],
  );

  const signatoryOptions = $derived<Array<{ value: string; label: string }>>(
    (catalog?.npcs ?? []).map((n) => {
      const areaPrefix = n.room_pack.startsWith("neighborhood.")
        ? n.room_pack
        : n.pack_id.replace("tacsim.cast.", "neighborhood.");
      const val = `${areaPrefix}.${n.entry_id}`;
      return {
        value: val,
        label: `${n.name} (${val})`,
      };
    }),
  );

  function commitDef(nextDef: Record<string, unknown>): void {
    if (!documentJson || !activeEntry || activeEntry.readOnly) return;
    onChange(updateCorridorEntryDef(documentJson, activeEntry.id, nextDef));
  }

  function patchField(key: string, value: unknown): void {
    if (!activeEntry || activeEntry.readOnly) return;
    commitDef({
      ...activeEntry.def,
      [key]: value,
    });
  }

  function patchSubJson(key: string, rawText: string): void {
    if (!activeEntry || activeEntry.readOnly) return;
    try {
      const parsed = JSON.parse(rawText);
      patchField(key, parsed);
    } catch {
      // Wait for valid JSON before patching
    }
  }
</script>

<div class="corridor-studio">
  <div class="corridor-filter-bar">
    {#each [
      ["all", "All"],
      ["port", "Ports (11)"],
      ["route", "Routes (20)"],
      ["location", "Locations (12)"],
      ["body", "Ephemeris Bodies (16, read-only)"],
      ["relay", "Relays & Manufacture"],
    ] as [id, label] (id)}
      <button
        type="button"
        class="filter-pill"
        class:active={filter === id}
        onclick={() => (filter = id as CorridorFilter)}
      >
        {label}
      </button>
    {/each}
  </div>

  <div class="corridor-columns">
    <div class="entry-list">
      {#each filteredEntries as entry (entry.id)}
        <button
          type="button"
          class="entry-card"
          class:active={entry.id === selectedId}
          onclick={() => (selectedId = entry.id)}
        >
          <div class="row-top">
            <strong>{entry.label}</strong>
            <span class="kind-chip">{entry.kind}</span>
          </div>
          <div class="row-sub">
            <code>{entry.id}</code> · {entry.subtitle}
          </div>
        </button>
      {/each}
    </div>

    {#if activeEntry}
      <div class="entry-inspector">
        <div class="inspector-head">
          <h4>
            {activeEntry.label} (<code>{activeEntry.id}</code>)
          </h4>
          {#if activeEntry.readOnly}
            <span class="ro-badge">Read-Only (JPL Horizons ephemeris)</span>
          {/if}
        </div>

        {#if activeEntry.kind === "body"}
          <div class="ro-notice">
            Generated by <code>fetch_horizons.py</code>. Orbital element tables are read-only in the browser to preserve astrological precision.
          </div>
          <pre class="ro-pre">{JSON.stringify(activeEntry.def, null, 2)}</pre>
        {:else if activeEntry.kind === "port"}
          <div class="field-grid">
            <label>
              <span>Venue Name</span>
              <input
                type="text"
                value={String(activeEntry.def.venue ?? "")}
                oninput={(e) => patchField("venue", (e.currentTarget as HTMLInputElement).value)}
              />
            </label>
            <label>
              <span>Location Ref</span>
              <input
                type="text"
                value={String(activeEntry.def.location ?? "")}
                oninput={(e) => patchField("location", (e.currentTarget as HTMLInputElement).value)}
              />
            </label>
            <label>
              <span>Neighborhood Area Ref</span>
              <input
                type="text"
                value={String(activeEntry.def.area ?? "")}
                oninput={(e) => patchField("area", (e.currentTarget as HTMLInputElement).value)}
              />
            </label>
          </div>

          <!-- Structured Port Organizations Builder -->
          <div class="builder-section">
            <div class="section-head">
              <strong>Organizations ({portOrgs.length})</strong>
              <button
                type="button"
                class="mini-btn"
                onclick={() => commitDef(addPortOrg(activeEntry.def))}
              >
                + Add Org
              </button>
            </div>
            {#each portOrgs as org, idx (idx)}
              <div class="sub-card">
                <div class="field-grid">
                  <label>
                    <span>Org ID</span>
                    <input
                      type="text"
                      value={org.id}
                      oninput={(e) =>
                        commitDef(
                          updatePortOrg(activeEntry.def, idx, {
                            id: (e.currentTarget as HTMLInputElement).value,
                          }),
                        )}
                    />
                  </label>
                  <label>
                    <span>Display Name</span>
                    <input
                      type="text"
                      value={org.name}
                      oninput={(e) =>
                        commitDef(
                          updatePortOrg(activeEntry.def, idx, {
                            name: (e.currentTarget as HTMLInputElement).value,
                          }),
                        )}
                    />
                  </label>
                  <label>
                    <span>Capital (Major Cr)</span>
                    <input
                      type="number"
                      value={org.capital_major}
                      oninput={(e) =>
                        commitDef(
                          updatePortOrg(activeEntry.def, idx, {
                            capital_major: Number((e.currentTarget as HTMLInputElement).value),
                          }),
                        )}
                    />
                  </label>
                  <label>
                    <span>Faction</span>
                    <input
                      type="text"
                      value={org.faction}
                      oninput={(e) =>
                        commitDef(
                          updatePortOrg(activeEntry.def, idx, {
                            faction: (e.currentTarget as HTMLInputElement).value,
                          }),
                        )}
                    />
                  </label>
                </div>
                <button
                  type="button"
                  class="mini-btn danger"
                  onclick={() => commitDef(removePortOrg(activeEntry.def, idx))}
                >
                  Remove Org
                </button>
              </div>
            {/each}
          </div>

          <!-- Structured Port Vendors Builder -->
          <div class="builder-section">
            <div class="section-head">
              <strong>Vendors ({portVendors.length})</strong>
              <button
                type="button"
                class="mini-btn"
                onclick={() => commitDef(addPortVendor(activeEntry.def, activeEntry.id))}
              >
                + Add Vendor
              </button>
            </div>
            {#each portVendors as vendor, vIdx (vIdx)}
              <div class="sub-card">
                <div class="field-grid">
                  <label>
                    <span>Vendor Org</span>
                    <input
                      type="text"
                      value={vendor.org}
                      oninput={(e) =>
                        commitDef(
                          updatePortVendor(activeEntry.def, vIdx, {
                            ...vendor,
                            org: (e.currentTarget as HTMLInputElement).value,
                          }),
                        )}
                    />
                  </label>
                  <label>
                    <span>Signatory NPC</span>
                    <select
                      value={vendor.signatory}
                      onchange={(e) =>
                        commitDef(
                          updatePortVendor(activeEntry.def, vIdx, {
                            ...vendor,
                            signatory: (e.currentTarget as HTMLSelectElement).value,
                          }),
                        )}
                    >
                      <option value={vendor.signatory}>
                        {vendor.signatory || "(select signatory NPC)"}
                      </option>
                      {#each signatoryOptions as opt (opt.value)}
                        <option value={opt.value}>{opt.label}</option>
                      {/each}
                    </select>
                  </label>
                  <label>
                    <span>Storefront Item Ref</span>
                    <input
                      type="text"
                      value={vendor.storefront}
                      oninput={(e) =>
                        commitDef(
                          updatePortVendor(activeEntry.def, vIdx, {
                            ...vendor,
                            storefront: (e.currentTarget as HTMLInputElement).value,
                          }),
                        )}
                    />
                  </label>
                  <label>
                    <span>Clerk Agent</span>
                    <input
                      type="text"
                      value={vendor.agent}
                      oninput={(e) =>
                        commitDef(
                          updatePortVendor(activeEntry.def, vIdx, {
                            ...vendor,
                            agent: (e.currentTarget as HTMLInputElement).value,
                          }),
                        )}
                    />
                  </label>
                </div>

                <!-- Vendor Stock -->
                <div class="table-block">
                  <div class="section-head">
                    <span>Stock ({vendor.stock.length})</span>
                    <button
                      type="button"
                      class="mini-btn"
                      onclick={() =>
                        commitDef(
                          updatePortVendor(activeEntry.def, vIdx, {
                            ...vendor,
                            stock: [
                              ...vendor.stock,
                              { good: "metals", qty: 500, grade: 50, issue: 1 },
                            ],
                          }),
                        )}
                    >
                      + Stock
                    </button>
                  </div>
                  {#each vendor.stock as st, sIdx (sIdx)}
                    <div class="inline-row">
                      <select
                        value={st.good}
                        onchange={(e) => {
                          const nextStock = [...vendor.stock];
                          nextStock[sIdx] = {
                            ...st,
                            good: (e.currentTarget as HTMLSelectElement).value,
                          };
                          commitDef(
                            updatePortVendor(activeEntry.def, vIdx, {
                              ...vendor,
                              stock: nextStock,
                            }),
                          );
                        }}
                      >
                        {#each catalogGoods as g (g)}
                          <option value={g}>{g}</option>
                        {/each}
                      </select>
                      <input
                        type="number"
                        title="Quantity"
                        value={st.qty}
                        oninput={(e) => {
                          const nextStock = [...vendor.stock];
                          nextStock[sIdx] = {
                            ...st,
                            qty: Number((e.currentTarget as HTMLInputElement).value),
                          };
                          commitDef(
                            updatePortVendor(activeEntry.def, vIdx, {
                              ...vendor,
                              stock: nextStock,
                            }),
                          );
                        }}
                      />
                      <button
                        type="button"
                        class="mini-btn danger"
                        onclick={() => {
                          const nextStock = vendor.stock.filter((_, i) => i !== sIdx);
                          commitDef(
                            updatePortVendor(activeEntry.def, vIdx, {
                              ...vendor,
                              stock: nextStock,
                            }),
                          );
                        }}
                      >
                        ×
                      </button>
                    </div>
                  {/each}
                </div>

                <!-- Vendor Asks & Bids -->
                <div class="table-block">
                  <div class="section-head">
                    <span>Asks ({vendor.asks.length}) &amp; Bids ({vendor.bids.length})</span>
                    <div>
                      <button
                        type="button"
                        class="mini-btn"
                        onclick={() =>
                          commitDef(
                            updatePortVendor(activeEntry.def, vIdx, {
                              ...vendor,
                              asks: [
                                ...vendor.asks,
                                { good: "metals", price_major: 100, boards: ["exchange"] },
                              ],
                            }),
                          )}
                      >
                        + Ask
                      </button>
                      <button
                        type="button"
                        class="mini-btn"
                        onclick={() =>
                          commitDef(
                            updatePortVendor(activeEntry.def, vIdx, {
                              ...vendor,
                              bids: [
                                ...vendor.bids,
                                {
                                  good: "metal_ore",
                                  price_major: 50,
                                  intake: 100,
                                  boards: ["exchange"],
                                },
                              ],
                            }),
                          )}
                      >
                        + Bid
                      </button>
                    </div>
                  </div>
                  {#each vendor.asks as ask, aIdx (aIdx)}
                    <div class="inline-row">
                      <span class="tag-chip">ASK</span>
                      <select
                        value={ask.good}
                        onchange={(e) => {
                          const nextAsks = [...vendor.asks];
                          nextAsks[aIdx] = {
                            ...ask,
                            good: (e.currentTarget as HTMLSelectElement).value,
                          };
                          commitDef(
                            updatePortVendor(activeEntry.def, vIdx, {
                              ...vendor,
                              asks: nextAsks,
                            }),
                          );
                        }}
                      >
                        {#each catalogGoods as g (g)}
                          <option value={g}>{g}</option>
                        {/each}
                      </select>
                      <input
                        type="number"
                        title="Price (Major Cr)"
                        value={ask.price_major}
                        oninput={(e) => {
                          const nextAsks = [...vendor.asks];
                          nextAsks[aIdx] = {
                            ...ask,
                            price_major: Number((e.currentTarget as HTMLInputElement).value),
                          };
                          commitDef(
                            updatePortVendor(activeEntry.def, vIdx, {
                              ...vendor,
                              asks: nextAsks,
                            }),
                          );
                        }}
                      />
                    </div>
                  {/each}
                  {#each vendor.bids as bid, bIdx (bIdx)}
                    <div class="inline-row">
                      <span class="tag-chip">BID</span>
                      <select
                        value={bid.good}
                        onchange={(e) => {
                          const nextBids = [...vendor.bids];
                          nextBids[bIdx] = {
                            ...bid,
                            good: (e.currentTarget as HTMLSelectElement).value,
                          };
                          commitDef(
                            updatePortVendor(activeEntry.def, vIdx, {
                              ...vendor,
                              bids: nextBids,
                            }),
                          );
                        }}
                      >
                        {#each catalogGoods as g (g)}
                          <option value={g}>{g}</option>
                        {/each}
                      </select>
                      <input
                        type="number"
                        title="Price (Major Cr)"
                        value={bid.price_major}
                        oninput={(e) => {
                          const nextBids = [...vendor.bids];
                          nextBids[bIdx] = {
                            ...bid,
                            price_major: Number((e.currentTarget as HTMLInputElement).value),
                          };
                          commitDef(
                            updatePortVendor(activeEntry.def, vIdx, {
                              ...vendor,
                              bids: nextBids,
                            }),
                          );
                        }}
                      />
                      <input
                        type="number"
                        title="Intake Cap"
                        value={bid.intake}
                        oninput={(e) => {
                          const nextBids = [...vendor.bids];
                          nextBids[bIdx] = {
                            ...bid,
                            intake: Number((e.currentTarget as HTMLInputElement).value),
                          };
                          commitDef(
                            updatePortVendor(activeEntry.def, vIdx, {
                              ...vendor,
                              bids: nextBids,
                            }),
                          );
                        }}
                      />
                    </div>
                  {/each}
                </div>

                <!-- Vendor Mill Recipes -->
                {#if vendor.mill.length > 0}
                  <div class="table-block">
                    <div class="section-head">
                      <span>Mill Conversion Recipes ({vendor.mill.length})</span>
                    </div>
                    {#each vendor.mill as ml, mIdx (mIdx)}
                      <div class="inline-row">
                        <code>{ml.ore} → {ml.into}</code>
                        <span>Ratio {ml.ratio_ore}:{ml.ratio_product}</span>
                        <input
                          type="number"
                          title="Daily Cap (t)"
                          value={ml.per_ic_day_t}
                          oninput={(e) => {
                            const nextMill = [...vendor.mill];
                            nextMill[mIdx] = {
                              ...ml,
                              per_ic_day_t: Number((e.currentTarget as HTMLInputElement).value),
                            };
                            commitDef(
                              updatePortVendor(activeEntry.def, vIdx, {
                                ...vendor,
                                mill: nextMill,
                              }),
                            );
                          }}
                        />
                      </div>
                    {/each}
                  </div>
                {/if}

                <button
                  type="button"
                  class="mini-btn danger"
                  onclick={() => commitDef(removePortVendor(activeEntry.def, vIdx))}
                >
                  Remove Vendor
                </button>
              </div>
            {/each}
          </div>

          <!-- Structured Mining Site & Claim Blocks A-P Builder -->
          {#if portSite}
            <div class="builder-section">
              <div class="section-head">
                <strong>Mining Site &amp; Claim Blocks A–P ({portSite.name || activeEntry.id})</strong>
              </div>
              <div class="field-grid">
                <label>
                  <span>Site Name</span>
                  <input
                    type="text"
                    value={portSite.name}
                    oninput={(e) =>
                      commitDef(
                        updatePortSiteMeta(activeEntry.def, {
                          name: (e.currentTarget as HTMLInputElement).value,
                        }),
                      )}
                  />
                </label>
                <label>
                  <span>Issuer Org</span>
                  <input
                    type="text"
                    value={portSite.issuer}
                    oninput={(e) =>
                      commitDef(
                        updatePortSiteMeta(activeEntry.def, {
                          issuer: (e.currentTarget as HTMLInputElement).value,
                        }),
                      )}
                  />
                </label>
                <label>
                  <span>Default Tenure</span>
                  <select
                    value={portSite.defaultTenure}
                    onchange={(e) =>
                      commitDef(
                        updatePortSiteMeta(activeEntry.def, {
                          defaultTenure: (e.currentTarget as HTMLSelectElement).value,
                        }),
                      )}
                  >
                    <option value="open">open</option>
                    <option value="commons">commons</option>
                    <option value="held">held</option>
                  </select>
                </label>
              </div>

              <div class="block-grid">
                {#each portSite.blocks as blk (blk.id)}
                  <div class="block-cell">
                    <div class="row-top">
                      <strong>Block {blk.id}</strong>
                      <select
                        value={blk.tenure}
                        onchange={(e) =>
                          commitDef(
                            updatePortSiteBlock(activeEntry.def, blk.id, {
                              tenure: (e.currentTarget as HTMLSelectElement).value,
                            }),
                          )}
                      >
                        <option value="open">open</option>
                        <option value="commons">commons</option>
                        <option value="held">held</option>
                      </select>
                    </div>
                    <input
                      type="text"
                      placeholder="Holder org"
                      value={blk.holder}
                      oninput={(e) =>
                        commitDef(
                          updatePortSiteBlock(activeEntry.def, blk.id, {
                            holder: (e.currentTarget as HTMLInputElement).value,
                          }),
                        )}
                    />
                  </div>
                {/each}
              </div>
            </div>
          {/if}

          <details class="raw-drawer">
            <summary>Advanced Port Sub-JSON (rooms, programme, vehicles)</summary>
            <label class="sub-editor">
              <span>Port Room Role Bindings (<code>rooms</code>)</span>
              <textarea
                rows="4"
                value={JSON.stringify(activeEntry.def.rooms ?? {}, null, 2)}
                onchange={(e) => patchSubJson("rooms", (e.currentTarget as HTMLTextAreaElement).value)}
              ></textarea>
            </label>
          </details>
        {:else if activeEntry.kind === "route"}
          <div class="field-grid">
            <label>
              <span>Origin Location</span>
              <input
                type="text"
                value={String(activeEntry.def.origin ?? activeEntry.def.from ?? "")}
                oninput={(e) =>
                  patchField(
                    activeEntry.def.from !== undefined ? "from" : "origin",
                    (e.currentTarget as HTMLInputElement).value,
                  )}
              />
            </label>
            <label>
              <span>Destination Location</span>
              <input
                type="text"
                value={String(activeEntry.def.destination ?? activeEntry.def.to ?? "")}
                oninput={(e) =>
                  patchField(
                    activeEntry.def.to !== undefined ? "to" : "destination",
                    (e.currentTarget as HTMLInputElement).value,
                  )}
              />
            </label>
            <label>
              <span>Duration (seconds)</span>
              <input
                type="number"
                value={Number(activeEntry.def.duration_seconds ?? 0)}
                oninput={(e) =>
                  patchField("duration_seconds", Number((e.currentTarget as HTMLInputElement).value))}
              />
            </label>
          </div>
          <label class="sub-editor">
            <span>Legs / Waypoints JSON</span>
            <textarea
              rows="5"
              value={JSON.stringify(activeEntry.def.legs ?? activeEntry.def.waypoints ?? [], null, 2)}
              onchange={(e) =>
                patchSubJson(
                  activeEntry.def.legs !== undefined ? "legs" : "waypoints",
                  (e.currentTarget as HTMLTextAreaElement).value,
                )}
            ></textarea>
          </label>
        {:else if activeEntry.kind === "location"}
          <div class="field-grid">
            <label>
              <span>Role (<code>station | body | extraction</code>)</span>
              <input
                type="text"
                value={String(activeEntry.def.role ?? "")}
                oninput={(e) => patchField("role", (e.currentTarget as HTMLInputElement).value)}
              />
            </label>
            <label>
              <span>Region / Body</span>
              <input
                type="text"
                value={String(activeEntry.def.region ?? activeEntry.def.body ?? "")}
                oninput={(e) =>
                  patchField(
                    activeEntry.def.body !== undefined ? "body" : "region",
                    (e.currentTarget as HTMLInputElement).value,
                  )}
              />
            </label>
          </div>
        {:else}
          <!-- Relays (`network` nodes & pairs, `private_pair` manufacture recipe, or `sol` region) -->
          {#if relayNodes.length > 0 || relayPairs.length > 0}
            <div class="builder-section">
              <div class="section-head">
                <strong>Relay Nodes ({relayNodes.length})</strong>
              </div>
              {#each relayNodes as node (node.id)}
                <div class="sub-card">
                  <div class="field-grid">
                    <label>
                      <span>Node ID (<code>{node.id}</code>) Label</span>
                      <input
                        type="text"
                        value={node.label}
                        oninput={(e) =>
                          commitDef(
                            updateRelayNode(activeEntry.def, node.id, {
                              label: (e.currentTarget as HTMLInputElement).value,
                            }),
                          )}
                      />
                    </label>
                    <label>
                      <span>Place Pack</span>
                      <input
                        type="text"
                        value={node.placePack}
                        oninput={(e) =>
                          commitDef(
                            updateRelayNode(activeEntry.def, node.id, {
                              placePack: (e.currentTarget as HTMLInputElement).value,
                            }),
                          )}
                      />
                    </label>
                    <label>
                      <span>Place Room Entry</span>
                      <input
                        type="text"
                        value={node.placeEntry}
                        oninput={(e) =>
                          commitDef(
                            updateRelayNode(activeEntry.def, node.id, {
                              placeEntry: (e.currentTarget as HTMLInputElement).value,
                            }),
                          )}
                      />
                    </label>
                  </div>
                </div>
              {/each}
            </div>

            <div class="builder-section">
              <div class="section-head">
                <strong>Entangled Relay Pairs ({relayPairs.length})</strong>
              </div>
              {#each relayPairs as pair (pair.id)}
                <div class="sub-card">
                  <div class="field-grid">
                    <label>
                      <span>Pair <code>{pair.id}</code> Capacity (u/IC s)</span>
                      <input
                        type="number"
                        value={pair.capacityUnitsPerIcSecond}
                        oninput={(e) =>
                          commitDef(
                            updateRelayPair(activeEntry.def, pair.id, {
                              capacityUnitsPerIcSecond: Number(
                                (e.currentTarget as HTMLInputElement).value,
                              ),
                            }),
                          )}
                      />
                    </label>
                    <label>
                      <span>Endpoint Node A</span>
                      <input
                        type="text"
                        value={pair.nodeA}
                        oninput={(e) =>
                          commitDef(
                            updateRelayPair(activeEntry.def, pair.id, {
                              nodeA: (e.currentTarget as HTMLInputElement).value,
                            }),
                          )}
                      />
                    </label>
                    <label>
                      <span>Endpoint Node B</span>
                      <input
                        type="text"
                        value={pair.nodeB}
                        oninput={(e) =>
                          commitDef(
                            updateRelayPair(activeEntry.def, pair.id, {
                              nodeB: (e.currentTarget as HTMLInputElement).value,
                            }),
                          )}
                      />
                    </label>
                  </div>
                </div>
              {/each}
            </div>
          {:else if relayRecipe}
            <div class="builder-section">
              <div class="section-head">
                <strong>Manufacture Recipe (<code>{activeEntry.id}</code>)</strong>
              </div>
              <div class="field-grid">
                <label>
                  <span>Recipe Label</span>
                  <input
                    type="text"
                    value={relayRecipe.label}
                    oninput={(e) =>
                      commitDef(
                        updateRelayRecipe(activeEntry.def, {
                          label: (e.currentTarget as HTMLInputElement).value,
                        }),
                      )}
                  />
                </label>
                <label>
                  <span>Work Units</span>
                  <input
                    type="number"
                    value={relayRecipe.workUnits}
                    oninput={(e) =>
                      commitDef(
                        updateRelayRecipe(activeEntry.def, {
                          workUnits: Number((e.currentTarget as HTMLInputElement).value),
                        }),
                      )}
                  />
                </label>
              </div>
            </div>
          {:else}
            <label class="sub-editor">
              <span>Entry Definition (<code>{activeEntry.id}</code>)</span>
              <textarea
                rows="8"
                value={JSON.stringify(activeEntry.def, null, 2)}
                onchange={(e) => {
                  if (!documentJson) return;
                  try {
                    const parsed = JSON.parse((e.currentTarget as HTMLTextAreaElement).value);
                    onChange(updateCorridorEntryDef(documentJson, activeEntry.id, parsed));
                  } catch {
                    // Wait for valid JSON
                  }
                }}
              ></textarea>
            </label>
          {/if}
        {/if}
      </div>
    {/if}
  </div>
</div>

<style>
  .corridor-studio {
    display: flex;
    flex-direction: column;
    border-bottom: 1px solid var(--border, #2a2f3a);
    max-height: 70%;
    min-height: 320px;
  }

  .corridor-filter-bar {
    display: flex;
    gap: 0.35rem;
    padding: 0.45rem 0.65rem;
    border-bottom: 1px solid var(--border, #2a2f3a);
    flex-wrap: wrap;
  }

  .filter-pill {
    font-size: 11px;
    padding: 0.16rem 0.55rem;
    border-radius: 999px;
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid var(--border, #2a2f3a);
    color: var(--muted, #8892b0);
    cursor: pointer;
  }

  .filter-pill.active {
    color: var(--accent, #88c0d0);
    border-color: var(--accent, #88c0d0);
    background: rgba(136, 192, 208, 0.12);
  }

  .corridor-columns {
    display: grid;
    grid-template-columns: 270px 1fr;
    flex: 1;
    min-height: 0;
  }

  .entry-list {
    border-right: 1px solid var(--border, #2a2f3a);
    overflow-y: auto;
    padding: 0.45rem;
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }

  .entry-card {
    text-align: left;
    padding: 0.45rem 0.55rem;
    border-radius: 4px;
    background: rgba(255, 255, 255, 0.02);
    border: 1px solid var(--border, #2a2f3a);
    color: inherit;
    cursor: pointer;
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
  }

  .entry-card.active {
    border-color: var(--accent, #88c0d0);
    background: rgba(136, 192, 208, 0.1);
  }

  .row-top {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 0.35rem;
  }

  .row-sub {
    font-size: 11px;
    color: var(--muted, #8892b0);
  }

  .kind-chip,
  .tag-chip {
    font-size: 10px;
    padding: 0.08rem 0.35rem;
    border-radius: 3px;
    background: rgba(136, 192, 208, 0.14);
    color: var(--accent, #88c0d0);
  }

  .entry-inspector {
    padding: 0.65rem;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 0.65rem;
  }

  .inspector-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
  }

  .inspector-head h4 {
    margin: 0;
  }

  .ro-badge {
    font-size: 11px;
    padding: 0.12rem 0.45rem;
    border-radius: 3px;
    background: rgba(235, 203, 139, 0.16);
    color: #ebcb8b;
  }

  .ro-notice {
    padding: 0.45rem 0.6rem;
    background: rgba(235, 203, 139, 0.1);
    border: 1px solid rgba(235, 203, 139, 0.3);
    border-radius: 4px;
    color: #ebcb8b;
  }

  .ro-pre {
    max-height: 220px;
    overflow: auto;
    background: rgba(0, 0, 0, 0.3);
    padding: 0.5rem;
    border-radius: 4px;
    font-size: 11px;
  }

  .field-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
    gap: 0.45rem;
  }

  .field-grid label,
  .sub-editor {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    font-size: 11px;
  }

  .builder-section {
    display: flex;
    flex-direction: column;
    gap: 0.45rem;
    padding: 0.5rem;
    border: 1px solid var(--border, #2a2f3a);
    border-radius: 4px;
    background: rgba(255, 255, 255, 0.015);
  }

  .section-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 0.4rem;
  }

  .sub-card {
    padding: 0.45rem;
    border: 1px solid var(--border, #2a2f3a);
    border-radius: 4px;
    background: rgba(0, 0, 0, 0.2);
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }

  .table-block {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }

  .inline-row {
    display: flex;
    align-items: center;
    gap: 0.35rem;
  }

  .block-grid {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 0.35rem;
  }

  .block-cell {
    padding: 0.35rem;
    border: 1px solid var(--border, #2a2f3a);
    border-radius: 3px;
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }

  input,
  select,
  textarea {
    background: var(--bg, #0f1218);
    color: inherit;
    border: 1px solid var(--border, #2a2f3a);
    border-radius: 3px;
    padding: 0.25rem 0.4rem;
    font-family: monospace;
    font-size: 11px;
  }

  .mini-btn {
    font-size: 11px;
    padding: 0.14rem 0.45rem;
    border-radius: 3px;
    background: rgba(136, 192, 208, 0.12);
    border: 1px solid var(--accent, #88c0d0);
    color: var(--accent, #88c0d0);
    cursor: pointer;
    align-self: flex-start;
  }

  .mini-btn.danger {
    background: rgba(191, 97, 106, 0.14);
    border-color: #bf616a;
    color: #bf616a;
  }

  .raw-drawer {
    font-size: 11px;
    color: var(--muted, #8892b0);
  }
</style>
