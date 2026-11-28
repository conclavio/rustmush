<script lang="ts">
  // The quarry screen (doc 20 cut SM.9; §10, D-20-13): the hull's rigs on the left, the map in
  // the middle — the drive view, the whole site, or the body's ground chart as a 2:1 picture
  // with a flat-map toggle — and the viewer's readings and log (or the ground chart's sites)
  // on the right. Every glyph is the frame's (`$lib/quarry`'s pure helpers, pinned by vitest);
  // the picture colours a glyph and draws a mark, never decides one. The one thing reckoned
  // here is where along its frozen leg the viewer's own moving rig stands now (the lead's 0z
  // ruling 1): a one-second tick re-draws it while a leg is in flight, and pulls nothing.
  //
  // **Click-to-prefill, never submit**: a site cell prefills `+tv/<n> drive <x>,<y>`, a reading
  // `+tv/<n> drive <target>`, a ground site `+ship/plan <venue>` — each into the composer.
  import type { GameApp } from "$lib/game.svelte";
  import {
    glyphColour,
    groundPoint,
    groundPrintRows,
    icNow,
    placedRows,
    rigPrefill,
    siteCellOf,
  } from "$lib/quarry";
  import { expandRows } from "$lib/stores/map.svelte";
  import { extractSurfaceMiningPrepActions } from "$lib/stores/quests.svelte";

  let { app }: { app: GameApp } = $props();

  const q = $derived(app.quarry);
  const site = $derived(q.site);
  const block = $derived(site?.site ?? null);
  const ground = $derived(q.ground);
  const groundBlock = $derived(ground?.ground ?? null);
  const moving = $derived(block?.rigs.some((r) => r.leg !== null) ?? false);
  const questMining = $derived(app.quests?.surfaceMiningStatus ?? null);
  const questMiningPrepActions = $derived(
    extractSurfaceMiningPrepActions(questMining, q.selectedRig ?? 1),
  );
  const questNextRigCmd = $derived.by((): { label: string; verb: string } | null => {
    if (!questMining) return null;
    if (!questMining.scanDone) return { label: "Scan Site", verb: "scan go" };
    if (!questMining.soundDone) return { label: "Sound Pocket", verb: "sound go" };
    if (!questMining.mineDone) return { label: "Mine Ore", verb: "mine go" };
    return { label: "Recover Rig", verb: "recover go" };
  });

  function matchesQuestOre(text: string | null | undefined): boolean {
    if (!questMining || !text) return false;
    const low = text.toLowerCase();
    const slug = questMining.targetOreSlug.toLowerCase();
    const spaced = slug.replace(/_/g, " ");
    const fam = questMining.recommendedFamily.toLowerCase();
    return low.includes(slug) || low.includes(spaced) || (fam.length > 0 && low.includes(fam));
  }

  function matchesQuestSite(s: { best?: string | null; best_line?: string | null }): boolean {
    return matchesQuestOre(s.best_line) || matchesQuestOre(s.best);
  }

  const recommendedQuestSite = $derived.by(() => {
    if (!questMining || questMining.mineDone || !groundBlock) return null;
    const matched = groundBlock.sites.find((s) => !s.here && matchesQuestSite(s));
    if (matched) return matched;
    // Before the player has surface readings at any site, guide them to the open Ceres
    // starter sites named in Step 6 (Freeport Commons / Ahuna Flank), or first open site.
    return (
      groundBlock.sites.find((s) => !s.here && s.venue.toLowerCase() === "commons") ??
      groundBlock.sites.find((s) => !s.here && s.venue.toLowerCase() === "ahuna") ??
      groundBlock.sites.find((s) => !s.here && s.tenure.toLowerCase().includes("open")) ??
      null
    );
  });

  const questReturnDest = $derived(
    questMining?.mineDone ? (app.quests?.activeDestinationPort ?? null) : null,
  );

  function isQuestReadingMatch(
    r: { target: string; line: string },
    family: string,
  ): boolean {
    if (!questMining || questMining.mineDone) return false;
    return (
      matchesQuestOre(r.line) ||
      family.toLowerCase() === questMining.recommendedFamily.toLowerCase()
    );
  }

  const orderedReadings = $derived.by(() => {
    if (!block) return [];
    if (!questMining || questMining.mineDone) return block.readings;
    const direct: typeof block.readings = [];
    const familyOnly: typeof block.readings = [];
    const rest: typeof block.readings = [];
    const famMatch = block.family.toLowerCase() === questMining.recommendedFamily.toLowerCase();
    for (const r of block.readings) {
      if (matchesQuestOre(r.line)) direct.push(r);
      else if (famMatch) familyOnly.push(r);
      else rest.push(r);
    }
    return [...direct, ...familyOnly, ...rest];
  });

  const firstQuestReading = $derived.by(() => {
    if (!questMining || questMining.mineDone || !block || q.mode === "ground") return null;
    return orderedReadings.find((r) => isQuestReadingMatch(r, block.family)) ?? null;
  });

  function isHighlightedQuestSite(s: {
    venue: string;
    best?: string | null;
    best_line?: string | null;
  }): boolean {
    if (!questMining || questMining.mineDone) return false;
    return (
      matchesQuestSite(s) ||
      (recommendedQuestSite !== null && recommendedQuestSite.venue === s.venue)
    );
  }

  const orderedGroundSites = $derived.by(() => {
    if (!groundBlock) return [];
    if (!questMining || questMining.mineDone) return groundBlock.sites;
    const direct: typeof groundBlock.sites = [];
    const recommended: typeof groundBlock.sites = [];
    const rest: typeof groundBlock.sites = [];
    for (const s of groundBlock.sites) {
      if (matchesQuestSite(s)) direct.push(s);
      else if (recommendedQuestSite !== null && recommendedQuestSite.venue === s.venue) {
        recommended.push(s);
      } else {
        rest.push(s);
      }
    }
    return [...direct, ...recommended, ...rest];
  });

  function selectQuestWash(): void {
    if (!questMining) return;
    app.quarry.family = questMining.recommendedFamily;
    app.refreshQuarry();
  }

  function openQuestGroundChart(): void {
    if (!questMining) return;
    app.quarry.mode = "ground";
    app.quarry.family = questMining.recommendedFamily;
    app.refreshQuarry();
  }

  /** The paint clock (ms): ticks each second while one of the viewer's rigs has a leg. */
  let now = $state(Date.now());
  $effect(() => {
    if (!moving) {
      return;
    }
    const timer = setInterval(() => {
      now = Date.now();
    }, 1000);
    return () => clearInterval(timer);
  });

  let flat = $state(false);

  /** The site rows, row 0 the south edge, the viewer's rigs placed at the reckoned instant. */
  const rows = $derived(site && block ? placedRows(site, icNow(block, q.siteAt, now)) : []);
  const CELL = 14;
  const SQUARES = new Set([",", "^", "=", "_", " ", "O"]);

  /** The ground's 2:1 picture: the 56 × 28 provinces, each as tall as it is wide in degrees,
   * north at the top (the grid's row 0 is the south edge). */
  const GW = 896;
  const GH = 448;
  const groundRows = $derived(ground ? expandRows({ ...ground.grid, units: [] }) : []);
  const flatRows = $derived(
    groundBlock && ground ? groundPrintRows(groundBlock, ground.grid.height) : [],
  );

  function prefill(line: string): void {
    app.focusComposer(line);
  }

  function clickCell(fx: number, fy: number): void {
    if (!block) {
      return;
    }
    const cell = siteCellOf(block, fx, fy);
    if (cell) {
      prefill(rigPrefill(q.selectedRig, `drive ${cell[0]},${cell[1]}`));
    }
  }

  function pickRig(slot: number): void {
    app.quarry.rig = slot;
    app.refreshQuarry();
  }
</script>

<div class="quarry">
  <div class="rigs">
    <h3>rigs</h3>
    {#if !block}
      <p class="muted">{site?.grid.title || (q.refused ? "The map the server sent did not read." : "No map yet: r asks the server.")}</p>
    {:else}
      <ul>
        {#each block.rigs as r (r.slot)}
          <li>
            <button type="button" class="rig" class:on={q.selectedRig === r.slot} onclick={() => pickRig(r.slot)}>
              {r.line}
            </button>
            {#if r.run}<div class="sub">{r.run.text}</div>{/if}
            {#if r.queued}<div class="sub">{r.queued}</div>{/if}
            {#if r.plan}<div class="sub">{r.plan.text}</div>{/if}
          </li>
        {/each}
      </ul>
      {#if block.hold}<p class="muted">{block.hold.line}</p>{/if}
    {/if}
  </div>

  <div class="map">
    {#if questMining}
      <div class="quest-mining-pill" role="status" aria-label="quest mining checklist">
        <div class="qm-head">
          <span class="qm-tag">[QUEST]</span>
          <strong>{questMining.questTitle}</strong>
          {#if questNextRigCmd}
            <button
              type="button"
              class="qm-action"
              onclick={() => prefill(rigPrefill(q.selectedRig, questNextRigCmd.verb))}
            >
              Next: {questNextRigCmd.label}
            </button>
          {/if}
          {#if firstQuestReading}
            <button
              type="button"
              class="qm-plan"
              title={`Prefill ${rigPrefill(q.selectedRig, `drive ${firstQuestReading.target}`)}`}
              onclick={() => prefill(rigPrefill(q.selectedRig, `drive ${firstQuestReading.target}`))}
            >
              Drive: {firstQuestReading.target}
            </button>
          {/if}
          {#if questReturnDest}
            <button
              type="button"
              class="qm-return"
              title={`Prefill +ship/plan ${questReturnDest.portId}`}
              onclick={() => prefill(`+ship/plan ${questReturnDest.portId}`)}
            >
              Return: {questReturnDest.portLabel}
            </button>
          {:else if recommendedQuestSite}
            <button
              type="button"
              class="qm-plan"
              title={`Prefill +ship/plan ${recommendedQuestSite.venue}`}
              onclick={() => prefill(`+ship/plan ${recommendedQuestSite.venue}`)}
            >
              Plan: {recommendedQuestSite.site}
            </button>
          {:else if q.mode !== "ground" && !block}
            <button
              type="button"
              class="qm-plan"
              onclick={openQuestGroundChart}
            >
              View Ground
            </button>
          {/if}
          <button type="button" class="qm-wash" onclick={selectQuestWash}>
            {questMining.recommendedFamily.toUpperCase()} wash
          </button>
        </div>
        <div class="qm-steps">
          <span class:done={questMining.scanDone}>{questMining.scanDone ? "[✓]" : "[ ]"} Scan Site</span>
          <span class:done={questMining.soundDone}>{questMining.soundDone ? "[✓]" : "[ ]"} Sound Pocket</span>
          <span class:done={questMining.mineDone}>{questMining.mineDone ? "[✓]" : "[ ]"} Mine {questMining.targetOreSlug}</span>
        </div>
        {#if questMiningPrepActions.length > 0}
          <div class="qm-prep" aria-label="quest prospecting prep actions">
            <span class="qm-prep-label">Prep:</span>
            {#each questMiningPrepActions as act (act.command)}
              <button
                type="button"
                class="qm-prep-btn"
                title={`Prefill ${act.command}`}
                onclick={() => prefill(act.command)}
              >
                {act.label}
              </button>
            {/each}
          </div>
        {/if}
      </div>
    {/if}
    {#if q.mode === "ground"}
      {#if !ground || !groundBlock}
        <p class="muted pad">{ground?.grid.title || "No ground chart yet: r asks the server."}</p>
      {:else}
        <h2>{ground.grid.title}</h2>
        {#if flat}
          <pre class="flat" aria-label="the flat ground chart">{flatRows.join("\n")}</pre>
        {:else}
          <svg viewBox="0 0 {GW} {GH}" role="img" aria-label="the ground chart">
            {#each groundRows as row, y (y)}
              {#each row as glyph, x (x)}
                <rect
                  x={(x * GW) / ground.grid.width}
                  y={((ground.grid.height - 1 - y) * GH) / ground.grid.height}
                  width={GW / ground.grid.width}
                  height={GH / ground.grid.height}
                  fill={glyphColour(glyph)}
                />
              {/each}
            {/each}
            {#each groundBlock.ports as p (p.glyph)}
              {@const at = groundPoint(p, GW, GH)}
              <circle cx={at.x} cy={at.y} r="6" fill="#f2c94c" />
              <text x={at.x + 8} y={at.y - 6} fill="#f2c94c" font-size="13">{p.glyph} {p.port}</text>
            {/each}
            {#each groundBlock.sites as s (s.letter)}
              {@const at = groundPoint(s, GW, GH)}
              {@const isQuestSite = isHighlightedQuestSite(s)}
              {#if isQuestSite}
                <circle cx={at.x} cy={at.y} r="8" fill="none" stroke="#f2c94c" stroke-width="2" />
              {/if}
              <circle cx={at.x} cy={at.y} r="4" fill={s.here ? "#ffffff" : isQuestSite ? "#f2c94c" : "#9fc4d8"} />
              <text
                x={at.x + 6}
                y={at.y + 14}
                fill={isQuestSite ? "#f2c94c" : "#9fc4d8"}
                font-size="12"
                role="button"
                tabindex="0"
                onclick={() => prefill(`+ship/plan ${s.venue}`)}
                onkeydown={(e) => e.key === "Enter" && prefill(`+ship/plan ${s.venue}`)}
              >{s.letter} {s.site}{isQuestSite ? " [!]" : ""}</text>
            {/each}
          </svg>
        {/if}
        <div class="toggles">
          <button type="button" class:on={!flat} onclick={() => (flat = false)}>picture</button>
          <button type="button" class:on={flat} onclick={() => (flat = true)}>flat map</button>
          <span class="muted">{groundBlock.footer}</span>
        </div>
        <p class="legend muted">{ground.grid.legend.map(([g, l]) => `${g} ${l}`).join("  ")}</p>
      {/if}
    {:else if !site || !block}
      <p class="muted pad">{site?.grid.title || "No map yet: r asks the server."}</p>
      {#if site}<p class="muted pad">z: the ground chart</p>{/if}
    {:else}
      <h2>{site.grid.title}</h2>
      <svg
        viewBox="0 0 {site.grid.width * CELL} {site.grid.height * CELL}"
        role="img"
        aria-label="the site map"
      >
        {#each rows as row, y (y)}
          {#each row as glyph, x (x)}
            {@const top = (site.grid.height - 1 - y) * CELL}
            <rect
              x={x * CELL}
              y={top}
              width={CELL}
              height={CELL}
              fill={glyphColour(SQUARES.has(glyph) ? glyph : ",")}
              role="button"
              tabindex="-1"
              onclick={() => clickCell(x, y)}
              onkeydown={(e) => e.key === "Enter" && clickCell(x, y)}
            />
            {#if !SQUARES.has(glyph)}
              <text
                x={x * CELL + CELL / 2}
                y={top + CELL - 3}
                text-anchor="middle"
                font-size="11"
                fill={glyphColour(glyph)}
                pointer-events="none">{glyph}</text>
            {/if}
          {/each}
        {/each}
        {#each block.marks.letters as [fx, fy, letter] (`${fx},${fy}`)}
          <rect
            x={fx * CELL}
            y={(site.grid.height - 1 - fy) * CELL}
            width={CELL}
            height={CELL}
            fill="none"
            stroke="#5c6670"
            stroke-dasharray="2 2"
            pointer-events="none"
          ><title>block {letter}</title></rect>
        {/each}
        {#if block.marks.hull}
          <rect
            x={block.marks.hull[0] * CELL - 2}
            y={(site.grid.height - 1 - block.marks.hull[1]) * CELL - 2}
            width={CELL + 4}
            height={CELL + 4}
            fill="none"
            stroke="#f2c94c"
            stroke-width="2"
            pointer-events="none"
          ><title>your hull</title></rect>
        {/if}
      </svg>
      <p class="legend muted">{site.grid.legend.map(([g, l]) => `${g} ${l}`).join("  ")}</p>
    {/if}
  </div>

  <div class="side">
    {#if q.mode === "ground"}
      <h3>sites</h3>
      {#if groundBlock}
        <ul>
          {#each orderedGroundSites as s (s.letter)}
            {@const isQuestSite = isHighlightedQuestSite(s)}
            <li>
              <button
                type="button"
                class="row"
                class:quest-match={isQuestSite}
                onclick={() => prefill(`+ship/plan ${s.venue}`)}
              >
                {#if isQuestSite}<span class="qm-badge">[QUEST]</span>{" "}{/if}{s.letter} {s.site}{s.passage ? `: ${s.passage}` : ""}
              </button>
              <div class="sub">{s.best_line}</div>
            </li>
          {/each}
        </ul>
        <pre class="lines">{groundBlock.lines.join("\n")}</pre>
      {/if}
    {:else if block}
      <h3>readings, {block.family}</h3>
      {#if block.readings.length === 0}
        {#if block.no_readings}<p class="muted">{block.no_readings}</p>{/if}
      {:else}
        <ul>
          {#each orderedReadings as r, i (i)}
            {@const isQuestReading = isQuestReadingMatch(r, block.family)}
            <li>
              <button
                type="button"
                class="row"
                class:quest-match={isQuestReading}
                onclick={() => prefill(rigPrefill(q.selectedRig, `drive ${r.target}`))}
              >
                {#if isQuestReading}<span class="qm-badge">[QUEST]</span>{" "}{/if}{r.line}
              </button>
            </li>
          {/each}
        </ul>
      {/if}
      {#if block.other_rigs}<p class="muted other-rigs">{block.other_rigs}</p>{/if}
      {#if block.presence_line}<p class="muted presence">{block.presence_line}</p>{/if}
      <h3>log</h3>
      <ul>
        {#each block.log as l, i (i)}
          <li class="log">{l.text}</li>
        {/each}
      </ul>
    {/if}
  </div>
</div>

<style>
  .quarry {
    display: flex;
    gap: 0.75rem;
    padding: 0.5rem 0.75rem;
    height: 100%;
    box-sizing: border-box;
    overflow: auto;
  }
  .rigs {
    flex: 0 0 16rem;
  }
  .map {
    flex: 1 1 32rem;
    min-width: 18rem;
  }
  .side {
    flex: 0 0 18rem;
  }
  h2 {
    font-size: 1rem;
    margin: 0 0 0.5rem;
  }
  h3 {
    font-size: 0.85rem;
    margin: 0.5rem 0 0.25rem;
    color: var(--muted, #9aa);
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  li {
    margin: 0.15rem 0;
  }
  button.rig,
  button.row {
    background: none;
    border: 1px solid transparent;
    color: inherit;
    text-align: left;
    font: inherit;
    cursor: pointer;
    padding: 0.1rem 0.25rem;
    width: 100%;
  }
  button.rig.on {
    border-color: var(--accent, #6b9fd4);
    font-weight: 600;
  }
  .sub {
    padding-left: 1rem;
    font-size: 0.85rem;
    color: var(--muted, #9aa);
  }
  .muted {
    color: var(--muted, #9aa);
  }
  .pad {
    padding: 0.5rem;
  }
  svg {
    width: 100%;
    height: auto;
    background: #0e1116;
  }
  .flat,
  .lines {
    font-family: var(--mono, monospace);
    font-size: 0.75rem;
    white-space: pre;
    overflow: auto;
  }
  .legend {
    font-size: 0.8rem;
  }
  .toggles {
    display: flex;
    gap: 0.5rem;
    align-items: center;
    margin-top: 0.25rem;
  }
  .toggles button.on {
    font-weight: 600;
  }
  .log {
    font-size: 0.85rem;
  }
  .quest-mining-pill {
    margin-bottom: 0.5rem;
    padding: 0.45rem 0.65rem;
    border-radius: 6px;
    border: 1px solid #d4a634;
    background: rgba(58, 44, 12, 0.88);
    color: #fff2c4;
    font-size: 0.82rem;
  }
  .qm-head {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    flex-wrap: wrap;
    margin-bottom: 0.2rem;
  }
  .qm-tag {
    color: #ffd84d;
    font-weight: 700;
    font-family: var(--mono, monospace);
  }
  .qm-wash {
    margin-left: auto;
    padding: 0.15rem 0.5rem;
    border-radius: 4px;
    border: 1px solid #f2c94c;
    background: #6c4c12;
    color: #fff;
    font: inherit;
    font-size: 0.75rem;
    font-weight: 600;
    cursor: pointer;
  }
  .qm-action {
    margin-left: auto;
    padding: 0.15rem 0.55rem;
    border-radius: 4px;
    border: 1px solid #ffd84d;
    background: #b88416;
    color: #161103;
    font: inherit;
    font-size: 0.75rem;
    font-weight: 700;
    cursor: pointer;
  }
  .qm-action + .qm-wash,
  .qm-plan + .qm-wash,
  .qm-return + .qm-wash {
    margin-left: 0;
  }
  .qm-plan {
    padding: 0.15rem 0.55rem;
    border-radius: 4px;
    border: 1px solid #ffd84d;
    background: #8a6412;
    color: #fff6d4;
    font: inherit;
    font-size: 0.75rem;
    font-weight: 700;
    cursor: pointer;
  }
  .qm-return {
    padding: 0.15rem 0.55rem;
    border-radius: 4px;
    border: 1px solid #34d399;
    background: #0d6e4c;
    color: #e6fff5;
    font: inherit;
    font-size: 0.75rem;
    font-weight: 700;
    cursor: pointer;
  }
  button.row.quest-match {
    border-color: rgba(242, 201, 76, 0.75);
    background: rgba(94, 70, 16, 0.45);
    color: #fff2c4;
    border-radius: 4px;
  }
  .qm-badge {
    color: #ffd84d;
    font-weight: 700;
    font-family: var(--mono, monospace);
    font-size: 0.74rem;
  }
  .qm-steps {
    display: flex;
    gap: 0.85rem;
    flex-wrap: wrap;
    font-family: var(--mono, monospace);
    font-size: 0.78rem;
  }
  .qm-steps .done {
    color: #6ee7a8;
  }
  .qm-prep {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    flex-wrap: wrap;
    margin-top: 0.3rem;
    padding-top: 0.25rem;
    border-top: 1px solid rgba(212, 166, 52, 0.35);
    font-size: 0.74rem;
  }
  .qm-prep-label {
    color: #ffd84d;
    font-weight: 700;
    font-family: var(--mono, monospace);
  }
  .qm-prep-btn {
    padding: 0.1rem 0.45rem;
    border-radius: 4px;
    border: 1px solid rgba(242, 201, 76, 0.65);
    background: rgba(88, 64, 14, 0.72);
    color: #fff6d4;
    font: inherit;
    font-size: 0.72rem;
    font-weight: 600;
    cursor: pointer;
  }
  .qm-prep-btn:hover {
    background: rgba(132, 96, 20, 0.85);
  }
</style>
