//! The guided SR5 chargen **wizard** — a step-by-step front door over the
//! `+chargen`/`+stat` accounting, driven by the input-context stack.
//!
//! `+chargen/wizard` opens (or resumes) the same session `+chargen` does — one
//! Unapproved [`SheetDoc`](mush_ruleset::SheetDoc), one
//! [`ChargenSession`](crate::state::ChargenSession) — then pushes a
//! [`ChargenWizard`] input context. Every step calls the shared `apply_*`
//! helpers, so the wizard and the command surface are interchangeable and a
//! character built either way submits through the one `shadowrun-chargen` job.

use std::sync::Arc;

use mush_core::{CmdError, ContextOutcome, Ctx, InputContext, World};
use mush_proto::{InputMode, OutputEvent, PromptSpec, RichText};

use crate::Sr5Cmd;
use crate::chargen_cmds::{
    apply_priorities, apply_spec, apply_stat, budget_report, open_chargen_session,
    parse_priority_assignment, submit,
};

/// One wizard step.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Step {
    Priorities,
    Metatype,
    Attributes,
    Magic,
    Skills,
    Resources,
    Finishing,
    Review,
}

const ORDER: &[Step] = &[
    Step::Priorities,
    Step::Metatype,
    Step::Attributes,
    Step::Magic,
    Step::Skills,
    Step::Resources,
    Step::Finishing,
    Step::Review,
];

impl Step {
    fn title(self) -> &'static str {
        match self {
            Step::Priorities => "Priorities",
            Step::Metatype => "Metatype",
            Step::Attributes => "Attributes",
            Step::Magic => "Magic / Resonance",
            Step::Skills => "Skills",
            Step::Resources => "Resources & Gear",
            Step::Finishing => "Finishing Touches",
            Step::Review => "Review & Submit",
        }
    }

    fn allocates(self) -> bool {
        !matches!(self, Step::Review)
    }
}

/// The guided chargen input context.
pub(crate) struct ChargenWizard {
    cmd: Arc<Sr5Cmd>,
    idx: usize,
}

/// Start (or resume) the guided wizard.
pub(crate) fn start_wizard(cmd: Arc<Sr5Cmd>, ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    open_chargen_session(&cmd, ctx)?;
    let me = ctx.actor;
    ctx.out.info(me, "=== Guided Runner Creation (SR5) ===");
    ctx.out.info(
        me,
        "I'll walk you through your runner one step at a time. At any prompt: \
         'next' continues, 'back' goes back, 'budget' shows your points, 'help' \
         explains the step, and '~q' saves and exits (resume with +chargen/wizard).",
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
                        "Reach Review first ('review' jumps there), then 'submit'.",
                    );
                    ContextOutcome::Consumed
                }
            }
            "abort" | "cancel" | "exit" | "quit" => {
                ctx.out.info(
                    me,
                    "Chargen saved. Resume with +chargen/wizard (or +chargen).",
                );
                ContextOutcome::Pop
            }
            _ => {
                match self.apply_input(ctx, line) {
                    Ok(msg) => {
                        ctx.out.info(me, msg);
                        if self.step().allocates() {
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
    fn step(&self) -> Step {
        ORDER[self.idx.min(ORDER.len() - 1)]
    }

    fn advance(&mut self, ctx: &mut Ctx<'_>, forward: bool) {
        let me = ctx.actor;
        if forward {
            if self.idx + 1 >= ORDER.len() {
                ctx.out.info(
                    me,
                    "You're at Review. 'submit' files for approval, 'back' revises.",
                );
                return;
            }
            self.idx += 1;
        } else {
            if self.idx == 0 {
                ctx.out.info(me, "You're at the first step.");
                return;
            }
            self.idx -= 1;
        }
        self.show_step(ctx);
        self.reprompt(ctx);
    }

    fn jump_to(&mut self, ctx: &mut Ctx<'_>, target: Step) {
        if let Some(pos) = ORDER.iter().position(|&s| s == target) {
            self.idx = pos;
            self.show_step(ctx);
            self.reprompt(ctx);
        }
    }

    fn reprompt(&self, ctx: &mut Ctx<'_>) {
        let spec = self.prompt(ctx.w);
        ctx.out.to_actor(ctx.actor, OutputEvent::Prompt(spec));
    }

    fn show_step(&self, ctx: &mut Ctx<'_>) {
        let me = ctx.actor;
        ctx.out.info(me, format!("--- {} ---", self.step().title()));
        for line in self.guidance(ctx) {
            ctx.out.info(me, line);
        }
        if self.step().allocates() {
            self.show_budget(ctx);
        }
    }

    fn guidance(&self, ctx: &mut Ctx<'_>) -> Vec<String> {
        match self.step() {
            Step::Priorities => vec![
                "Assign A–E (each once) across the five categories, all on one line:".to_string(),
                "  metatype=A attributes=B magic=C skills=D resources=E".to_string(),
            ],
            Step::Metatype => vec![
                "Choose your metatype with metatype=<name>:".to_string(),
                "  human  elf  dwarf  ork  troll".to_string(),
            ],
            Step::Attributes => vec![
                "Spend your Attribute points. Set the final rating with <attr>=<n>:".to_string(),
                "  body agility reaction strength | charisma intuition logic willpower".to_string(),
                "Special (from the Metatype pool): edge=<n>, and magic=<n>/resonance=<n> if \
                 Awakened. One attribute may reach its metatype maximum."
                    .to_string(),
            ],
            Step::Magic => vec![
                "If Awakened, set awakened_type=<magician|adept|mystic_adept|aspected_magician|\
                 technomancer> and tradition=<hermetic|shamanic|…>."
                    .to_string(),
                "Learn spells with spell=<name>, complex forms with complexform=<name>, \
                 adept powers with adept_powers:<name>=<level>."
                    .to_string(),
                "Mundane? Just type 'next'.".to_string(),
            ],
            Step::Skills => vec![
                "Buy active skills with <skill>=<n> (e.g. pistols=4), skill groups with \
                 group:<name>=<n>, and specializations with spec <skill>=<text>."
                    .to_string(),
                "Knowledge/Language use their own pool: knowledge:<name>=<n>, \
                 language:<name>=<n>."
                    .to_string(),
            ],
            Step::Resources => vec![
                "Set starting nuyen with nuyen=<amount> (within your Resource priority) and \
                 augmentations with augmentations:<name>=<essence×100> (Essence updates at \
                 submit)."
                    .to_string(),
            ],
            Step::Finishing => vec![
                "Spend Karma (25) on qualities and contacts: positive_qualities:<name>=<karma>, \
                 negative_qualities:<name>=<karma> (refunds Karma), \
                 contacts_connection:<name>=<n>, contacts_loyalty:<name>=<n>."
                    .to_string(),
            ],
            Step::Review => {
                let (_, issues) = budget_report(&self.cmd, ctx).unwrap_or_default();
                let mut v = Vec::new();
                if issues.is_empty() {
                    v.push("Everything balances. Type 'submit' to file for approval.".to_string());
                } else {
                    v.push(format!("{} item(s) still need attention:", issues.len()));
                    for i in issues.iter().take(10) {
                        v.push(format!("  - {i}"));
                    }
                    v.push("'back' to revise, 'budget' for the full ledger.".to_string());
                }
                v
            }
        }
    }

    fn show_budget(&self, ctx: &mut Ctx<'_>) {
        let me = ctx.actor;
        let (ledgers, issues) = match budget_report(&self.cmd, ctx) {
            Ok(r) => r,
            Err(e) => {
                ctx.out.best_effort_error(me, e.to_string());
                return;
            }
        };
        ctx.out.sys_to_actor(me, "sr", "Points:");
        for l in &ledgers {
            let over = if l.spent > l.total { " OVER" } else { "" };
            ctx.out.sys_to_actor(
                me,
                "sr",
                format!("  {:<24} {}/{}{over}", l.label, l.spent, l.total),
            );
        }
        if issues.is_empty() {
            ctx.out.info(me, "Balances — ready to submit.");
        } else {
            ctx.out
                .info(me, format!("{} item(s) still to resolve.", issues.len()));
        }
    }

    /// Apply one step-specific set line.
    fn apply_input(&self, ctx: &mut Ctx<'_>, line: &str) -> Result<String, CmdError> {
        match self.step() {
            Step::Priorities => {
                let p = parse_priority_assignment(line)?;
                apply_priorities(&self.cmd, ctx, &p)?;
                Ok("Priorities set.".to_string())
            }
            Step::Metatype => {
                let value = strip_field_prefix(line, "metatype");
                apply_stat(&self.cmd, ctx, "metatype", None, value)?;
                Ok(format!("Metatype set to {value}."))
            }
            Step::Attributes | Step::Finishing => self.apply_generic(ctx, line),
            Step::Magic => self.apply_magic(ctx, line),
            Step::Skills => self.apply_skills(ctx, line),
            Step::Resources => self.apply_generic(ctx, line),
            Step::Review => Err(CmdError::msg(
                "Type 'submit' to file for approval, or 'back'.",
            )),
        }
    }

    /// Skills step: `spec <skill>=<text>`, `group:<name>=<n>`,
    /// `knowledge:<name>=<n>`, `language:<name>=<n>`, or `<skill>=<n>`.
    fn apply_skills(&self, ctx: &mut Ctx<'_>, line: &str) -> Result<String, CmdError> {
        if let Some(rest) = strip_verb(line, "spec") {
            let (skill, text) = split_eq(rest)?;
            apply_spec(&self.cmd, ctx, &skill, &text)?;
            return Ok(format!("Specialization added to {skill}."));
        }
        let (lhs, value) = split_eq(line)?;
        let (field, key) = match lhs.split_once(':') {
            Some((f, k)) => (f.trim().to_ascii_lowercase(), Some(k.trim().to_string())),
            None => (lhs.trim().to_ascii_lowercase(), None),
        };
        let (target_field, target_key) = match (field.as_str(), key) {
            ("group" | "skill_groups", Some(k)) => (crate::schema::SKILL_GROUPS_FIELD, k),
            ("knowledge" | "knowledge_skills", Some(k)) => {
                (crate::schema::KNOWLEDGE_SKILLS_FIELD, k)
            }
            ("language" | "language_skills", Some(k)) => (crate::schema::LANGUAGE_SKILLS_FIELD, k),
            (skill, None) => (crate::schema::ACTIVE_SKILLS_FIELD, skill.to_string()),
            _ => {
                return Err(CmdError::msg(
                    "Use <skill>=<n>, group:<name>=<n>, knowledge:<name>=<n>, or spec <skill>=<text>.",
                ));
            }
        };
        apply_stat(&self.cmd, ctx, target_field, Some(&target_key), &value)?;
        Ok(format!("{target_key} = {value}."))
    }

    /// Magic step: `spell=<name>`, `complexform=<name>`, `adept_powers:<n>=<l>`,
    /// or a scalar (`awakened_type`, `tradition`, `magic`, `resonance`).
    fn apply_magic(&self, ctx: &mut Ctx<'_>, line: &str) -> Result<String, CmdError> {
        let (lhs, value) = split_eq(line)?;
        let (field, key) = match lhs.split_once(':') {
            Some((f, k)) => (f.trim().to_ascii_lowercase(), Some(k.trim().to_string())),
            None => (lhs.trim().to_ascii_lowercase(), None),
        };
        match (field.as_str(), key) {
            ("spell" | "spells", _) => {
                self.learn_list(ctx, "spells", &value)?;
                Ok(format!("Learned spell {value}."))
            }
            ("complexform" | "complex_forms", _) => {
                self.learn_list(ctx, "complex_forms", &value)?;
                Ok(format!("Gained complex form {value}."))
            }
            ("adept_powers", Some(k)) => {
                apply_stat(&self.cmd, ctx, "adept_powers", Some(&k), &value)?;
                Ok(format!("{k} = {value}."))
            }
            (f, None) => {
                apply_stat(&self.cmd, ctx, f, None, &value)?;
                Ok(format!("{f} set."))
            }
            _ => Err(CmdError::msg("Type 'help' to see this step's options.")),
        }
    }

    /// A free-form `field[:key]=value` set.
    fn apply_generic(&self, ctx: &mut Ctx<'_>, line: &str) -> Result<String, CmdError> {
        let (lhs, value) = split_eq(line)?;
        let (field, key) = match lhs.split_once(':') {
            Some((f, k)) => (f.trim().to_string(), Some(k.trim().to_string())),
            None => (lhs.clone(), None),
        };
        apply_stat(&self.cmd, ctx, &field, key.as_deref(), &value)?;
        Ok(match key {
            Some(k) => format!("{field}:{k} = {value}."),
            None => format!("{field} = {value}."),
        })
    }

    /// Append a value to a list field (spells / complex forms) on the sheet.
    fn learn_list(&self, ctx: &mut Ctx<'_>, field: &str, value: &str) -> Result<(), CmdError> {
        let actor = ctx.actor;
        let mut doc = self.cmd.read_or_default_ctx(ctx, actor)?;
        let mut list: Vec<serde_json::Value> = doc.list(field).cloned().unwrap_or_default();
        if list
            .iter()
            .filter_map(serde_json::Value::as_str)
            .any(|s| s.eq_ignore_ascii_case(value))
        {
            return Err(CmdError::msg("Already recorded."));
        }
        list.push(serde_json::json!(value));
        doc.set(field, serde_json::Value::Array(list));
        self.cmd
            .store()
            .put_in(actor, &doc, actor, ctx)
            .map_err(|es| {
                CmdError::msg(
                    es.iter()
                        .map(std::string::ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("; "),
                )
            })
    }
}

fn split_eq(line: &str) -> Result<(String, String), CmdError> {
    line.split_once('=')
        .map(|(l, r)| (l.trim().to_string(), r.trim().to_string()))
        .ok_or_else(|| CmdError::msg("Use <field>=<value> (type 'help' for this step)."))
}

/// Strip an optional leading `field` / `field=` / `field ` prefix.
fn strip_field_prefix<'a>(line: &'a str, field: &str) -> &'a str {
    let t = line.trim();
    if let Some(rest) = t
        .get(..field.len())
        .filter(|h| h.eq_ignore_ascii_case(field))
        .map(|_| &t[field.len()..])
    {
        let rest = rest.trim_start_matches(['=', ' ']).trim();
        if !rest.is_empty() {
            return rest;
        }
    }
    t
}

/// Case-insensitively strip a leading `verb ` from `line`.
fn strip_verb<'a>(line: &'a str, verb: &str) -> Option<&'a str> {
    let head = line.get(..verb.len())?;
    if !head.eq_ignore_ascii_case(verb) {
        return None;
    }
    let rest = &line[verb.len()..];
    rest.starts_with(char::is_whitespace).then(|| rest.trim())
}
