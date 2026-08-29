# Character generation

Your character is described by a **sheet** — nine Attributes, thirty Abilities,
Backgrounds, Virtues, Willpower, and (for a splat) your template's own powers.
Chargen is where you build that sheet by spending the game's starting point
pools; when it balances you **submit it for a storyteller's approval**, and once
approved you play. Nothing you allocate is spent for real until you submit, and
submitting never destroys anything — you can keep editing until a storyteller
picks the job up.

There are two ways to build a character. They edit the **same** sheet and the
**same** point budget, so you can switch between them freely.

## The guided wizard (recommended for new players)

  +chargen/wizard            Start (or resume) the step-by-step wizard.

The wizard walks you through one decision at a time — template, concept,
Attribute priorities and dots, Ability priorities and dots, Backgrounds, your
splat's powers, Virtues, freebies, and a final review — showing your remaining
points after every change. At any prompt:

  next        move on to the next step (skips the splat step for a Mortal)
  back        return to the previous step
  budget      show your remaining points for the current step
  help        explain the current step
  review      jump to the final review
  submit      file the finished sheet for approval (from the review step)
  ~q          save and leave the wizard — your partial sheet is kept

Set lines take the shape the step asks for, e.g. `vampire`; `concept=Detective`;
`physical=7 social=5 mental=3`; `strength=4`; `Resources=3`; `clan=brujah`;
`Celerity=2`; `conscience=3`. Leaving with `~q` never discards anything —
`+chargen/wizard` resumes exactly where you stopped.

## The classic commands

Prefer typing? The same sheet is driven directly:

  +chargen                   Start (or resume) chargen; creates your sheet.
  +chargen/template <mortal|vampire|mage|werewolf>
                             Choose your template/splat (re-shapes the sheet).
  +chargen/attr physical=7 social=5 mental=3     Set Attribute priorities.
  +chargen/abil talents=13 skills=9 knowledges=5 Set Ability priorities.
  +stat <field>[:<key>]=<value>                  Set one trait, e.g.
                             `+stat strength=4`, `+stat Disciplines:Celerity=2`.
  +spec <field>=<specialty>  Add a specialty to a trait.
  +chargen/status            Show your remaining budget across every pool.
  +chargen/submit            Finalize and file for staff approval.

While a sheet is submitted it is **locked** (Pending) until a storyteller
approves or bounces the job; you cannot re-open it until then. On approval the
sheet is stamped and you keep it; raise traits further afterward with XP
(`+xp/spend`, `help wod`).

## When chargen is required to play (`chargen_required`)

Some games gate play behind an approved sheet. On those servers a brand-new
character is placed in a quiet **chargen area** and dropped straight into the
wizard on connect. Until a storyteller approves you, you are held to a small
command set — chargen, `look`, `say`/`pose`, `help`, and `who` — and everything
else (moving onto the grid, speaking on a channel, `+roll`, building) waits.
The moment your `wod-chargen` job is approved the restriction lifts, you are
moved to the game's live start room, and you have full access. If your game does
not require chargen (the default), you play immediately and can build your sheet
whenever you like.

See also: `help wod` (the sheet, rolling, splats), `help jobs` (how approval
queues work), `help getting-started`.
