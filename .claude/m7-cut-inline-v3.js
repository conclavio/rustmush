export const meta = {
  name: 'm7-cut',
  description: 'Execute one cut of the M7 completion plan (docs/design/17g-m7-completion-plan.md, cuts C1 to C4) on its own branch in the M7 worktree: plan and briefs, a two-lens plan review, the design note in 17g section 7, serial sub-cuts (build, three review lenses, fixer, gate, judge) with a commit and a push each, then the local lint gates, the cut PR and its CI with a repair loop, and the as-built record — or, with planOnly, stop after the reviewed plan',
  phases: [
    { title: 'Plan', detail: 'planner writes the cut plan and one brief per sub-cut (or an existing plan is loaded); two reviewers; the planner revises; unless planOnly, the design note is committed and pushed' },
    { title: 'Sub-cuts', detail: 'serial; each is a build, three review lenses, a fixer, a gate with a repair loop and a judge, then a commit and a push to the cut branch' },
    { title: 'Close', detail: 'local fmt, clippy, strict rustdoc and contract tests, then the cut PR and its CI with a repair loop, then the 17g ledger record, committed and pushed' },
  ],
}

const A = args
const REPO = 'C:/Users/faeni/cc/rustmush/.claude/worktrees/dazzling-sutherland-89f039'
const S = 'C:/Users/faeni/.claude/projects/C--Users-faeni-cc-rustmush/plan-data/m7-completion'
const COMMON = S + '/common-m7.md'
const MODEL_BRIEF = S + '/cuts/C1/c1c-brief.md'
const DIR = `${S}/cuts/${A.cut}`
const SPEC = 'docs/design/17-unit-minds.md'
const PLAN = 'docs/design/17g-m7-completion-plan.md'
const RECORD = 'docs/design/17f-mind-commerce-implementation-plan.md'
const TRAILER = 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
const BRANCH = A.branch
const DATE = A.date

const CTX = `
CONTEXT
Repository ${REPO}: a git worktree of rustmush (Rust workspace; a MUSH server whose one game is TacSim; Svelte web client under web/), on branch ${BRANCH} (the cut's own branch; a draft pull request into main is opened with the design note, so the Linux CI, fifteen test cells under cargo-nextest in about 26 minutes, runs on every sub-cut push; the PR lands the cut after that CI and the M7 story dispatch pass). Broad test suites run ONCE per sub-cut, in its gate, which sends the workspace sweep to the owner's VPS runner (scripts/vps-gate.sh --wip --runner conclav-dev: 32 cores, about 20 minutes warm) and keeps only focused runs on this Windows box; builders and fixers test only what they changed. Since test-strategy cut T.3 (2026-10-26/27) a crate's integration tests are GROUP BINARIES (tacsim tests/shard_1..shard_6, mush-server tests/world_task|commerce_composition|split_boundary, mushd tests/durable_process|sim_lenses|world_journeys, mush-gateway tests/split_halves; a test is <module>::<fn>, run as --test <group> <module>::; a new test file is a mod line in a group root), as docs/ops.md 'Test targets since T.3' and the common rules say. M7 is the unit-minds programme's commerce milestone. THE SPEC is ${SPEC} (doc 17: section 22's M7 paragraph is the milestone; section 20's D-UM decisions are BINDING; section 21 the owner's answers; section 24 as-built amendments). THE IMPLEMENTATION RECORD of everything already built is ${RECORD} (17f: its status sections at the top, the product rules and the cut map further down). THE PLAN for what remains is ${PLAN} (17g: section 1 the owner's rulings, including those of 2026-10-25 that approve section 3 C2's command table, put C3 inside M7 and take provisional defaults for player-market tuning; section 2 where M7 stands; section 3 the cuts C0 to C4; section 4 what is deferred; section 6 the risks; section 7 the ledger, where C0's entry shows the house style). You are working on CUT ${A.cut} of 17g section 3. The standing builder rules are ${COMMON} (read it). A model brief at the right precision, from an earlier programme, is ${MODEL_BRIEF} (its subject is the arena; take its precision, not its content). Standing gates: the per-crate fmt check, workspace clippy with -D warnings, strict rustdoc for the touched crates, mush-contract-tests, -j 8 on every cargo command; the terminal-width floor is 120 columns for any client surface; the bot (crates/mush-bot) stays external and cargo test -p mush-bot (run on its own) is in the gate of any sub-cut that changes a line-mode sentence it reads.
`
const LEAD_NOTE = A.plannerNote ? `\nA NOTE FROM THE PROGRAMME LEAD, which outranks 17g's prose and any earlier plan text where they differ (never 17g section 1's rulings or doc 17's decisions without saying so): ${A.plannerNote}\n` : ''

const TABLE_SCHEMA = { type: 'object', properties: {
  plan_path: { type: 'string' }, table_path: { type: 'string' },
  design_note: { type: 'string', description: 'the 17g section 7 design note for this cut: one to three dense paragraphs in the style of the C0 entry, separated by blank lines' },
  subcuts: { type: 'array', items: { type: 'object', properties: {
    key: { type: 'string', description: 'lower case, e.g. c1a' }, cut: { type: 'string', description: 'e.g. C1a' }, title: { type: 'string' }, brief: { type: 'string', description: 'absolute path of the brief file' },
    touchesGateway: { type: 'boolean' }, touchesWeb: { type: 'boolean' }, touchesProto: { type: 'boolean' },
    gateCommands: { type: 'array', items: { type: 'string' } }, reviewFocus: { type: 'string' },
  }, required: ['key', 'cut', 'title', 'brief', 'gateCommands', 'reviewFocus'] } },
  assumed_defaults: { type: 'array', items: { type: 'string' } },
  summary: { type: 'string' },
}, required: ['plan_path', 'table_path', 'design_note', 'subcuts', 'summary'] }
const REVIEW_SCHEMA = { type: 'object', properties: {
  findings: { type: 'array', items: { type: 'object', properties: { id: { type: 'string' }, title: { type: 'string' }, grade: { type: 'string', enum: ['critical', 'high', 'medium', 'low'] }, where: { type: 'string' }, evidence: { type: 'string' }, consequence: { type: 'string' }, recommendation: { type: 'string' } }, required: ['id', 'title', 'grade', 'evidence', 'consequence', 'recommendation'] } },
  summary: { type: 'string' },
}, required: ['findings', 'summary'] }
const COMMIT_SCHEMA = { type: 'object', properties: { committed: { type: 'boolean' }, commit: { type: 'string' }, pushed: { type: 'boolean' }, files_changed: { type: 'integer' }, refused_reason: { type: 'string' }, notes: { type: 'string' } }, required: ['committed', 'commit', 'pushed', 'notes'] }
const SWEEP_SCHEMA = { type: 'object', properties: {
  steps: { type: 'array', items: { type: 'object', properties: { command: { type: 'string' }, status: { type: 'string', enum: ['pass', 'fail', 'timeout', 'skipped'] }, summary: { type: 'string' } }, required: ['command', 'status', 'summary'] } },
  failures: { type: 'array', items: { type: 'object', properties: { test: { type: 'string' }, output_excerpt: { type: 'string' }, reruns: { type: 'string' }, assessment: { type: 'string' } }, required: ['test', 'output_excerpt', 'assessment'] } },
  pr: { type: 'string', description: 'the cut PR URL' }, ci_run: { type: 'string', description: 'the CI run id and its conclusion' },
  green: { type: 'boolean' }, notes: { type: 'string' },
}, required: ['steps', 'failures', 'green', 'notes'] }
const FIX_SCHEMA = { type: 'object', properties: { items: { type: 'array', items: { type: 'object', properties: { item: { type: 'string' }, action: { type: 'string', enum: ['fixed', 'not-needed', 'deferred'] }, detail: { type: 'string' } }, required: ['item', 'action', 'detail'] } }, commit_subject: { type: 'string' }, commit_body: { type: 'string' }, commands: { type: 'string' }, notes: { type: 'string' } }, required: ['items', 'commit_subject', 'commit_body', 'commands', 'notes'] }
const DOCS_SCHEMA = { type: 'object', properties: { files_changed: { type: 'array', items: { type: 'string' } }, edits: { type: 'array', items: { type: 'string' } }, notes: { type: 'string' } }, required: ['files_changed', 'notes'] }

const commitPrompt = (what, subject, body, extra) => `You are the committer for ${what} in ${REPO} (branch ${BRANCH}). Do exactly this and nothing else:\n 1. git branch --show-current must print ${BRANCH}; if it does not, stop (committed=false) and say so. git status --short. If there is nothing to commit, say so (committed=false). Look at every UNTRACKED path: source, tests, content, docs, deploy, scripts and web/src files belong; a log, a scratch file, or anything under target/ or node_modules/ does NOT: if you see one, do not commit it, set committed=false and name it in refused_reason.\n 2. With the Write tool create the commit message file ${DIR}/commit-${what.replace(/[^A-Za-z0-9.]+/g, '-')}.txt as: the subject line, a blank line, the body, a blank line, then exactly this trailer line: ${TRAILER}\n 3. git add -A ; git commit -q -F that file. Never amend, never touch history, never force, never commit to main.\n 4. git push -u origin ${BRANCH}. If the push fails, say why (pushed=false) and do not retry more than once.\n 5. Report the short hash (git log --oneline -1) and the files changed (git show --stat --format= HEAD | tail -1).${extra || ''}\n\nSUBJECT:\n${subject}\n\nBODY:\n${body}`

const LENSES = [
  { key: 'feasibility', focus: 'LENS: feasibility. Open every file and symbol the plan and briefs name and check it exists and behaves as assumed at HEAD (git log -1 first): the MindCommerce builder and its ordering guards, configure_roll / configure_economy / configure_market, the launch Active branch, LaunchDeployment and its format, mush_server::boot, the gateway +actions and +act paths, the legal-action providers each sub-cut wraps, and the integration fixtures whose wiring the plan copies. Check each 17g section 1 ruling and each D-UM decision the plan touches is read as written, that the chosen design is the smallest sound one, that the sub-cuts each land green alone in order (no two coupled through a shared type), the sizes are honest, the listed gate commands exist and are complete (every test target named exists or is created by that sub-cut, every command carries -j 8), every existing test that will break is listed (the M7 stories, the House and original-purchase fixtures, the boot-parity tests, the help pins), and every gate the change moves (definition digests, deployment format, IPC_VERSION, TS bindings, legacy-handlers, state-slot owners) is named.' },
  { key: 'adversarial', focus: 'LENS: adversarial: authority, money, privacy and restart. Try to break each decision: can a client reach a commerce action with an actor, payer, digest, price or term it supplies; is an operator right (Registry recall, revocation) granted to anyone but the deployment-named operators; does any new +actions offer, reply, error, event or help text reveal a mind source, memory, an oracle digest, another holder or their holdings, or answer differently for unheld and absent; is money ever moved outside mush-economy, double-charged on a retry or reported refunded on an unknown outcome; does each new owner survive a boot after every step and an unknown acknowledgement; does the builder ordering (v1 against v2 purchases, House two-seat Pins before House renewals, revocation before pending charters) hold in production composition; and do the defaults the plan assumes stay within the economy rules 17f records. For each, say whether the plan handles it and the smallest change if not.' },
]
const reviewPlan = async (plan) => (await parallel(LENSES.map(l => () => agent(`${CTX}${LEAD_NOTE}\nYOUR TASK: you are an independent reviewer of the cut ${A.cut} plan and briefs (${plan.plan_path} and the brief files it names: read them all). READ-ONLY: no edits, no commits, no cargo, no npm. ${l.focus}\nGround every finding in code or document text you read (file and line). Quality over quantity.`, { label: `plan-review:${A.cut}:${l.key}`, phase: 'Plan', model: 'opus', schema: REVIEW_SCHEMA }).then(r => r ? { lens: l.key, ...r } : null)))).filter(Boolean)
const revisePlan = async (plan, reviews) => await agent(
  `${CTX}${LEAD_NOTE}\nYOUR TASK: you are the planner of cut ${A.cut}, revising after review. READ-ONLY on the repository; you may write under ${DIR}/. First run git log -1 and, if the plan was written from an earlier commit than HEAD, re-verify every grounding fact and every file and symbol the plan and briefs name against HEAD (say in the plan's head what you re-verified and what moved). Read your plan (${plan.plan_path}), the briefs, and every finding below; check each against the code yourself before accepting it; then REWRITE ${DIR}/plan.md (adding, or extending, a closing "Review dispositions" table: id, adopted / adopted in part / rejected with the evidence) and every affected brief in place, and rewrite ${DIR}/table.json. Return the final table exactly as you wrote it.\n\nYOUR FIRST TABLE:\n${JSON.stringify(plan, null, 1)}\n\nREVIEWS:\n${JSON.stringify(reviews, null, 1)}`,
  { label: `plan:${A.cut}:revise`, phase: 'Plan', effort: 'high', schema: TABLE_SCHEMA })
const commitNote = async (table) => {
  await agent(
    `${CTX}\nYOUR TASK: record cut ${A.cut}'s design note in ${PLAN} and nothing else. In section 7 ("## 7. Ledger"), after the last existing subsection, add a subsection headed exactly "### ${A.cut} — plan (${DATE}, branch \`${BRANCH}\`)" whose body is the design note below, then a line "Sub-cuts:" and one bullet per sub-cut ("- **<cut>** <title>") from the list below, then a line "Provisional defaults (17g section 1 ruling 6):" and one bullet per assumed default (omit that line if there are none). Use the Edit tool. If a "### ${A.cut} — plan" subsection already exists, stop and say so. Then run: cargo test -p mush-contract-tests --test hex_vocabulary -j 8 (the docs vocabulary gate) and report its result line. Edit no other file; do not commit.\n\nTHE NOTE:\n${table.design_note}\n\nSUB-CUTS:\n${table.subcuts.map(s => `${s.cut}: ${s.title}`).join('\n')}\n\nASSUMED DEFAULTS:\n${(table.assumed_defaults || []).join('\n')}`,
    { label: `docs:${A.cut}:design-note`, phase: 'Plan', model: 'opus', schema: DOCS_SCHEMA })
  return await agent(commitPrompt(`cut ${A.cut} design note`, `docs: 17g cut ${A.cut}, the design note`, `${table.summary}\n\nPlanned from 17g section 3, re-verified against HEAD, reviewed by a feasibility lens and an adversarial lens, and split into ${table.subcuts.length} serial sub-cuts; the as-built record is written when the last one lands.`,
    `\n 6. Open the cut's DRAFT pull request so the Linux CI runs on every sub-cut push: gh pr list --head ${BRANCH} --state open --json number,url; if none is open, Write a short body file under ${DIR}/ (the cut's summary below, its sub-cut list, "Draft until the cut's close sweep is green.", and a last line: 🤖 Generated with [Claude Code](https://claude.com/claude-code)) and run gh pr create --draft --base main --head ${BRANCH} --title "M7 ${A.cut}: <the cut's title from ${PLAN} section 3>" --body-file <that file>. Report the PR URL in notes.`), { label: `commit:${A.cut}:design-note`, phase: 'Plan', model: 'opus', effort: 'low', schema: COMMIT_SCHEMA })
}
// A plan written while the previous cut was still landing is re-verified against HEAD once, before its note is committed.
const refreshPlan = async (plan) => await agent(
  `${CTX}${LEAD_NOTE}\nYOUR TASK: you are the planner of cut ${A.cut}. Your reviewed plan (${DIR}/plan.md, its briefs, ${DIR}/table.json) was written while the previous cut was still landing; it has landed since. READ-ONLY on the repository; you may write under ${DIR}/. Run git log -5, then re-verify every grounding fact, file, symbol, test target and gate command the plan and briefs name against HEAD, especially everything the previous cut's as-built record in ${PLAN} section 7 says it built or deviated on. The CI and test layout also changed under the plan (test-strategy cuts T.0 to T.3 and the pull-request wall, 2026-10-26/27): read docs/ops.md 'Test targets since T.3' and 'The VPS gate', the head comments of .github/workflows/ci.yml (the test matrix and the mind-commerce-acceptance matrix), .github/tacsim-integration-shards.txt and .config/nextest.toml, then translate every run line in the plan, the briefs and the gate lists to the new targets (--test <group> <module>:: or --lib paths; no --test <old file> remains), place any new integration test file as a mod line in the group that fits (tacsim: the shard with the most room; a mush-server lib test module needs no group), write the new mind-commerce-acceptance matrix entry in exactly the form the file now uses, and keep the gated-test inventory's rules. Rewrite in place whatever moved (say in the plan's head what you re-verified and what changed) and rewrite ${DIR}/table.json. EVERY GATE LIST is: the per-crate fmt check, workspace clippy, strict rustdoc for the touched crates, the focused --lib and --test <group> <module>:: runs for what the sub-cut touches (about 15 minutes on this box, never a crate's whole integration set), the relevant mush-contract-tests targets, and as its LAST command exactly: scripts/vps-gate.sh --wip --runner conclav-dev (the workspace sweep on the owner's 32-core VPS runner; it pushes the working tree to gate/<branch>, waits, and exits with the run's status). Return the final table exactly as you wrote it.\n\nYOUR TABLE:\n${JSON.stringify(plan, null, 1)}`,
  { label: `plan:${A.cut}:refresh`, phase: 'Plan', effort: 'high', schema: TABLE_SCHEMA })


// ---------------------------------------------------------------- the sub-cut build (folded in from m7-subcut-build.js)
const buildSubcut = async (A) => {
  const S = A.scratch
  const COMMON = A.common
  const REPO = A.repo

  const SB_REVIEW_SCHEMA = {
    type: 'object',
    properties: {
      findings: {
        type: 'array',
        items: {
          type: 'object',
          properties: {
            id: { type: 'string' },
            title: { type: 'string' },
            grade: { type: 'string', enum: ['must-fix', 'should-fix', 'note'] },
            file: { type: 'string' },
            line: { type: 'integer' },
            evidence: { type: 'string' },
            failure_scenario: { type: 'string' },
            fix_sketch: { type: 'string' },
            confidence: { type: 'string', enum: ['high', 'medium', 'low'] },
          },
          required: ['id', 'title', 'grade', 'file', 'evidence', 'failure_scenario', 'confidence'],
        },
      },
      checklist: {
        type: 'array',
        items: {
          type: 'object',
          properties: { item: { type: 'string' }, status: { type: 'string', enum: ['met', 'partial', 'missing', 'deviated'] }, note: { type: 'string' } },
          required: ['item', 'status'],
        },
      },
      summary: { type: 'string' },
    },
    required: ['findings', 'summary'],
  }

  const SB_FIX_SCHEMA = {
    type: 'object',
    properties: {
      dispositions: {
        type: 'array',
        items: {
          type: 'object',
          properties: {
            id: { type: 'string' },
            action: { type: 'string', enum: ['fixed', 'rebutted', 'deferred'] },
            detail: { type: 'string', description: 'what changed (files, functions, tests), or the evidence that the finding is wrong, or why it is deferred' },
          },
          required: ['id', 'action', 'detail'],
        },
      },
      surface_changes: { type: 'string', description: 'any change to the public surface the builder reported' },
      commands: { type: 'string', description: 'each command run with its final result line verbatim' },
      notes: { type: 'string' },
    },
    required: ['dispositions', 'commands', 'notes'],
  }

  const SB_GATE_SCHEMA = {
    type: 'object',
    properties: {
      steps: {
        type: 'array',
        items: {
          type: 'object',
          properties: {
            command: { type: 'string' },
            status: { type: 'string', enum: ['pass', 'fail', 'timeout', 'skipped'] },
            result_lines: { type: 'string', description: 'the cargo "test result:" lines or the error summary, verbatim' },
            log: { type: 'string', description: 'path of the log file' },
          },
          required: ['command', 'status', 'result_lines'],
        },
      },
      failures: {
        type: 'array',
        items: {
          type: 'object',
          properties: { test: { type: 'string' }, output_excerpt: { type: 'string' }, assessment: { type: 'string' } },
          required: ['test', 'output_excerpt', 'assessment'],
        },
      },
      green: { type: 'boolean' },
      notes: { type: 'string' },
    },
    required: ['steps', 'failures', 'green', 'notes'],
  }

  const SB_JUDGE_SCHEMA = {
    type: 'object',
    properties: {
      verdict: { type: 'string', enum: ['ship', 'ship-with-notes', 'do-not-ship'] },
      reasons: { type: 'string' },
      residual_risks: { type: 'array', items: { type: 'string' } },
      landed_text: { type: 'string', description: 'one dense paragraph for the 17g ledger: what is now in the tree, backticked identifiers, no newlines' },
      deviations_text: { type: 'string', description: 'numbered deviations from the brief, the plan, 17g and doc 17 with reasons, one paragraph, no newlines' },
      commit_subject: { type: 'string' },
      commit_body: { type: 'string', description: 'three to fourteen lines in the style of the repository history: what the change does in plain prose, then a Design: line naming 17g and the cut; no trailer' },
      next_builder_surface: { type: 'string', description: 'the public surface later sub-cuts depend on, verified against the diff' },
    },
    required: ['verdict', 'reasons', 'residual_risks', 'landed_text', 'deviations_text', 'commit_subject', 'commit_body', 'next_builder_surface'],
  }

  const SB_CTX = `Repository ${REPO} (a git worktree of rustmush: a Rust workspace, a MUSH server whose one game is TacSim), branch ${A.branch}. M7 is the unit-minds commerce milestone: doc 17 (docs/design/17-unit-minds.md) is the spec, 17f (docs/design/17f-mind-commerce-implementation-plan.md) the implementation record, 17g (docs/design/17g-m7-completion-plan.md) the completion plan with the owner's rulings in section 1 and the ledger in section 7. Sub-cut ${A.cut}: ${A.title}. The builder's brief is ${A.brief}; the rules every builder follows are ${COMMON}; the reviewed plan behind the brief is ${A.packet}. The base commit is ${A.base}; the sub-cut's changes are UNCOMMITTED in the working tree (see them with: git status --short ; git diff ; and read new untracked files directly). Everything uncommitted in the tree belongs to this sub-cut.`


  if (A.buildReport) log(`${A.cut}: the build already stands in the tree; its report was supplied, so Build is skipped and the review starts from it`)
  const buildReport = A.buildReport || await agent(
    `You are the builder of sub-cut ${A.cut} (${A.title}) in ${REPO}. Read ${COMMON} first, then ${A.brief}, and execute the brief completely. Test only what you changed: the focused commands your brief lists, plus the per-crate fmt check, workspace clippy and strict rustdoc for the touched crates; never a workspace run, every tacsim integration target, the full server, mushd or gateway suites, or an M7 story (the gate runner and the PR's Linux CI run the broad suites). Your final message is your report in the eight sections the common rules name; it is consumed by a reviewer agent, so be exact and complete rather than brief.${A.builderExtra ? '\n\n' + A.builderExtra : ''}`,
    { label: `build:${A.cut}`, phase: 'Build', model: 'opus', effort: 'high' },
  )
  if (!buildReport) return { cut: A.cut, error: 'builder returned nothing' }


  const SB_LENSES = [
    {
      key: 'correctness',
      focus: `LENS: correctness, authority and adversarial. Read the whole diff and the new files. Hunt for bugs the change introduces or fails to close: an authority path a client can reach that the brief does not grant (a client-supplied actor, payer, digest, price or term; an operator right granted to the wrong principal; a verb that skips the legal-action provider's own checks), a privacy or observer leak in any new reply, error, event, notice or +actions listing (source, memory, an oracle digest, another holder's identity or holdings; a refusal that differs between "unheld" and "absent"), money that moves outside mush-economy's posting API or is not conserved, an idempotence or restart claim that does not hold (a boot after each step, an unknown outcome, a retried request, a lost acknowledgement), builder ordering the composition gets wrong (v1 against v2 purchases, House two-seat Pins before House renewals, revocation before pending charters), state stranded on an error path, panics or unwraps in non-test code, integer overflow. For each suspected bug trace the real code path and give a concrete scenario. ${A.correctnessExtra || ''}`,
    },
    {
      key: 'conformance',
      focus: `LENS: conformance. Go through the brief item by item and sentence by sentence and return a checklist entry for each instruction (met / partial / missing / deviated), reading the code, not the builder's report. Check that every decision the brief marks as made was followed, that nothing the brief did not ask for was added (scope creep, extra verbs, compatibility shims for data that does not exist, a second lane kept beside the old one), that 17g section 1's rulings and doc 17's D-UM decisions still hold or the deviation is declared in the builder's report, that every player-facing sentence and help text is true, short and in the voice of the existing ones, that the gates the change moves were re-blessed (panic budget, state-slot owners, legacy-handlers, module-ns-owners, restriction lints, feature census, content digests, IPC_VERSION, TS bindings, the tacsim shard list), and that the builder's SURFACE section matches the code exactly. ${A.conformanceExtra || ''}`,
    },
    {
      key: 'tests',
      focus: `LENS: tests. For every test the brief requires, find it and judge whether it really proves the claim or only smoke-tests it (asserts a specific outcome against the real store, the real ledger or the real transcript, versus asserting that nothing errored; boots the production composition and drives the real verb path, versus a fixture that skips the step under test). List every EXISTING assertion the diff changed or deleted (git diff on the test files) and judge whether each change is justified by a behaviour the brief deliberately changes or is a weakening. Check that every new surface has a privacy or observer assertion, every new command a refusal test for the wrong principal, and every new state a restart (boot after the step) test. Check new tests for flakiness (wall-clock sleeps, races, order dependence) and for run time. ${A.testsExtra || ''}`,
    },
  ]
  const reviews = (await parallel(SB_LENSES.map(l => () =>
    agent(
      `${SB_CTX}\n\nYou are an independent reviewer; you did not write this change. READ-ONLY: do not edit any file, do not commit, and do NOT run cargo (the builder has finished; a gate runner will run the suites after you). git read commands are fine. In the Bash tool a doubled backslash collapses to one: prefer the Read, Grep and Glob tools.\n\n${l.focus}\n\nTHE BUILDER'S REPORT:\n${buildReport}\n\nGrade each finding must-fix (a defect, a missed brief item, a vacuous required test, a weakened assertion, a leak, an authority hole), should-fix (real but small, or a text that misleads) or note. Quote file and line for each. Ten real findings beat thirty guesses; an empty list is a valid answer.`,
      { label: `review:${A.cut}:${l.key}`, phase: 'Review', model: 'opus', schema: SB_REVIEW_SCHEMA },
    ).then(r => r ? { lens: l.key, ...r } : null)))).filter(Boolean)

  const actionable = reviews.flatMap(r => (r.findings || []).filter(f => f.grade !== 'note').map(f => ({ ...f, id: `${r.lens}/${f.id}` })))
  log(`${A.cut}: ${reviews.length} reviews, ${actionable.length} actionable findings`)

  let fix = null
  if (actionable.length) {
    fix = await agent(
      `${SB_CTX}\n\nYou are the fixer for sub-cut ${A.cut}. Read ${COMMON} and ${A.brief} first. Three independent reviewers read the builder's uncommitted change and raised the findings below. For EACH finding: open the code, decide whether it is right, and either fix it (code and tests, in the style of the surrounding code, within the brief's decisions and the common rules) or rebut it with exact evidence (file and line); never ignore one, and never weaken a test to make a finding go away. A finding that asks for something outside the brief's scope is "deferred" with the reason. You own cargo now (-j 8 always): after your changes run the per-crate fmt check, cargo clippy --workspace --all-targets -j 8 -- -D warnings, strict rustdoc for the crates you touched, and only the focused tests for the code your fixes touched (never a broad suite: the gate runs those next), and paste the result lines. You never commit and never edit docs/design.\n\nTHE BUILDER'S REPORT:\n${buildReport}\n\nTHE FINDINGS:\n${JSON.stringify(actionable, null, 1)}`,
      { label: `fix:${A.cut}`, phase: 'Fix', model: 'opus', effort: 'high', schema: SB_FIX_SCHEMA },
    )
  }

  const gatePrompt = (round) => `${SB_CTX}\n\nYou are the gate runner for sub-cut ${A.cut}, round ${round}. You own cargo (always -j 8); nobody else is building. You do NOT edit any file in the repository and do not commit. Create the log directory ${S}/${A.key}/gate${round}/ and run each command below in order (long ones with run_in_background, output redirected to a log file there, wrapped in an outer timeout; poll the log), then record its status and paste its final cargo "test result:" lines (or the error) verbatim:\n${A.gateCommands.map((c, i) => `  ${i + 1}. ${c}`).join('\n')}\nKnown environment facts: Windows link error LNK1104 is transient, rerun once. A test binary that prints nothing new for 20 minutes is hung: find it with PowerShell Get-Process (path under target), kill it, and report it. The command scripts/vps-gate.sh --wip --runner conclav-dev is the workspace sweep on the owner's VPS runner (docs/ops.md 'The VPS gate'): run it under the Bash tool from the repository root, in the background with an outer timeout of 90 minutes, output to a log; it pushes a commit of the working tree to gate/<branch> (the tree and index untouched), dispatches .github/workflows/gate.yml, waits, downloads the nextest JUnit reports to target/gate/<run id>/, prints each failed test's binary and name, and exits with the run's status; the warm sweep takes about 20 minutes, a cold one about 40. Read its log for the failed tests (a <flakyFailure> that passed on retry is not a failure); rerun a failed test alone here with its T.3 run line (cargo test -p <crate> --test <group> <module>::<fn> -j 8 -- --exact) before assessing it. If the dispatch itself fails (no write access, a workflow error) or the run is cancelled, say so as environment and report the sweep as not run, not as green. Load-sensitive suites that may fail under load and pass alone are listed in ${COMMON}; a "panicked at ...process_fault.rs" line inside a passing fault test is injected. For every failing test: capture the failure output, rerun that single test alone up to two times, and assess whether it is a real defect caused by this change, a pre-existing or load flake (with evidence), or environment. green = every command passed (after at most one rerun for a transient).`
  let gate = await agent(gatePrompt(1), { label: `gate:${A.cut}:1`, phase: 'Gate', model: 'opus', schema: SB_GATE_SCHEMA })
  let repairs = []
  for (let round = 2; round <= 3 && gate && !gate.green; round++) {
    const repair = await agent(
      `${SB_CTX}\n\nYou are the repairer for sub-cut ${A.cut}. Read ${COMMON} and ${A.brief}. The gate run failed; the failures are below. Find the cause of each real failure and fix it in the style of the surrounding code, within the brief's decisions. Never weaken or delete a test to get green; if a failing test pins behaviour the brief deliberately changes, update that assertion and say so. If a failure is a flake or environment, prove it (rerun alone) and leave the code. You own cargo (-j 8): rerun the failing targets, then the per-crate fmt check and cargo clippy --workspace --all-targets -j 8 -- -D warnings. You never commit and never edit docs/design.\n\nGATE RESULT:\n${JSON.stringify(gate, null, 1)}`,
      { label: `repair:${A.cut}:${round - 1}`, phase: 'Gate', model: 'opus', effort: 'high', schema: SB_FIX_SCHEMA },
    )
    repairs.push(repair)
    gate = await agent(gatePrompt(round), { label: `gate:${A.cut}:${round}`, phase: 'Gate', model: 'opus', schema: SB_GATE_SCHEMA })
  }

  const judge = await agent(
    `${SB_CTX}\n\nYou are the judge for sub-cut ${A.cut}; you wrote none of it. READ-ONLY, no cargo. Read ${A.brief}, the cut's plan (${A.packet}) and the cut's design note in 17g section 7 (the "### ${A.ledgerRow}" subsection of docs/design/17g-m7-completion-plan.md). Read the WHOLE uncommitted diff and the new files yourself (git status --short; git diff; Read the untracked files); do not trust the reports below for what the code does. Then decide: ship (the brief is met, the gate is green, no must-fix finding stands), ship-with-notes (green and sound, with named residual items that are honestly small), or do-not-ship (say exactly what must change). Check specifically that every must-fix finding was really fixed or really rebutted, that no existing assertion was weakened, that no client-reachable authority or privacy hole remains, and that the builder's claimed surface matches the code.\nThen write: landed_text and deviations_text for the 17g ledger (dense, factual, backticked identifiers, no newlines; deviations numbered (1), (2), ... each with its reason, including anything deferred); a commit subject in the repository's style ("<crate>(<areas>), <crate>: <what> (17g ${A.cut})") and a commit body of three to fourteen lines of plain prose describing what the change does, ending with a "Design:" line naming 17g ${A.ledgerRow} (no trailer; the committer adds it); and next_builder_surface: the public surface later sub-cuts depend on, verified against the diff.\n\nBUILDER'S REPORT:\n${buildReport}\n\nREVIEWS:\n${JSON.stringify(reviews.map(r => ({ lens: r.lens, summary: r.summary, findings: r.findings, checklist: r.checklist })), null, 1)}\n\nFIXER:\n${JSON.stringify(fix, null, 1)}\n\nREPAIRS:\n${JSON.stringify(repairs, null, 1)}\n\nFINAL GATE:\n${JSON.stringify(gate, null, 1)}`,
    { label: `judge:${A.cut}`, phase: 'Judge', effort: 'high', schema: SB_JUDGE_SCHEMA },
  )

  return { cut: A.cut, buildReport, reviews, fix, repairs, gate, judge }
}

// ---------------------------------------------------------------- Plan
let table = null
let reviewCount = 0
phase('Plan')
if (A.skipPlan) {
  table = await agent(`Read the file ${DIR}/table.json with the Read tool and return its contents EXACTLY as written (every field of every entry verbatim, every gate command verbatim, nothing summarised, nothing dropped). Do nothing else.`, { label: `load:${A.cut}:table`, phase: 'Plan', model: 'opus', effort: 'low', schema: TABLE_SCHEMA })
  if (!table) return { cut: A.cut, error: 'table.json could not be loaded' }
  if (A.reviewExisting) {
    const reviews = await reviewPlan(table)
    reviewCount = reviews.reduce((n, r) => n + r.findings.length, 0)
    log(`cut ${A.cut} existing plan: ${reviewCount} review findings`)
    const revised = await revisePlan(table, reviews)
    if (revised) table = revised
  }
  if (A.refresh) {
    const refreshed = await refreshPlan(table)
    if (refreshed) table = refreshed
    log(`cut ${A.cut}: plan refreshed against HEAD`)
  }
  if (A.commitNote) {
    const c = await commitNote(table)
    log(`cut ${A.cut} design note: ${c ? c.commit : 'not committed'}`)
  }
} else {
  const plan1 = await agent(
    `${CTX}${LEAD_NOTE}\nYOUR TASK: plan cut ${A.cut} and write its builder briefs. READ-ONLY on the repository (no edits, no commits, no cargo, no npm); you may write files under ${DIR}/ (create it). Read: ${COMMON}; ${MODEL_BRIEF}; ${PLAN} WHOLE (section 3's ${A.cut} paragraph is the cut's entry and gate); ${RECORD}'s status sections and every rule section the cut touches; doc 17 section 22's M7 paragraph, every D-UM decision in section 20 the cut implements or touches, section 21, and section 24; and THE CODE behind every item (re-verify each at HEAD; never trust 17g's line numbers, which were taken at c123b069: the composition in crates/tacsim/src/composition/ (mind_commerce.rs and mod.rs), the launch (crates/tacsim/src/launch.rs and launch/), mush_server::boot, the gateway's +actions and +act paths in crates/mush-gateway/src/bridge_common.rs, the mush-roll, mush-market and mush-economy owners the cut composes or wraps, mush-proto's MindRequest and Query where the typed surface moves, the TUI and web consumers where a client changes, and the integration fixtures under crates/tacsim/src/minds/bulk/ whose wiring the cut copies).\nWrite ${DIR}/plan.md: per sub-cut its scope, the decisions (made, with types, config shapes and every player-facing sentence and help text verbatim; every 17g ruling and D-UM decision it implements cited), the files, the tests (a production-composition test for every newly composed owner; a refusal test for the wrong principal for every new command; a privacy or observer assertion for every new surface; a boot-after-the-step test for every new state; vitest and the ts_export diff where web/ or mush-proto changes), the existing assertions that will deliberately change, the gates that move (definition digests, deployment format, IPC_VERSION, TS bindings, legacy-handlers, state-slot owners, module-ns owners, panic budget, help pins, the tacsim shard list for any new crates/tacsim/tests file), whether the gateway, web/ or mush-proto changes, and any deployment or store consequence (pre-launch: a fresh store at the next deploy is acceptable; say when one is needed). Every default the cut must choose (prices, permilles, windows, service actors, operator lists) is a provisional default per 17g section 1 ruling 6: choose a reasonable one consistent with 17f's economy rules, state it in the plan, and list it in assumed_defaults. Split the cut into sub-cuts that each land green alone, in order, at roughly 1k to 3k lines with tests (17g section 3's sub-cut split is the starting point; say why where you depart from it).\nWrite ONE BRIEF PER SUB-CUT at ${DIR}/<key>-brief.md (keys ${A.cut.toLowerCase()}a, ${A.cut.toLowerCase()}b, ... unless 17g names them otherwise), self-contained at the model brief's precision: starts by telling the builder to read ${COMMON}; says HEAD is "the previous sub-cut landed" and that a section headed "WHAT THE EARLIER SUB-CUTS LEFT" may be appended at its end by the committer and outranks the plan; anchors on symbols, never line numbers; states decisions as made; names files; lists tests to add with what each proves; lists deliberate assertion changes; lists the FOCUSED commands the builder runs before reporting (the per-crate fmt check, workspace clippy, strict rustdoc for the touched crates, and only the tests for what the sub-cut changes: module-filtered --lib runs, the --test targets it touches, the exact named tests it adds; cargo test -p mush-bot only where a sentence the bot reads changes; npm ci / npm test / npm run build plus the ts_export diff where web/ or mush-proto changes; never a workspace run, every tacsim integration target, the full mush-server, mushd or mush-gateway suites, or an M7 story); ends with the common rules' report format with SURFACE spelled out for the next sub-cut.\nAlso return: the 17g section 7 design note for cut ${A.cut}, and per sub-cut its key, label, title, brief path, gate command list, what its diff reviewers must look hardest at, and whether it touches the gateway, web/ or mush-proto. THE GATE LIST is the sub-cut's one broad local run and must finish in about 25 minutes on this Windows box: the per-crate fmt check, workspace clippy, strict rustdoc for the touched crates, the touched crates' --lib suites (tacsim --lib takes about 3 minutes), the integration targets the sub-cut touches or that pin what it changes (by --test name), the relevant mush-contract-tests targets, and the gateway or mushd twins only where a session, renderer or line-mode reply changes; -j 8 on every cargo command. NEVER put in a gate list a workspace run, all tacsim integration targets, or an #[ignore]d M7 story: the cut PR's Linux CI runs the whole workspace on every push, and the four M7 stories run on a CI dispatch before the merge. Write the same table as JSON to ${DIR}/table.json (the exact object you return).`,
    { label: `plan:${A.cut}`, phase: 'Plan', effort: 'high', schema: TABLE_SCHEMA })
  if (!plan1) return { cut: A.cut, error: 'planner returned nothing' }
  const reviews = await reviewPlan(plan1)
  reviewCount = reviews.reduce((n, r) => n + r.findings.length, 0)
  log(`cut ${A.cut} plan: ${reviewCount} review findings`)
  table = (await revisePlan(plan1, reviews)) || plan1
  if (A.planOnly) {
    return { cut: A.cut, planOnly: true, plan_path: table.plan_path, table_path: table.table_path, subcuts: table.subcuts.map(s => ({ key: s.key, cut: s.cut, title: s.title, brief: s.brief, touchesGateway: !!s.touchesGateway, touchesWeb: !!s.touchesWeb, touchesProto: !!s.touchesProto, gates: s.gateCommands.length })), assumed_defaults: table.assumed_defaults, summary: table.summary, design_note_length: (table.design_note || '').length, review_findings: reviewCount, reviews: reviews.map(r => ({ lens: r.lens, summary: r.summary, findings: r.findings.map(f => `${f.grade}: ${f.title}`) })) }
  }
  const c = await commitNote(table)
  log(`cut ${A.cut} design note: ${c ? c.commit : 'not committed'}`)
}
if (!table || !table.subcuts || !table.subcuts.length) return { cut: A.cut, error: 'no sub-cut table' }

// ---------------------------------------------------------------- Sub-cuts
const startIndex = A.startAt ? table.subcuts.findIndex(x => x.key === A.startAt) : 0
if (startIndex < 0) return { cut: A.cut, error: 'startAt names no sub-cut: ' + A.startAt }
const done = []
let halted = null
if (A.startAt && A.resumeBuildReportPath && !A.resumeBuildReport) {
  const loaded = await agent(`Read the file ${A.resumeBuildReportPath} with the Read tool (it may need more than one read with an offset; read ALL of it) and return its full contents EXACTLY as written in the text field: every line verbatim, nothing summarised, nothing dropped, nothing added. Do nothing else.`, { label: `load:${A.cut}:build-report`, phase: 'Sub-cuts', model: 'opus', effort: 'low', schema: { type: 'object', properties: { text: { type: 'string' } }, required: ['text'] } })
  if (loaded && loaded.text && loaded.text.length > 1000) { A.resumeBuildReport = loaded.text; log(`${A.cut}: the saved builder report for ${A.startAt} was loaded (${loaded.text.length} chars); its review resumes from it`) }
  else return { cut: A.cut, error: 'the saved builder report could not be loaded; nothing was run' }
}
for (const sc of table.subcuts.slice(startIndex)) {
  phase('Sub-cuts')
  log(`${sc.cut}: ${sc.title}`)
  const later = table.subcuts.slice(table.subcuts.indexOf(sc) + 1).map(x => x.brief)
  const r = await buildSubcut({
    cut: sc.cut, key: sc.key, ledgerRow: `${A.cut} — plan`, title: sc.title, repo: REPO, branch: BRANCH, common: COMMON,
    base: `the current HEAD of ${BRANCH} (git log -1); every earlier sub-cut of this cut is committed and pushed below it`,
    scratch: DIR, brief: sc.brief, packet: `${DIR}/plan.md (the reviewed plan of cut ${A.cut}) and ${PLAN} (17g: section 1's rulings and section 3's ${A.cut} paragraph)`,
    builderExtra: `This is a sub-cut of cut ${A.cut} of the M7 completion plan (${PLAN} section 3). Your brief outranks the plan where they differ; 17g section 1's rulings and doc 17's decisions outrank both unless the brief says the lead ruled otherwise. If your brief ends with sections headed "WHAT THE EARLIER SUB-CUTS LEFT", read them first.` + (sc.key === A.startAt && A.resumeNote ? ' ' + A.resumeNote : ''),
    correctnessExtra: sc.reviewFocus,
    conformanceExtra: 'Check the brief item by item and every 17g ruling and D-UM decision it says it implements; every player-facing sentence and help text verbatim; every gate the change moves; that changes to docs/design are DECLARED in the report for the record writer rather than edited (builders never edit docs/design).',
    testsExtra: 'Check a production-composition test for every newly composed owner, a wrong-principal refusal test for every new command, a privacy or observer assertion for every new surface, a boot-after-the-step test for every new state, vitest and the ts_export diff where a client or mush-proto changed, that every deliberately changed existing assertion is listed with its reason and none was weakened, and that each decision built has a test that fails on the old code.',
    gateCommands: sc.gateCommands,
    buildReport: (sc.key === A.startAt && A.resumeBuildReport) ? A.resumeBuildReport : undefined,
  })
  const verdict = r && r.judge ? r.judge.verdict : 'no-judge'
  const green = !!(r && r.gate && r.gate.green)
  if (!r || verdict === 'do-not-ship' || verdict === 'no-judge' || !green) {
    halted = { at: sc.cut, verdict, green, reasons: r && r.judge ? r.judge.reasons : null, gate_failures: r && r.gate ? r.gate.failures : null, gate_notes: r && r.gate ? (r.gate.notes || '').slice(0, 1500) : null }
    log(`${sc.cut}: HALT (verdict ${verdict}, gate green ${green})`)
    break
  }
  const j = r.judge
  const commit = await agent(commitPrompt(`sub-cut ${sc.cut}`, j.commit_subject, j.commit_body,
    `\n 6. With the Write tool create ${DIR}/${sc.key}-out/landed.txt, deviations.txt, surface.md and residual.txt from the texts below.\n 7. For EACH of these later brief files, append (read it, then write it back with the new section added at the very end) a section headed "## WHAT THE EARLIER SUB-CUTS LEFT: ${sc.cut} (commit <hash>)" followed by the SURFACE text, verbatim: ${JSON.stringify(later)}\n\nLANDED (ledger):\n${j.landed_text}\n\nDEVIATIONS (ledger):\n${j.deviations_text}\n\nRESIDUAL RISKS:\n${(j.residual_risks || []).join('\n')}\n\nSURFACE:\n${j.next_builder_surface}`),
    { label: `commit:${sc.cut}`, phase: 'Sub-cuts', model: 'opus', effort: 'low', schema: COMMIT_SCHEMA })
  if (!commit || !commit.committed) { halted = { at: sc.cut, verdict, green, reasons: 'the committer did not commit: ' + (commit ? (commit.refused_reason || commit.notes) : 'no answer') }; break }
  done.push({ cut: sc.cut, title: sc.title, verdict, commit: commit.commit, pushed: commit.pushed, files_changed: commit.files_changed, touchesGateway: !!sc.touchesGateway, touchesWeb: !!sc.touchesWeb, touchesProto: !!sc.touchesProto, residual_risks: (j.residual_risks || []).slice(0, 8) })
  log(`${sc.cut}: ${verdict}, committed ${commit.commit}, pushed ${commit.pushed}`)
}

// ---------------------------------------------------------------- Close
let sweep = null, docs = null, finalCommit = null
const repairs = []
if (!halted) {
  phase('Close')
  const sweepPrompt = (round) => `You are the sweep runner closing cut ${A.cut} in ${REPO} (branch ${BRANCH}), round ${round}. Every sub-cut is committed and pushed; the tree should be clean (git status --short). You own cargo (always -j 8); you edit no repository file and commit nothing. Create ${DIR}/sweep/round${round}/ and run, strictly one after another, each with its output in a log file there (long ones with run_in_background, wrapped in an outer timeout, polling; a single Bash call is capped at 10 minutes):\n  1. For each crate with a changed file since the cut's base (git diff --name-only origin/main...HEAD, mapped to crates/<name>), cargo fmt -p <crate> -- --check (cargo fmt --all fails on Windows with os error 206).\n  2. cargo clippy --workspace --all-targets -j 8 -- -D warnings\n  3. RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items -j 8\n  4. cargo test -p mush-contract-tests -j 8 --no-fail-fast\n  5. THE CUT PR AND ITS CI (the Linux workspace run that is the cut's real test gate). gh pr list --head ${BRANCH} --state open --json number,url. If none is open, create one: gh pr create --base main --head ${BRANCH} --title "M7 ${A.cut}: <the cut's title from ${PLAN} section 3>" --body-file <a file you Write under ${DIR}/sweep/ summarising the cut from ${DIR}/plan.md and the sub-cuts below, ending with the line: 🤖 Generated with [Claude Code](https://claude.com/claude-code)>. Then find the pull_request CI run for the branch's current head commit (git rev-parse HEAD; gh run list --branch ${BRANCH} --workflow CI --event pull_request --json databaseId,headSha,status,conclusion; it can take a minute to appear) and wait for it to complete: poll gh run view <id> --json status,conclusion,jobs every few minutes, each Bash call under 10 minutes, for up to 75 minutes in all. For every failed job, read its failing tests (gh run view <id> --log-failed, or gh api repos/conclavio/rustmush/actions/jobs/<job id>/logs) and record each failing test with its output excerpt; compare with main's most recent CI run (gh run list --branch main --workflow CI --limit 3) to tell a pre-existing failure from one this cut introduced.\nKnown environment facts: LNK1104 is transient (rerun once); the load-sensitive suites in ${COMMON} may fail under load and pass alone; a "panicked at ...process_fault.rs" line inside a passing fault test is injected. green = steps 1 to 4 passed and the PR's CI run concluded success (or failed only on a job main fails in the same way, with the evidence). When green and the PR is a draft, mark it ready: gh pr ready <number>.\n\nSUB-CUTS AS LANDED:\n${JSON.stringify(done.map(d => ({ cut: d.cut, title: d.title, commit: d.commit })), null, 1)}`
  sweep = await agent(sweepPrompt(1), { label: `sweep:${A.cut}:1`, phase: 'Close', model: 'opus', schema: SWEEP_SCHEMA })
  for (let round = 2; round <= 3 && sweep && !sweep.green; round++) {
    const repair = await agent(`You are the repairer closing cut ${A.cut} in ${REPO} (branch ${BRANCH}). Read ${COMMON}, ${DIR}/plan.md and the brief of whichever sub-cut owns the failing area. The close sweep failed; the REAL failures are below (local gates, or the PR's Linux CI). Fix the cause of each in the style of the surrounding code, within the cut's decisions. Never weaken or delete a test to get green; if a failing test pins behaviour the cut deliberately changed, update that assertion and say so. You own cargo (-j 8): rerun the failing targets, then the per-crate fmt check and workspace clippy. Do not commit; never edit docs/design. Return the commit subject and the commit body IN THEIR OWN FIELDS (commit_subject, commit_body), never only inside notes. Before fixing, scan for every OTHER place in the workspace the same cause would bite and fix them in the same pass, so one repair round is enough.\n\nSWEEP RESULT:\n${JSON.stringify(sweep, null, 1)}`, { label: `repair:${A.cut}:${round - 1}`, phase: 'Close', model: 'opus', effort: 'high', schema: FIX_SCHEMA })
    repairs.push(repair)
    if (repair) await agent(commitPrompt(`cut ${A.cut} sweep repair ${round - 1}`, repair.commit_subject || `M7 ${A.cut}: a repair of what the close sweep found (17g ${A.cut})`, repair.commit_body || repair.notes || 'A repair of what the close sweep found.'), { label: `commit:${A.cut}:repair${round - 1}`, phase: 'Close', model: 'opus', effort: 'low', schema: COMMIT_SCHEMA })
    sweep = await agent(sweepPrompt(round), { label: `sweep:${A.cut}:${round}`, phase: 'Close', model: 'opus', schema: SWEEP_SCHEMA })
  }
  if (sweep && sweep.green) {
    docs = await agent(
      `${CTX}\nYOUR TASK: write the as-built record of cut ${A.cut}. You edit ONLY files under docs/design/ and you do not commit. INPUTS: the plan ${DIR}/plan.md; per sub-cut the independent judge's texts under ${DIR}/<key>-out/ (landed.txt, deviations.txt, residual.txt, surface.md); the sub-cuts as they landed and the close sweep, below. WRITE: (a) in ${PLAN} section 7, directly after the "### ${A.cut} — plan" subsection, a subsection "### ${A.cut} — as built" with Landed (per sub-cut "**<cut>** (\`hash\`): " + the judge's landed text), Evidence (the loop each sub-cut went through, must-fix counts where the texts give them, the close sweep and the PR's CI run, and what was NOT run: the four M7 stories run on a CI dispatch before the merge, and no hand-played session on this build), and Deviations and residuals (per sub-cut the judge's deviations text, then the residuals still open, merged and de-duplicated); in the house style of the C0 entry. (b) In ${PLAN} sections 2 and 3, minimal in-place edits only where a statement became untrue and you have checked the code (for example the section 2 table's Production and Player-entry cells for what the cut composed or exposed). (c) In ${RECORD}, only where a normative sentence became untrue and you have checked the code. (d) In ${SPEC} section 24, as-built bullets for any departure from a D-UM decision, named with its number. (e) Run cargo test -p mush-contract-tests --test hex_vocabulary -j 8 and report its result line. Do NOT rewrite earlier ledger subsections.\n\nSUB-CUTS AS LANDED:\n${JSON.stringify(done, null, 1)}\n\nTHE CLOSE SWEEP:\n${JSON.stringify({ steps: sweep.steps, failures: sweep.failures, pr: sweep.pr, ci_run: sweep.ci_run, notes: (sweep.notes || '').slice(0, 2000) }, null, 1)}\n\nSWEEP REPAIRS:\n${JSON.stringify(repairs.map(x => x ? { items: x.items, subject: x.commit_subject } : null), null, 1)}`,
      { label: `docs:${A.cut}:record`, phase: 'Close', effort: 'high', schema: DOCS_SCHEMA })
    finalCommit = await agent(commitPrompt(`cut ${A.cut} record`, `docs: 17g cut ${A.cut} as built`, `The 17g section 7 record of cut ${A.cut}: what each sub-cut landed, the evidence (the sub-cut loop, the close sweep and the PR's CI run), and the deviations and open residuals, from the judges' texts. Sections 2 and 3 are corrected where the cut made them untrue.`), { label: `commit:${A.cut}:record`, phase: 'Close', model: 'opus', effort: 'low', schema: COMMIT_SCHEMA })
  }
}

return { cut: A.cut, branch: BRANCH, subcuts_planned: table.subcuts.map(s => s.cut + ' ' + s.title), assumed_defaults: table.assumed_defaults, review_findings: reviewCount, done, halted,
  sweep: sweep ? { green: sweep.green, pr: sweep.pr, ci_run: sweep.ci_run, failures: sweep.failures, steps: sweep.steps } : null,
  repairs: repairs.map(x => x ? { items: x.items, subject: x.commit_subject } : null), docs, final_commit: finalCommit ? finalCommit.commit : null }
