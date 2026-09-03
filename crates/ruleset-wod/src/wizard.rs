//! The guided terminal chargen **wizard** — a friendly step-by-step front door
//! layered over the existing [`+chargen`/`+stat`](crate::chargen_cmds)
//! accounting, driven by the input-context stack.
//!
//! `+chargen/wizard` opens (or resumes) the same chargen session `+chargen`
//! does — one Unapproved [`SheetDoc`](mush_ruleset::SheetDoc), one
//! [`ChargenSession`](crate::state::ChargenSession) — then pushes a
//! [`ChargenWizard`] input context. Every step sets the **same sheet fields and
//! budget** as the command surface (it calls the shared `apply_*` helpers), so
//! the wizard and `+chargen`/`+stat` are interchangeable and a character built
//! either way submits through the one `wod-chargen` approval job.
//!
//! ## Command / context surface (mirror this for the web chargen panel)
//!
//! - Entry: `+chargen/wizard` (plain `+chargen` still works). Opens the session
//!   and pushes the context; `~q` (the stack's hard escape) saves the partial
//!   sheet and exits — nothing is discarded, so `+chargen/wizard` resumes.
//! - While the context is on top every line is a wizard command:
//!   - `next` / `back` — step navigation (the Splat step is skipped for a
//!     Mortal); `budget` — the live ledgers; `help` — the step's guidance;
//!     `review` — jump to the final step; `submit` — file for approval (only
//!     from Review); `~q` — save and exit.
//!   - a set line, whose shape is the step's (`vampire`; `concept=Detective`;
//!     `physical=7 social=5 mental=3`; `strength=4`; `Resources=3`;
//!     `clan=brujah` / `Celerity=2`; `conscience=3`).
//! - Each set routes to a shared helper: [`apply_template`], [`apply_stat`],
//!   [`apply_priorities`], [`apply_spec`], or the active splat's
//!   [`wizard_set_pool`](crate::splats::Splat::wizard_set_pool); the budget
//!   comes from [`budget_report`]. A web panel drives the identical helpers.

use std::sync::Arc;

use mush_core::{CmdError, ContextOutcome, Ctx, InputContext, World};
use mush_proto::{InputMode, OutputEvent, PromptSpec, RichText};
use mush_ruleset::schema::FieldType;

use crate::WodCmd;
use crate::chargen_cmds::{
    apply_priorities, apply_stat, apply_template, budget_report, open_chargen_session,
    parse_priority_assignment, submit,
};
use crate::config::{PRIORITY_ABIL, PRIORITY_ATTR};

/// One wizard step.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Step {
    Template,
    Concept,
    AttrPriorities,
    Attributes,
    AbilPriorities,
    Abilities,
    Backgrounds,
    Splat,
    Virtues,
    Freebies,
    Review,
}

/// The steps in order.
const ORDER: &[Step] = &[
    Step::Template,
    Step::Concept,
    Step::AttrPriorities,
    Step::Attributes,
    Step::AbilPriorities,
    Step::Abilities,
    Step::Backgrounds,
    Step::Splat,
    Step::Virtues,
    Step::Freebies,
    Step::Review,
];

impl Step {
    /// The short title shown in the prompt / headers.
    fn title(self) -> &'static str {
        match self {
            Step::Template => "Template",
            Step::Concept => "Concept",
            Step::AttrPriorities => "Attribute Priorities",
            Step::Attributes => "Attributes",
            Step::AbilPriorities => "Ability Priorities",
            Step::Abilities => "Abilities",
            Step::Backgrounds => "Backgrounds",
            Step::Splat => "Template Traits",
            Step::Virtues => "Virtues",
            Step::Freebies => "Freebies",
            Step::Review => "Review & Submit",
        }
    }
}

/// The guided chargen input context.
pub(crate) struct ChargenWizard {
    cmd: Arc<WodCmd>,
    /// Index into [`ORDER`].
    idx: usize,
}

/// Start (or resume) the guided wizard: open the chargen session, greet, show
/// the first step, and push the context.
pub(crate) fn start_wizard(cmd: Arc<WodCmd>, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    // Same session/sheet the classic +chargen opens; refuses an approved /
    // submitted sheet with the same message.
    open_chargen_session(&cmd, ctx)?;
    let me = ctx.actor;
    ctx.out.info(me, "=== Guided Character Creation ===");
    ctx.out.info(
        me,
        "I'll walk you through your character one step at a time. At any prompt: \
         'next' continues, 'back' goes back, 'budget' shows your points, 'help' \
         explains the step, and '~q' saves and exits (resume later with \
         +chargen/wizard).",
    );
    let wiz = ChargenWizard {
        cmd: Arc::clone(&cmd),
        idx: 0,
    };
    wiz.show_step(ctx);
    ctx.push_input_context(Box::new(wiz));
    Ok(())
}

impl InputContext for ChargenWizard {
    fn prompt(&self, _w: &World) -> PromptSpec {
        PromptSpec {
            mode: InputMode::Normal,
            text: Some(RichText::plain(format!(
                "Chargen [{}/{}] {}>",
                self.idx + 1,
                ORDER.len(),
                self.step().title()
            ))),
        }
    }

    fn feed(&mut self, ctx: &mut Ctx<'_>, line: &str) -> ContextOutcome {
        let me = ctx.actor;
        let line = line.trim();
        match line.to_ascii_lowercase().as_str() {
            "" => ContextOutcome::Consumed,
            "next" | "n" | "done" | "continue" => {
                self.advance(ctx, true);
                ContextOutcome::Consumed
            }
            "back" | "prev" | "previous" => {
                self.advance(ctx, false);
                ContextOutcome::Consumed
            }
            "budget" | "status" | "points" => {
                self.show_budget(ctx);
                ContextOutcome::Consumed
            }
            "help" | "?" => {
                self.show_step(ctx);
                ContextOutcome::Consumed
            }
            "review" => {
                self.jump_to(ctx, Step::Review);
                ContextOutcome::Consumed
            }
            "submit" => {
                if self.step() == Step::Review {
                    match submit(&self.cmd, ctx) {
                        Ok(()) => ContextOutcome::Pop,
                        Err(e) => {
                            ctx.out.error(me, e.to_string());
                            ContextOutcome::Consumed
                        }
                    }
                } else {
                    ctx.out.info(
                        me,
                        "Reach the final step first ('review' jumps there), then 'submit'.",
                    );
                    ContextOutcome::Consumed
                }
            }
            "abort" | "cancel" | "exit" | "quit" => {
                ctx.out.info(
                    me,
                    "Chargen saved. Resume any time with +chargen/wizard (or use +chargen).",
                );
                ContextOutcome::Pop
            }
            _ => {
                match self.apply_input(ctx, line) {
                    Ok(msg) => {
                        ctx.out.info(me, msg);
                        // Re-show the live budget after any allocation.
                        if self.step_allocates() {
                            self.show_budget(ctx);
                        }
                    }
                    Err(e) => ctx.out.error(me, e.to_string()),
                }
                ContextOutcome::Consumed
            }
        }
    }
}

impl ChargenWizard {
    /// The current step.
    fn step(&self) -> Step {
        ORDER[self.idx.min(ORDER.len() - 1)]
    }

    /// Whether the current step is a point-allocation step (so a set should be
    /// followed by a fresh budget display).
    fn step_allocates(&self) -> bool {
        matches!(
            self.step(),
            Step::Attributes
                | Step::Abilities
                | Step::Backgrounds
                | Step::Splat
                | Step::Virtues
                | Step::Freebies
                | Step::AttrPriorities
                | Step::AbilPriorities
        )
    }

    /// The current template id on the sheet (`"mortal"` if none/unknown).
    fn template(&self, w: &World, actor: mush_core::ObjId) -> String {
        self.cmd
            .read(w, actor)
            .and_then(|d| d.text("template").map(str::to_string))
            .unwrap_or_else(|| "mortal".to_string())
    }

    /// Move to the next/previous step, skipping the Splat step for a Mortal.
    fn advance(&mut self, ctx: &mut Ctx<'_>, forward: bool) {
        let me = ctx.actor;
        let mortal = self.template(ctx.w, me) == "mortal";
        let mut i = self.idx;
        loop {
            if forward {
                if i + 1 >= ORDER.len() {
                    ctx.out.info(
                        me,
                        "You're at the final step. Type 'submit' to file for approval, \
                         or 'back' to revise.",
                    );
                    return;
                }
                i += 1;
            } else {
                if i == 0 {
                    ctx.out.info(me, "You're at the first step.");
                    return;
                }
                i -= 1;
            }
            // Skip the splat step for a mortal.
            if ORDER[i] == Step::Splat && mortal {
                continue;
            }
            break;
        }
        self.idx = i;
        self.show_step(ctx);
        self.reprompt(ctx);
    }

    /// Jump directly to a step (used by `review`).
    fn jump_to(&mut self, ctx: &mut Ctx<'_>, target: Step) {
        if let Some(pos) = ORDER.iter().position(|&s| s == target) {
            self.idx = pos;
            self.show_step(ctx);
            self.reprompt(ctx);
        }
    }

    /// Re-emit the prompt so it reflects the current step.
    fn reprompt(&self, ctx: &mut Ctx<'_>) {
        let spec = self.prompt(ctx.w);
        ctx.out.to_actor(ctx.actor, OutputEvent::Prompt(spec));
    }

    /// Print the current step's header + guidance (and the live budget on
    /// allocation steps).
    fn show_step(&self, ctx: &mut Ctx<'_>) {
        let me = ctx.actor;
        let step = self.step();
        ctx.out.info(me, format!("--- {} ---", step.title()));
        for line in self.guidance(ctx) {
            ctx.out.info(me, line);
        }
        if self.step_allocates() {
            self.show_budget(ctx);
        }
    }

    /// The guidance lines for the current step (template/enum options are read
    /// live off the active schema).
    fn guidance(&self, ctx: &mut Ctx<'_>) -> Vec<String> {
        let me = ctx.actor;
        match self.step() {
            Step::Template => {
                let mut v = vec![
                    "Choose the kind of character you're making (type the name, then 'next'):"
                        .to_string(),
                ];
                for id in self.cmd.schemas().ids() {
                    v.push(format!("  {}  --  {}", id, template_blurb(id)));
                }
                v.push("e.g.  vampire".to_string());
                v
            }
            Step::Concept => vec![
                "Describe who your character is. Set each with <field>=<text>:".to_string(),
                "  concept=<a short phrase>   nature=<archetype>   demeanor=<archetype>"
                    .to_string(),
                "e.g.  concept=Jaded Detective".to_string(),
            ],
            Step::AttrPriorities => vec![
                "Rank your three Attribute categories 7 / 5 / 3 (primary/secondary/tertiary), \
                 all on one line:"
                    .to_string(),
                format!(
                    "  physical=<n> social=<n> mental=<n>   (the three must be {}, {}, {})",
                    PRIORITY_ATTR[0], PRIORITY_ATTR[1], PRIORITY_ATTR[2]
                ),
                "e.g.  physical=7 social=5 mental=3".to_string(),
            ],
            Step::Attributes => vec![
                "Spend your Attribute dots. Everyone starts at 1; set the final rating with \
                 <attribute>=<n>:"
                    .to_string(),
                "  strength dexterity stamina | charisma manipulation appearance | \
                 perception intelligence wits"
                    .to_string(),
                "e.g.  strength=4".to_string(),
            ],
            Step::AbilPriorities => vec![
                "Rank your three Ability groups 13 / 9 / 5, all on one line:".to_string(),
                format!(
                    "  talents=<n> skills=<n> knowledges=<n>   (the three must be {}, {}, {})",
                    PRIORITY_ABIL[0], PRIORITY_ABIL[1], PRIORITY_ABIL[2]
                ),
                "e.g.  talents=13 skills=9 knowledges=5".to_string(),
            ],
            Step::Abilities => vec![
                "Buy Abilities (cap 3 before freebies) with <ability>=<n>. Add a specialty with \
                 spec <ability>=<text>:"
                    .to_string(),
                "e.g.  brawl=3   or   spec brawl=Kickboxing".to_string(),
            ],
            Step::Backgrounds => vec![
                "Spend your Background dots with <name>=<dots> (open-ended list):".to_string(),
                "e.g.  Resources=3   Allies=2".to_string(),
            ],
            Step::Splat => self.splat_guidance(ctx),
            Step::Virtues => {
                let mut v = vec![
                    "Rate your Virtues with <virtue>=<n> (each starts at 1; more cost freebies):"
                        .to_string(),
                ];
                v.push(format!("  {}", self.virtue_fields(ctx).join("  ")));
                v.push("e.g.  conscience=3".to_string());
                v
            }
            Step::Freebies => vec![
                "Spend your remaining freebie points on anything beyond your pools, using the \
                 same syntax as before:"
                    .to_string(),
                "  <attribute>=<n>  <ability>=<n>  <name>=<dots>  willpower=<n>  <virtue>=<n>"
                    .to_string(),
                "Maps/pools use <field>:<name>=<n>, e.g. backgrounds:Resources=4 or \
                 disciplines:Celerity=3."
                    .to_string(),
            ],
            Step::Review => {
                let (_, issues) = budget_report(&self.cmd, ctx).unwrap_or_default();
                let mut v = Vec::new();
                if issues.is_empty() {
                    v.push(
                        "Everything balances. Type 'submit' to file your character for staff \
                         approval."
                            .to_string(),
                    );
                } else {
                    v.push(format!(
                        "{} item(s) still need attention before you can submit:",
                        issues.len()
                    ));
                    for i in &issues {
                        v.push(format!("  - {i}"));
                    }
                    v.push("Use 'back' to revise, or 'budget' to see the full ledger.".to_string());
                }
                let _ = me;
                v
            }
        }
    }

    /// The splat-step guidance for the active template (fields + pool, read off
    /// the schema so enum options are accurate).
    fn splat_guidance(&self, ctx: &mut Ctx<'_>) -> Vec<String> {
        let me = ctx.actor;
        let Some(doc) = self.cmd.read(ctx.w, me) else {
            return vec!["No sheet yet — pick a template first.".to_string()];
        };
        let splat = self.cmd.splat_for_doc(&doc);
        let schema = self.cmd.schema_for_doc(&doc);
        let mut v = vec!["Set your template-specific traits:".to_string()];
        for field in splat.wizard_splat_fields() {
            let label = schema
                .field(field)
                .map_or_else(|| (*field).to_string(), |f| f.label.clone());
            match schema.field(field).map(|f| &f.ty) {
                Some(FieldType::Enum { options }) => {
                    let opts: Vec<&str> = options.iter().map(smol_str::SmolStr::as_str).collect();
                    v.push(format!("  {field}=<{}>", opts.join("|")));
                    let _ = label;
                }
                _ => v.push(format!("  {field}=<value>   ({label})")),
            }
        }
        if let Some(pool) = splat.wizard_pool_field() {
            v.push(format!(
                "  Allocate your {pool} pool with <name>=<dots> (e.g. Celerity=2), or \
                 {pool}:<name>=<dots>."
            ));
        }
        v
    }

    /// The virtue field ids for the active template (Path vampires swap two).
    fn virtue_fields(&self, ctx: &mut Ctx<'_>) -> Vec<String> {
        let Some(doc) = self.cmd.read(ctx.w, ctx.actor) else {
            return vec!["conscience".into(), "self_control".into(), "courage".into()];
        };
        let schema = self.cmd.schema_for_doc(&doc);
        schema
            .field("conviction")
            .filter(|_| {
                doc.text("path")
                    .map(|p| !p.trim().is_empty() && !p.eq_ignore_ascii_case("humanity"))
                    .unwrap_or(false)
            })
            .map_or_else(
                || {
                    vec![
                        "conscience".to_string(),
                        "self_control".to_string(),
                        "courage".to_string(),
                    ]
                },
                |_| {
                    vec![
                        "conviction".to_string(),
                        "instinct".to_string(),
                        "courage".to_string(),
                    ]
                },
            )
    }

    /// Print the live budget ledgers (identical numbers to `+chargen/status`).
    fn show_budget(&self, ctx: &mut Ctx<'_>) {
        let me = ctx.actor;
        let (ledgers, issues) = match budget_report(&self.cmd, ctx) {
            Ok(r) => r,
            Err(e) => {
                ctx.out.best_effort_error(me, e.to_string());
                return;
            }
        };
        ctx.out.sys_to_actor(me, "wod", "Points:");
        for l in &ledgers {
            let over = if l.spent > l.total && l.name != "freebies" {
                format!(" (+{} to freebies)", l.spent - l.total)
            } else {
                String::new()
            };
            ctx.out.sys_to_actor(
                me,
                "wod",
                format!(
                    "  {:<14} {}/{}{over}",
                    l.label,
                    l.spent.min(l.total),
                    l.total
                ),
            );
        }
        if issues.is_empty() {
            ctx.out.info(me, "Balances — ready to submit.");
        } else {
            ctx.out
                .info(me, format!("{} item(s) still to resolve.", issues.len()));
        }
    }

    /// Apply one step-specific set line, returning a confirmation.
    fn apply_input(&self, ctx: &mut Ctx<'_>, line: &str) -> Result<String, CmdError> {
        match self.step() {
            Step::Template => {
                let id = strip_field_prefix(line, "template");
                let id = apply_template(&self.cmd, ctx, id)?;
                Ok(format!("Template set to {id}. Type 'next' to continue."))
            }
            Step::Concept => {
                let (field, value) = split_eq(line)?;
                if !matches!(
                    field.to_ascii_lowercase().as_str(),
                    "concept" | "nature" | "demeanor"
                ) {
                    return Err(CmdError::msg(
                        "Set concept, nature, or demeanor (e.g. concept=Jaded Detective).",
                    ));
                }
                apply_stat(&self.cmd, ctx, &field, None, &value)?;
                Ok(format!("{field} set."))
            }
            Step::AttrPriorities => {
                let a = parse_priority_assignment(
                    line,
                    &["physical", "social", "mental"],
                    PRIORITY_ATTR,
                )?;
                apply_priorities(&self.cmd, ctx, true, a)?;
                Ok("Attribute priorities set.".to_string())
            }
            Step::AbilPriorities => {
                let a = parse_priority_assignment(
                    line,
                    &["talents", "skills", "knowledges"],
                    PRIORITY_ABIL,
                )?;
                apply_priorities(&self.cmd, ctx, false, a)?;
                Ok("Ability priorities set.".to_string())
            }
            Step::Attributes | Step::Abilities | Step::Virtues => {
                // `spec <ability>=<text>` shortcut in the Abilities step.
                if self.step() == Step::Abilities
                    && let Some(rest) = strip_verb(line, "spec")
                {
                    let (ability, text) = split_eq(rest)?;
                    crate::chargen_cmds::apply_spec(&self.cmd, ctx, &ability, &text)?;
                    return Ok(format!("Specialty added to {ability}."));
                }
                let (field, value) = split_eq(line)?;
                apply_stat(&self.cmd, ctx, &field, None, &value)?;
                Ok(format!("{field} = {value}."))
            }
            Step::Backgrounds => {
                let (name, dots) = split_eq(line)?;
                apply_stat(&self.cmd, ctx, "backgrounds", Some(&name), &dots)?;
                Ok(format!("Background {name} = {dots}."))
            }
            Step::Splat => self.apply_splat(ctx, line),
            Step::Freebies => self.apply_generic(ctx, line),
            Step::Review => Err(CmdError::msg(
                "Type 'submit' to file for approval, or 'back' to revise.",
            )),
        }
    }

    /// Apply a splat-step set: a splat scalar field, an explicit
    /// `pool:name=dots`, or a bare `name=dots` power-pool entry.
    fn apply_splat(&self, ctx: &mut Ctx<'_>, line: &str) -> Result<String, CmdError> {
        let doc = self
            .cmd
            .read(ctx.w, ctx.actor)
            .ok_or_else(|| CmdError::msg("No sheet yet — pick a template first."))?;
        let splat = self.cmd.splat_for_doc(&doc);
        let (lhs, value) = split_eq(line)?;
        let (field, key) = match lhs.split_once(':') {
            Some((f, k)) => (f.trim().to_string(), Some(k.trim().to_string())),
            None => (lhs.clone(), None),
        };
        let pool = splat.wizard_pool_field();
        // Explicit pool syntax: disciplines:Celerity=2 / spheres:Forces=3.
        if let (Some(p), Some(k)) = (pool, &key)
            && field.eq_ignore_ascii_case(p)
        {
            let rating = parse_dots(&value)?;
            splat.wizard_set_pool(&self.cmd, ctx, k, rating)?;
            return Ok(format!("{k} = {rating}."));
        }
        // A splat scalar field: clan=..., arete=..., breed=...
        if splat
            .wizard_splat_fields()
            .iter()
            .any(|f| f.eq_ignore_ascii_case(&field))
        {
            apply_stat(&self.cmd, ctx, &field, None, &value)?;
            return Ok(format!("{field} set."));
        }
        // A bare power-pool entry: Celerity=2 (the field IS the entry name).
        if let (Some(_), None) = (pool, &key) {
            let rating = parse_dots(&value)?;
            splat.wizard_set_pool(&self.cmd, ctx, &field, rating)?;
            return Ok(format!("{field} = {rating}."));
        }
        Err(CmdError::msg(
            "That isn't a valid pick for this template. Type 'help' to see the options.",
        ))
    }

    /// Apply a free-form `field[:key]=value` set (the Freebies step), routing a
    /// splat pool field to the splat's own setter.
    fn apply_generic(&self, ctx: &mut Ctx<'_>, line: &str) -> Result<String, CmdError> {
        let (lhs, value) = split_eq(line)?;
        let (field, key) = match lhs.split_once(':') {
            Some((f, k)) => (f.trim().to_string(), Some(k.trim().to_string())),
            None => (lhs.clone(), None),
        };
        let doc = self
            .cmd
            .read(ctx.w, ctx.actor)
            .ok_or_else(|| CmdError::msg("No sheet yet."))?;
        let splat = self.cmd.splat_for_doc(&doc);
        if let (Some(p), Some(k)) = (splat.wizard_pool_field(), &key)
            && field.eq_ignore_ascii_case(p)
        {
            let rating = parse_dots(&value)?;
            splat.wizard_set_pool(&self.cmd, ctx, k, rating)?;
            return Ok(format!("{k} = {rating}."));
        }
        apply_stat(&self.cmd, ctx, &field, key.as_deref(), &value)?;
        Ok(match key {
            Some(k) => format!("{field}:{k} = {value}."),
            None => format!("{field} = {value}."),
        })
    }
}

/// A one-line description of a template for the wizard's first step.
fn template_blurb(id: &str) -> &'static str {
    match id {
        "mortal" => "an ordinary human, no supernatural template",
        "vampire" => "Kindred of the Masquerade — Clan, Disciplines, Blood Pool",
        "mage" => "an Awakened will-worker — Tradition, Arete, nine Spheres",
        "werewolf" => "a Garou shapeshifter — Breed, Auspice, Tribe, Rage & Gnosis",
        "changeling" => "one of the Kithain — Kith, Seeming, Court, Arts & Realms, Glamour",
        "ghoul" => "a mortal bound to a vampire — Domitor's Clan, Disciplines, Blood Pool",
        "kinfolk" => "a Garou's mortal kin — Tribe, Gnosis, level-1 Gifts, Delirium immunity",
        _ => "a character template",
    }
}

/// Split `<lhs>=<rhs>` (first `=`), both trimmed; error if there is no `=`.
fn split_eq(line: &str) -> Result<(String, String), CmdError> {
    line.split_once('=')
        .map(|(l, r)| (l.trim().to_string(), r.trim().to_string()))
        .ok_or_else(|| CmdError::msg("Use <field>=<value> (type 'help' for this step)."))
}

/// Parse a dot rating (a small non-negative integer).
fn parse_dots(v: &str) -> Result<i32, CmdError> {
    v.trim()
        .parse::<i32>()
        .ok()
        .filter(|n| *n >= 0)
        .ok_or_else(|| CmdError::msg(format!("{v:?} is not a dot rating.")))
}

/// Strip an optional leading `field` / `field=` / `field ` from `line`,
/// returning the remainder (used so the Template step accepts `vampire`,
/// `template vampire`, and `template=vampire`).
fn strip_field_prefix<'a>(line: &'a str, field: &str) -> &'a str {
    let t = line.trim();
    if let Some(rest) = t.strip_prefix(field).or_else(|| {
        t.get(..field.len())
            .filter(|h| h.eq_ignore_ascii_case(field))
            .map(|_| &t[field.len()..])
    }) {
        let rest = rest.trim_start_matches(['=', ' ']).trim();
        if !rest.is_empty() {
            return rest;
        }
    }
    t
}

/// Case-insensitively strip a leading `verb ` from `line`, returning the
/// remainder when the verb is followed by whitespace.
fn strip_verb<'a>(line: &'a str, verb: &str) -> Option<&'a str> {
    let head = line.get(..verb.len())?;
    if !head.eq_ignore_ascii_case(verb) {
        return None;
    }
    let rest = &line[verb.len()..];
    rest.starts_with(char::is_whitespace).then(|| rest.trim())
}
