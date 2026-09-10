# Character and cast studio

Studio develops a character or an organized cast before you send bots into a
game. Start with a JSON concept, compare numbered alternatives, refine the
ones you like, then export ordinary character and group files. It uses the
bot's own model client and cost ledger. It does not connect to SSH, create game
accounts, or act in the world.

The supplied examples describe a woman who walks dogs by day and studies at
night, and a four-piece band with conflicting ambitions:

```powershell
cargo run -p mush-bot -- --config mush-bot.toml --studio bots/studio/dogwalker.json --output .mush-bot/studio-dogwalker
cargo run -p mush-bot -- --config mush-bot.toml --studio bots/studio/four-piece-band.json --output .mush-bot/studio-band
```

Add `--tui` for a candidate sidebar, a synopsis/detail pane and a command editor.
It has the same saved artifacts and authoring controls as the text interface.

Both request ten alternatives. The band concept preserves the singer and
drummer's history, the guitarist's showmanship, and the bassist's private
assessment of the group. Each alternative includes a synopsis and complete
character sheets, with appearance, background, personality, voice, interests,
role and initial goal. Group alternatives also include the roster, shared
mission and member briefings.

## Concepts and alternatives

A minimal concept is:

```json
{
  "version": 1,
  "kind": "character",
  "concept": "She walks dogs during the day while spending her nights studying."
}
```

Use `kind: "group"` for a cast. `count` defaults to 10 and accepts 1–10.
`max_tokens` defaults to 16,384 and accepts 512–65,536; this is the studio's
output allowance, independent of the usual gameplay response size. A large
cast may need a larger allowance or fewer alternatives. The provider still
needs to support the requested output size.

The initial generation is one model request for the whole batch. Each synopsis
must have 100–500 words; the prompt aims for 180–300. Character alternatives
contain one character; group alternatives contain 2–16 characters and a
matching group roster. Invalid responses are retained with their request and
cost information, while previously accepted alternatives remain available.

Candidate numbers are stable across refinement and saved sessions. After ten
alternatives, selecting 8 and refining it adds number 11:

```text
8
Blend the working-class background from 8 with the speaking style from 2. Keep her veterinary studies and make her less immediately trusting.
/show 11
/select 11
/save chosen-dogwalker-session.json
/export exported-dogwalker
```

An ordinary number selects; ordinary text asks for one new alternative. The
refinement instruction and selected parent number remain in the session so
you can see how a candidate developed. Explicit references in prose let you
ask for blends; inspect the result rather than assuming the model followed
every detail. A session retains at most 100 alternatives.

## Studio controls

| Control | Effect |
| --- | --- |
| `8` or `/select 8` | Select an existing candidate |
| Ordinary text or `/refine <text>` | Request one new alternative |
| `/show [number]` | Inspect an alternative |
| `/list` | List the numbered alternatives |
| `/generate` | Request another batch using the concept's count |
| `/model <profile>` | Select a configured model profile |
| `/models` | List available model profiles |
| `/costs` | Inspect recorded studio costs |
| `/budget` | Inspect the current session budget |
| `/save <new.json>` | Save the current session |
| `/export <new-directory> [bindings.json]` | Export the selected candidate |
| `/help`, `/quit` | Show controls or end the session |

In the TUI, Up/Down and Home/End preview alternatives. Press `s` or Enter to
select the previewed candidate; the sidebar updates after the runner confirms
the selection. Typing a number or a refinement opens the editor, and Enter
submits it. Press `i` to start an empty edit, Esc to cancel, or Ctrl+U to clear
the draft. F2 or Tab switches between the readable portrait/full profile and the runtime
log; PgUp/PgDn scroll the detail. Ctrl+Q or Ctrl+C requests a graceful exit.
An active generation and its receipt finish before the studio closes. Long
previews are labeled when truncated; saved sessions and exports retain the
complete content.

The model profiles, tape and budget options work when launching studio:

```powershell
cargo run -p mush-bot -- --config mush-bot.toml --studio bots/studio/dogwalker.json --output .mush-bot/studio-batch --headless --models bots/models/example.json --model careful --budget bots/budgets/session.json
```

Choose paths and profile names that exist in your setup. `--headless` generates
the initial batch and exits. `--model-tape <tape.json>` supplies local simulated
responses for tests. Studio budgets count generation and refinement requests,
including unsuccessful responses. Cost limits gate future requests using
recorded receipts; they cannot cap the final charge of an in-flight request.

Authoring uses the separate `character_authoring` and `group_authoring` cost
categories in a studio ledger namespace, so it can run alongside a playing bot.
Its lifetime totals cover studio sessions for the same configured identity and
campaign namespace; they are separate from gameplay lifetime totals.

## Save, resume and export

Every output directory must be new. It receives `session-0000.json` and later
numbered checkpoints, exact `request-0001.json` captures and matching
`result-0001.json` artifacts, a final `session.json`, and `report.json` with the
session and its costs. Keep the directory when comparing model behavior.
Studio captures record authoring provenance. The gameplay `--replay` command
rejects them; authoring-response replay is not implemented.

Resume drafting from the final session or an explicit saved checkpoint:

```powershell
cargo run -p mush-bot -- --config mush-bot.toml --studio-session .mush-bot/studio-dogwalker/session.json --output .mush-bot/studio-dogwalker-next
```

Resuming preserves alternatives and numbering without automatically generating
another batch. Use `/generate` or a refinement when ready. It starts a new
request-accounting session; earlier request artifacts remain in their original
output directory.

Export writes `selected.json`, individual sheets under
`characters/character-<actor>.json`, and group membership files under
`affiliations/affiliations-<actor>.json`. These are ordinary bot JSON profiles.
The studio does not assign capabilities or silently turn drafted biography into
observed game history. Appearance is authored portrayal; prior knowledge is
authored background, and observed evidence remains separate.

To create a runnable harness, supply an exact actor-to-login/config binding:

```json
{
  "version": 1,
  "actors": [
    { "actor": "singer", "config": "../../mush-bot.toml", "login": "band+Singer" },
    { "actor": "guitarist", "config": "../../mush-bot.toml", "login": "band+Guitarist" },
    { "actor": "bassist", "config": "../../mush-bot.toml", "login": "band+Bassist" },
    { "actor": "drummer", "config": "../../mush-bot.toml", "login": "band+Drummer" }
  ]
}
```

The example in `bots/studio/band-bindings.example.json` resolves configuration
paths relative to that bindings file. Replace its logins with existing game
characters and match its actor identifiers to the selected draft exactly.
Credentials remain in your normal configuration/environment. Then export and
inspect the harness before launching it:

```text
/export exported-band bots/studio/band-bindings.example.json
```

For paths containing spaces, use a JSON argument object:

```text
/export {"output":"exported band","bindings":"my band bindings.json"}
```

```powershell
cargo run -p mush-bot -- --harness exported-band/harness.json --plan-harness --report band-plan.json
cargo run -p mush-bot -- --harness exported-band/harness.json --output .mush-bot/band-run --tui
```

The harness carries each actor's character, role and goal and starts the actors
with `/resume`. Membership files expose only their actor's private briefing;
they do not supply another member's observations or private thoughts. Studio
sessions, selected cast files and harness manifests contain the whole authored
cast, including private briefings: keep those files with the runner rather than
giving them to every character as knowledge documents.

Group drafts also support [selectively granted background](bot-social.md): use
`background` facts with explicit `known_by` actor IDs for roommate habits, familiar
contacts or other established knowledge that only some members possess. The studio
exports the same scoped affiliation format used by the harness and campaigns.

See [the bot guide](bot.md) and [session budgets](bot-budgets.md) for the runtime
controls, character profiles, organized casts and evaluation tools.
