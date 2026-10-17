<script lang="ts">
  import { proveCommand, benchLoadCommand } from "$lib/mind-bulk";
  import { benchStepCommand, benchTimeCommand, benchExpressionCommand, benchLoadMatchCommand, benchCandidateCommand, benchStationCommand, benchActAsCommand } from "$lib/mind-bench";
  import MindCard from "./MindCard.svelte";
  import type { MindsStore } from "$lib/stores/minds.svelte";
  let { prefill, minds }: { prefill: (command: string) => void; minds: MindsStore } = $props();
  let own = $state("");
  let opponent = $state("Standard/Hunter");
  let pairs = $state(30);
  let scenario = $state("MUSTER");
  let red = $state(1);
  let blue = $state(1);
  let seed = $state("1");
  let run = $state("");
  let game = $state(1);
  let error = $state("");
  let steps = $state(1);
  let time = $state("1:00");
  let expression = $state("");
  let match = $state("");
  let callsign = $state("RA");
  let candidate = $state("");
  let stationCommand = $state("");
  let selectedCallsign = $state("RA");
  function prepare(make: () => string): void {
    try { const command = make(); error = ""; prefill(command); }
    catch (failure) { error = failure instanceof Error ? failure.message : "Invalid proving request."; }
  }
</script>

<section aria-label="Proving ground">
  <h3>Proving ground</h3>
  <p>Fly saved revisions across paired sides. Prepare the command, then press Enter in the command box to start.</p>
  <form onsubmit={event => { event.preventDefault(); prepare(() => proveCommand(own, opponent, pairs, scenario, red, blue, seed)); }}>
    <label>Your revision/profile<input bind:value={own} placeholder="mine@1/Line" required /></label>
    <label>Opponent<input bind:value={opponent} placeholder="Standard/Hunter or mirror" required /></label>
    <label>Matches (both sides per seed)<input type="number" min="10" max="100" step="2" bind:value={pairs} /></label>
    <label>Scenario<select bind:value={scenario}><option>DUEL</option><option>MUSTER</option><option>HOLD</option></select></label>
    <label>Red units<input type="number" min="1" max="6" step="1" bind:value={red} /></label>
    <label>Blue units<input type="number" min="1" max="6" step="1" bind:value={blue} /></label>
    <label>Seed<input bind:value={seed} inputmode="numeric" required /></label>
    <button>Prepare Prove</button>
  </form>
  <button type="button" onclick={() => minds.readProving(null, null)}>Refresh your runs</button>
  {#if minds.proving}<div class="results"><table><caption>Your proving runs</caption>
    <thead><tr><th>Run</th><th>State</th><th>Scenario</th><th>Games</th><th>Charged unit seconds</th><th>Actions</th></tr></thead>
    <tbody>{#each minds.proving.runs as row (row.run)}<tr><td>{row.run}</td><td>{row.state.replaceAll("_", " ")}</td><td>{row.scenario}</td><td>{row.games}</td><td>{row.charged / 1000}</td><td>
      {#if row.state === "complete"}<button type="button" onclick={() => minds.readProving(row.run, 1)}>Report</button>{/if}
      {#if row.canCancel}<button type="button" onclick={() => prefill(`@mind/cancel ${row.run}`)}>Prepare cancel</button>{/if}
      {#if row.canForget}<button type="button" onclick={() => prefill(`@mind/forget ${row.run}`)}>Prepare forget</button>{/if}
    </td></tr>{/each}</tbody>
  </table></div>
  {#if minds.proving.report}{@const report = minds.proving.report}<h4>Run {report.run} · page {report.page}/{report.pages}</h4>
    {#if report.context}{@const context = report.context}<details><summary>Frozen comparison settings</summary>
      <p>{context.scenario} · {context.ground} · {context.chassis}. Startup {context.startupMs / 1000}s; limit {context.maxDurationMs / 1000}s; native step {context.substepMs}ms.</p>
      <p>Candidate: Class {context.candidate.class}, {context.candidate.profile}. Opponent: Class {context.opponent.class}, {context.opponent.profile}.</p>
      <dl><dt>Scenario digest</dt><dd>{context.scenarioDigest}</dd><dt>Ground digest</dt><dd>{context.groundDigest}</dd>
        <dt>Configuration digest</dt><dd>{context.configDigest}</dd><dt>Candidate source digest</dt><dd>{context.candidate.digest}</dd><dt>Opponent source digest</dt><dd>{context.opponent.digest}</dd>
        <dt>Paired seeds</dt><dd>{context.seeds.join(", ")}</dd></dl>
      <p>Compare these settings before interpreting another run. Different seeds are independent samples.</p>
    </details>{/if}
    <pre>{report.lines.join("\n")}</pre>
    <button type="button" disabled={report.page <= 1} onclick={() => minds.readProving(report.run, report.page - 1)}>Previous</button>
    <button type="button" disabled={report.page >= report.pages} onclick={() => minds.readProving(report.run, report.page + 1)}>Next</button>
  {/if}{/if}
</section>
<section aria-label="Replay bench">
  <h3>Replay bench</h3>
  <p>Choose a retained complete tape. The report marks games whose replay could not be retained.</p>
  <form onsubmit={event => { event.preventDefault(); prepare(() => benchLoadCommand(run, game)); }}>
    <label>Run<input bind:value={run} inputmode="numeric" required /></label>
    <label>Game<input type="number" min="1" max="100" step="1" bind:value={game} /></label>
    <button>Prepare Bench</button>
  </form>
  <form onsubmit={event => { event.preventDefault(); prepare(() => benchLoadMatchCommand(match, callsign)); }}>
    <label>Kept match<input bind:value={match} inputmode="numeric" required /></label>
    <label>Your unit callsign<input bind:value={callsign} required /></label>
    <button>Prepare kept match</button>
  </form>
  <div class="controls"><label>Steps<input type="number" min="1" max="1000" bind:value={steps} /></label>
    <button type="button" onclick={() => prepare(() => benchStepCommand("STEP", steps))}>Prepare step</button>
    <button type="button" onclick={() => prepare(() => benchStepCommand("BACK", steps))}>Prepare back</button>
    {#each ["PIC", "TRACE", "CLOSE"] as action (action)}<button type="button" onclick={() => prefill(`BENCH ${action}`)}>{action}</button>{/each}
  </div>
  <form onsubmit={event => { event.preventDefault(); prepare(() => benchTimeCommand(time)); }}><label>Run to (minutes:seconds)<input bind:value={time} /></label><button>Prepare run</button></form>
  {#if minds.bench}<p>Bench: {minds.bench.state}</p>
    {#if minds.bench.picture}{@const picture = minds.bench.picture}
      <h4>Unit #{picture.unit} · {picture.atMs / 1000}s · event {picture.event}{picture.complete ? " · complete" : ""}</h4>
      <p>State {picture.stateHash}</p>
      {#if picture.timeline.kind === "fork"}
        <p><strong>Fork {picture.timeline.serial}</strong> from reference event {picture.timeline.parentEvent} at {picture.timeline.fromMs / 1000}s · {picture.possessed ? "You control this unit" : "Mind control"}.</p>
        <button type="button" onclick={() => prefill("BENCH REFERENCE")}>Prepare return to reference</button>
      {:else}<p>Reference timeline · verified against the recorded match.</p>{/if}
      {#if picture.refusals.length}<ul aria-label="Bench command refusals">{#each picture.refusals as refusal, index (index)}<li>{refusal}</li>{/each}</ul>{/if}
      <pre aria-label="Bench public picture">{picture.lines.join("\n")}</pre>
      {#if picture.card}<MindCard card={picture.card} stations={picture.armed ? ["helm", "sensors", "fire"] : ["helm", "sensors"]} />{/if}
      <details><summary>Selected mind trace</summary><pre>{picture.traces.join("\n") || "No recorded thinks."}</pre></details>
      <form onsubmit={event => { event.preventDefault(); prepare(() => benchExpressionCommand("EVAL", expression)); }}>
        <label>Inspect an expression<input bind:value={expression} placeholder="pic.speed_kph" disabled={!picture.canEvaluate} /></label>
        <button disabled={!picture.canEvaluate}>Prepare EVAL</button>
        <button type="button" disabled={!picture.canEvaluate} onclick={() => prepare(() => benchExpressionCommand("WATCH", expression))}>Prepare WATCH</button>
        <button type="button" onclick={() => prefill("BENCH WATCH OFF")}>Prepare watch clear</button>
      </form>
      <p>Expressions read the selected mind's authorized picture, card, and memory within 500 operations. Answers appear in the command log.</p>
      {#if minds.bench.watches.length}<ul>{#each minds.bench.watches as watch, index (index)}<li>{watch}</li>{/each}</ul>{/if}
      <h4>Try another mind</h4>
      <p>Compare runs the candidate on the reference pictures with its own memory. Swap starts a separate fork from the reference cursor.</p>
      <form onsubmit={event => { event.preventDefault(); prepare(() => benchCandidateCommand("DIFF", candidate)); }}>
        <label>Candidate revision/profile<input bind:value={candidate} placeholder="mine@2/Line" required /></label>
        <button disabled={picture.timeline.kind !== "reference"}>Prepare compare</button>
        <button type="button" onclick={() => prepare(() => benchCandidateCommand("SWAP", candidate))}>Prepare swap</button>
        <button type="button" onclick={() => prefill("BENCH DIFF OFF")}>Prepare compare off</button>
      </form>
      {#if picture.diff}{@const diff = picture.diff}
        <p>{diff.mind} · {diff.thinks} compared thinks since {diff.fromMs / 1000}s · {diff.differences} differences{diff.omitted ? ` · ${diff.omitted} rows omitted` : ""}.</p>
        <div class="results"><table><caption>Different think results</caption>
          <thead><tr><th>Think</th><th>Time</th><th>Reference intents</th><th>Candidate intents</th><th>Candidate fault</th></tr></thead>
          <tbody>{#each diff.rows as row, index (index)}<tr><td>{row.thinkSeq}</td><td>{row.atMs / 1000}s</td><td>{row.referenceIntents}</td><td>{row.candidateIntents}</td><td>{row.candidateFault ? "Yes" : "No"}</td></tr>{/each}</tbody>
        </table></div>
      {/if}
      <h4>Fly a fork</h4>
      <button type="button" onclick={() => prefill(picture.possessed ? "BENCH RELEASE" : "BENCH POSSESS")}>{picture.possessed ? "Prepare release" : "Prepare possess"}</button>
      <form onsubmit={event => { event.preventDefault(); prepare(() => benchStationCommand(stationCommand)); }}>
        <label>Station command<input bind:value={stationCommand} placeholder="SPEED 20" disabled={!picture.possessed} required /></label>
        <button disabled={!picture.possessed}>Prepare command</button>
      </form>
      {#if picture.privateRun}<form onsubmit={event => { event.preventDefault(); prepare(() => benchActAsCommand(selectedCallsign)); }}>
        <label>Private run callsign<input bind:value={selectedCallsign} required /></label><button>Prepare select unit</button>
      </form>{/if}
    {/if}
  {/if}
</section>
{#if error}<p role="alert">{error}</p>{/if}
<style>
  form { display: flex; flex-wrap: wrap; align-items: end; gap: .5rem; margin-bottom: .5rem; }
  label { display: grid; gap: .3rem; max-width: 100%; }
  input, select { max-width: 100%; min-width: 0; box-sizing: border-box; }
  input[type="number"] { width: 6rem; }
  .controls { display: flex; gap: .4rem; flex-wrap: wrap; }
  .results { overflow-x: auto; }
  table { border-collapse: collapse; width: 100%; }
  td, th { text-align: left; padding: .35rem; }
  pre { white-space: pre-wrap; overflow-wrap: anywhere; }
  dd { overflow-wrap: anywhere; }
  [role="alert"] { color: var(--danger, #ffa09a); }
</style>
