<script lang="ts">
  // SF.19 — Space & Hulls Studio (`space/hulls.json`, `doc 31` + `doc 35`):
  // Visual workbench for canonical space hulls, 5-facet armor, weapons/EW
  // loadouts, slot-stable Refit Bay packages, and orbital asteroid clusters.
  import type { JsonValue } from "$lib/proto";
  import type { CatalogData } from "$lib/content/store";
  import {
    ARMOR_FACET_LABELS,
    PIXEL_SHIP_MODELS,
    parseSpaceHullsDoc,
    type SpaceHullSpecDraft,
    type SpaceRefitPackageDraft,
    updateHullSpec,
    updateRefitPackage,
    validateSpaceHullsDoc,
  } from "$lib/content/space-editor";

  let {
    documentJson,
    catalog,
    onChange,
  }: {
    documentJson: JsonValue | null;
    catalog: CatalogData;
    onChange: (nextDoc: JsonValue) => void;
  } = $props();

  type SectionMode = "hulls" | "refits" | "clusters";
  let section = $state<SectionMode>("hulls");
  let selectedHullIdx = $state<number>(0);
  let selectedRefitIdx = $state<number>(0);

  const parsed = $derived(parseSpaceHullsDoc(documentJson));
  const localIssues = $derived(validateSpaceHullsDoc(documentJson));
  const activeHull = $derived<SpaceHullSpecDraft | null>(
    parsed.hulls[selectedHullIdx] ?? parsed.hulls[0] ?? null,
  );
  const activeHullIndex = $derived(
    parsed.hulls[selectedHullIdx] ? selectedHullIdx : 0,
  );
  const activeRefit = $derived<SpaceRefitPackageDraft | null>(
    parsed.refit_packages[selectedRefitIdx] ?? parsed.refit_packages[0] ?? null,
  );
  const activeRefitIndex = $derived(
    parsed.refit_packages[selectedRefitIdx] ? selectedRefitIdx : 0,
  );

  function patchHull(patch: Partial<SpaceHullSpecDraft>): void {
    onChange(updateHullSpec(documentJson, activeHullIndex, patch));
  }

  function patchArmorFacet(facetIdx: number, val: number): void {
    if (!activeHull) return;
    const nextArmor: [number, number, number, number, number] = [
      ...activeHull.hull_armor,
    ];
    nextArmor[facetIdx] = Math.max(0, Math.trunc(val));
    patchHull({ hull_armor: nextArmor });
  }

  function patchRefit(patch: Partial<SpaceRefitPackageDraft>): void {
    onChange(updateRefitPackage(documentJson, activeRefitIndex, patch));
  }
</script>

<div class="space-studio" aria-label="Space and Hulls Studio">
  <div class="section-bar" role="tablist" aria-label="Space Studio sections">
    <button
      type="button"
      role="tab"
      aria-selected={section === "hulls"}
      class="sec-pill"
      class:active={section === "hulls"}
      onclick={() => (section = "hulls")}
    >
      Space Hulls ({parsed.hulls.length})
    </button>
    <button
      type="button"
      role="tab"
      aria-selected={section === "refits"}
      class="sec-pill"
      class:active={section === "refits"}
      onclick={() => (section = "refits")}
    >
      Slot-Stable Refits ({parsed.refit_packages.length})
    </button>
    <button
      type="button"
      role="tab"
      aria-selected={section === "clusters"}
      class="sec-pill"
      class:active={section === "clusters"}
      onclick={() => (section = "clusters")}
    >
      Orbital Clusters &amp; Outer Bodies ({catalog.space_clusters.length})
    </button>
  </div>

  {#if localIssues.length > 0}
    <div class="local-warn" role="status">
      <strong>Refit / Hull schema notice:</strong>
      <ul>
        {#each localIssues as issue, idx (idx)}
          <li>{issue}</li>
        {/each}
      </ul>
    </div>
  {/if}

  {#if section === "hulls"}
    <div class="workspace">
      <div class="card-list">
        {#each parsed.hulls as hull, i (hull.id)}
          <button
            type="button"
            class="item-card"
            class:selected={activeHullIndex === i}
            onclick={() => (selectedHullIdx = i)}
          >
            <div class="card-top">
              <strong>{hull.label}</strong>
              <span class="tag">{hull.refit_family}</span>
            </div>
            <div class="card-meta">
              <code>{hull.short_id}</code> · {hull.dry_mass_t} t · Δv {hull.delta_v_mps} m/s · sprite <code>{hull.pixel_model}</code>
            </div>
          </button>
        {/each}
      </div>

      {#if activeHull}
        <div class="inspector">
          <h4>{activeHull.label} (<code>{activeHull.id}</code>)</h4>
          <div class="field-grid">
            <label>
              <span>Label</span>
              <input
                type="text"
                value={activeHull.label}
                oninput={(e) => patchHull({ label: (e.currentTarget as HTMLInputElement).value })}
              />
            </label>
            <label>
              <span>Short ID</span>
              <input
                type="text"
                value={activeHull.short_id}
                oninput={(e) => patchHull({ short_id: (e.currentTarget as HTMLInputElement).value })}
              />
            </label>
            <label>
              <span>Refit Family (D-31-14)</span>
              <input
                type="text"
                value={activeHull.refit_family}
                oninput={(e) => patchHull({ refit_family: (e.currentTarget as HTMLInputElement).value })}
              />
            </label>
            <label>
              <span>3D Sprite Model (SF.14)</span>
              <select
                value={activeHull.pixel_model}
                onchange={(e) => patchHull({ pixel_model: (e.currentTarget as HTMLSelectElement).value })}
              >
                {#each PIXEL_SHIP_MODELS as model (model)}
                  <option value={model}>{model}</option>
                {/each}
              </select>
            </label>
            <label>
              <span>Dry Mass (t)</span>
              <input
                type="number"
                value={activeHull.dry_mass_t}
                oninput={(e) => patchHull({ dry_mass_t: Number((e.currentTarget as HTMLInputElement).value) })}
              />
            </label>
            <label>
              <span>Propellant (kg)</span>
              <input
                type="number"
                value={activeHull.propellant_kg}
                oninput={(e) => patchHull({ propellant_kg: Number((e.currentTarget as HTMLInputElement).value) })}
              />
            </label>
            <label>
              <span>Max Accel (m/s²)</span>
              <input
                type="number"
                value={activeHull.max_accel_mps2}
                oninput={(e) => patchHull({ max_accel_mps2: Number((e.currentTarget as HTMLInputElement).value) })}
              />
            </label>
            <label>
              <span>Delta-V Budget (m/s)</span>
              <input
                type="number"
                value={activeHull.delta_v_mps}
                oninput={(e) => patchHull({ delta_v_mps: Number((e.currentTarget as HTMLInputElement).value) })}
              />
            </label>
            <label>
              <span>Return Reserve Δv (m/s)</span>
              <input
                type="number"
                value={activeHull.return_reserve_dv_mps}
                oninput={(e) => patchHull({ return_reserve_dv_mps: Number((e.currentTarget as HTMLInputElement).value) })}
              />
            </label>
            <label>
              <span>Cooling Rate (‰/s)</span>
              <input
                type="number"
                value={activeHull.cooling_rate_permille_s}
                oninput={(e) => patchHull({ cooling_rate_permille_s: Number((e.currentTarget as HTMLInputElement).value) })}
              />
            </label>
            <label>
              <span>Hit Radius (m)</span>
              <input
                type="number"
                value={activeHull.hit_radius_m}
                oninput={(e) => patchHull({ hit_radius_m: Number((e.currentTarget as HTMLInputElement).value) })}
              />
            </label>
            <label>
              <span>ECM Decoys</span>
              <input
                type="number"
                value={activeHull.decoys}
                oninput={(e) => patchHull({ decoys: Number((e.currentTarget as HTMLInputElement).value) })}
              />
            </label>
            <label>
              <span>Section Internals (Hull / Hold / Drive)</span>
              <div class="inline-triple">
                <input
                  type="number"
                  aria-label="Hull internals"
                  value={activeHull.hull_internals}
                  oninput={(e) => patchHull({ hull_internals: Number((e.currentTarget as HTMLInputElement).value) })}
                />
                <input
                  type="number"
                  aria-label="Hold internals"
                  value={activeHull.hold_internals}
                  oninput={(e) => patchHull({ hold_internals: Number((e.currentTarget as HTMLInputElement).value) })}
                />
                <input
                  type="number"
                  aria-label="Drive internals"
                  value={activeHull.drive_internals}
                  oninput={(e) => patchHull({ drive_internals: Number((e.currentTarget as HTMLInputElement).value) })}
                />
              </div>
            </label>
            <label>
              <span>Laser Turrets / Railgun Slugs</span>
              <div class="inline-pair">
                <input
                  type="number"
                  aria-label="Laser turrets"
                  value={activeHull.laser_turrets}
                  oninput={(e) => patchHull({ laser_turrets: Number((e.currentTarget as HTMLInputElement).value) })}
                />
                <input
                  type="number"
                  aria-label="Railgun rounds"
                  value={activeHull.railgun_rounds}
                  oninput={(e) => patchHull({ railgun_rounds: Number((e.currentTarget as HTMLInputElement).value) })}
                />
              </div>
            </label>
            <label>
              <span>Torpedo Tubes / Mag</span>
              <div class="inline-pair">
                <input
                  type="number"
                  aria-label="Torpedo tubes"
                  value={activeHull.torpedo_tubes}
                  oninput={(e) => patchHull({ torpedo_tubes: Number((e.currentTarget as HTMLInputElement).value) })}
                />
                <input
                  type="number"
                  aria-label="Torpedo magazine"
                  value={activeHull.torpedo_magazine}
                  oninput={(e) => patchHull({ torpedo_magazine: Number((e.currentTarget as HTMLInputElement).value) })}
                />
              </div>
            </label>
            <label>
              <span>PDC Mounts / Ammo</span>
              <div class="inline-pair">
                <input
                  type="number"
                  aria-label="PDC mounts"
                  value={activeHull.pdc_mounts}
                  oninput={(e) => patchHull({ pdc_mounts: Number((e.currentTarget as HTMLInputElement).value) })}
                />
                <input
                  type="number"
                  aria-label="PDC ammo"
                  value={activeHull.pdc_ammo}
                  oninput={(e) => patchHull({ pdc_ammo: Number((e.currentTarget as HTMLInputElement).value) })}
                />
              </div>
            </label>
          </div>

          <h5>5-Facet Directional Hull Armor (Front / Left / Right / Rear / Top)</h5>
          <div class="armor-grid">
            {#each ARMOR_FACET_LABELS as label, fIdx (label)}
              <label>
                <span>{label}</span>
                <input
                  type="number"
                  aria-label="{label} armor"
                  value={activeHull.hull_armor[fIdx]}
                  oninput={(e) => patchArmorFacet(fIdx, Number((e.currentTarget as HTMLInputElement).value))}
                />
              </label>
            {/each}
          </div>
        </div>
      {/if}
    </div>
  {:else if section === "refits"}
    <div class="workspace">
      <div class="card-list">
        {#each parsed.refit_packages as pkg, i (pkg.id)}
          <button
            type="button"
            class="item-card"
            class:selected={activeRefitIndex === i}
            onclick={() => (selectedRefitIdx = i)}
          >
            <div class="card-top">
              <strong>{pkg.label}</strong>
              <span class="tag">{pkg.refit_family}</span>
            </div>
            <div class="card-meta">
              <code>{pkg.id}</code> → <code>{pkg.target_hull_id}</code> · {pkg.cost_cr} cr · {pkg.work_hours} h
            </div>
          </button>
        {/each}
      </div>

      {#if activeRefit}
        <div class="inspector">
          <h4>{activeRefit.label} (<code>{activeRefit.id}</code>)</h4>
          <div class="field-grid">
            <label>
              <span>Label</span>
              <input
                type="text"
                value={activeRefit.label}
                oninput={(e) => patchRefit({ label: (e.currentTarget as HTMLInputElement).value })}
              />
            </label>
            <label>
              <span>Refit Family</span>
              <input
                type="text"
                value={activeRefit.refit_family}
                oninput={(e) => patchRefit({ refit_family: (e.currentTarget as HTMLInputElement).value })}
              />
            </label>
            <label>
              <span>Target Hull ID</span>
              <select
                value={activeRefit.target_hull_id}
                onchange={(e) => patchRefit({ target_hull_id: (e.currentTarget as HTMLSelectElement).value })}
              >
                {#each parsed.hulls as h (h.id)}
                  <option value={h.id}>{h.id} ({h.refit_family})</option>
                {/each}
              </select>
            </label>
            <label>
              <span>Work Order Cost (cr)</span>
              <input
                type="number"
                value={activeRefit.cost_cr}
                oninput={(e) => patchRefit({ cost_cr: Number((e.currentTarget as HTMLInputElement).value) })}
              />
            </label>
            <label>
              <span>Workshop Hours (IC h)</span>
              <input
                type="number"
                value={activeRefit.work_hours}
                oninput={(e) => patchRefit({ work_hours: Number((e.currentTarget as HTMLInputElement).value) })}
              />
            </label>
            <label>
              <span>+Delta-V (m/s)</span>
              <input
                type="number"
                value={activeRefit.add_delta_v_mps}
                oninput={(e) => patchRefit({ add_delta_v_mps: Number((e.currentTarget as HTMLInputElement).value) })}
              />
            </label>
            <label>
              <span>+Propellant (kg)</span>
              <input
                type="number"
                value={activeRefit.add_propellant_kg}
                oninput={(e) => patchRefit({ add_propellant_kg: Number((e.currentTarget as HTMLInputElement).value) })}
              />
            </label>
            <label>
              <span>+Laser Turrets / +Cooling (‰/s)</span>
              <div class="inline-pair">
                <input
                  type="number"
                  aria-label="Add laser turrets"
                  value={activeRefit.add_laser_turrets}
                  oninput={(e) => patchRefit({ add_laser_turrets: Number((e.currentTarget as HTMLInputElement).value) })}
                />
                <input
                  type="number"
                  aria-label="Add cooling rate"
                  value={activeRefit.add_cooling_permille_s}
                  oninput={(e) => patchRefit({ add_cooling_permille_s: Number((e.currentTarget as HTMLInputElement).value) })}
                />
              </div>
            </label>
            <label>
              <span>+Torpedo Mag / +Railgun Slugs</span>
              <div class="inline-pair">
                <input
                  type="number"
                  aria-label="Add torpedo magazine"
                  value={activeRefit.add_torpedo_magazine}
                  oninput={(e) => patchRefit({ add_torpedo_magazine: Number((e.currentTarget as HTMLInputElement).value) })}
                />
                <input
                  type="number"
                  aria-label="Add railgun rounds"
                  value={activeRefit.add_railgun_rounds}
                  oninput={(e) => patchRefit({ add_railgun_rounds: Number((e.currentTarget as HTMLInputElement).value) })}
                />
              </div>
            </label>
            <label>
              <span>+PDC Ammo / +Decoys</span>
              <div class="inline-pair">
                <input
                  type="number"
                  aria-label="Add PDC ammo"
                  value={activeRefit.add_pdc_ammo}
                  oninput={(e) => patchRefit({ add_pdc_ammo: Number((e.currentTarget as HTMLInputElement).value) })}
                />
                <input
                  type="number"
                  aria-label="Add decoys"
                  value={activeRefit.add_decoys}
                  oninput={(e) => patchRefit({ add_decoys: Number((e.currentTarget as HTMLInputElement).value) })}
                />
              </div>
            </label>
          </div>
        </div>
      {/if}
    </div>
  {:else}
    <div class="clusters-pane">
      <h4>Orbital Asteroid Clusters (D-35-10)</h4>
      <div class="cluster-grid">
        {#each catalog.space_clusters as cl (cl.id)}
          <div class="cluster-card">
            <div class="card-top">
              <strong>{cl.title}</strong>
              <span class="tag">{cl.jurisdiction}</span>
            </div>
            <div class="card-meta">
              <code>+helm/enter {cl.id}</code> · {cl.rock_count} seeded C/S/M rocks
            </div>
          </div>
        {/each}
      </div>

      <h4>Outer-System Ephemeris Bodies (Sky::Kepler)</h4>
      <div class="body-pills">
        {#each catalog.outer_bodies as body (body)}
          <span class="body-pill"><code>{body}</code></span>
        {/each}
      </div>
    </div>
  {/if}
</div>

<style>
  .space-studio {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .section-bar {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }

  .sec-pill {
    padding: 4px 10px;
    border-radius: 4px;
    border: 1px solid #3b4252;
    background: #1f2430;
    color: #d8dee9;
    cursor: pointer;
    font-size: 11px;
  }

  .sec-pill.active {
    background: #2e3440;
    border-color: #88c0d0;
    color: #88c0d0;
  }

  .local-warn {
    padding: 8px 10px;
    border-radius: 4px;
    border: 1px solid #ebcb8b;
    background: rgba(235, 203, 139, 0.12);
    color: #ebcb8b;
  }

  .workspace {
    display: grid;
    grid-template-columns: 280px 1fr;
    gap: 12px;
  }

  .card-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .item-card {
    text-align: left;
    padding: 8px 10px;
    border-radius: 4px;
    border: 1px solid #3b4252;
    background: #1f2430;
    color: #d8dee9;
    cursor: pointer;
  }

  .item-card.selected {
    border-color: #88c0d0;
    background: #262c3a;
  }

  .card-top {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 6px;
  }

  .card-meta {
    margin-top: 4px;
    font-size: 11px;
    color: #9aa5b4;
  }

  .tag {
    font-size: 10px;
    padding: 1px 6px;
    border-radius: 3px;
    background: #3b4252;
    color: #88c0d0;
  }

  .inspector {
    padding: 10px 12px;
    border-radius: 4px;
    border: 1px solid #3b4252;
    background: #1b202b;
  }

  .inspector h4,
  .inspector h5 {
    margin: 0 0 8px 0;
  }

  .inspector h5 {
    margin-top: 12px;
  }

  .field-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
    gap: 8px;
  }

  .armor-grid {
    display: grid;
    grid-template-columns: repeat(5, minmax(75px, 1fr));
    gap: 6px;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 3px;
    font-size: 11px;
  }

  .inline-pair {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 4px;
  }

  .inline-triple {
    display: grid;
    grid-template-columns: 1fr 1fr 1fr;
    gap: 4px;
  }

  input[type="text"],
  input[type="number"],
  select {
    padding: 4px 6px;
    border-radius: 3px;
    border: 1px solid #3b4252;
    background: #141821;
    color: #eceff4;
    font-size: 11px;
  }

  .clusters-pane {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .cluster-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
    gap: 8px;
  }

  .cluster-card {
    padding: 8px 10px;
    border-radius: 4px;
    border: 1px solid #3b4252;
    background: #1f2430;
  }

  .body-pills {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .body-pill {
    padding: 3px 8px;
    border-radius: 3px;
    border: 1px solid #3b4252;
    background: #1f2430;
  }
</style>
