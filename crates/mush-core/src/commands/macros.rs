//! `@macro`: a character's own command macros, and the kernel's alias stage
//! that expands them (16c N14; 16b cut AD.1).
//!
//! ## What a macro is (documented decision)
//!
//! A macro is a word of the player's own that stands for a line they type
//! often: `@macro lk=lock $1`, then `lk BA` types `lock BA`. Platform-wide —
//! a kernel verb, not a module's — and most useful in a cockpit whose action
//! verbs are typed on every surface by design.
//!
//! The one rule the whole module is built to: **a macro must never reach a
//! verb its player could not type, and a refusal on a name must never tell a
//! player what exists** (doc 16 §5, the observer rule, applied to a
//! refusal). It holds structurally:
//!
//! - An expansion is ONE [`AliasOutcome::Rewrite`] from [`MacroStage`], the
//!   first alias stage on every composition. Dispatch gates the substitute at
//!   once and offers it to the LATER stages only (16b cut AD.1, D9), then to
//!   `home`, exits and the command table under the typist's own `ObjId`,
//!   role, powers and locks — exactly as a typed line. The stage never sees
//!   its own output, so a macro is expanded once per line by construction,
//!   whatever the stored document says.
//! - A name is checked on its CHARACTERS first ([`BAD_NAME`]), before
//!   anything consults the registry. `@` and `+` are not in the set, and every
//!   staff-gated verb in the workspace is `@`-prefixed, so a staff verb —
//!   present or absent — reads byte-identical to a word that is no verb at
//!   all. Only then is a bare word refused as [`TAKEN`], one sentence
//!   whatever it collided with; a bare verb tells nothing typing it would
//!   not.
//! - The document lives on the player object as one JSON attribute
//!   ([`MACROS_ATTR`]), written through `World::set_attr` on the ordinary
//!   snapshot lane the way `@ignore` keeps its mute list. `@set` writes text
//!   only, and the stage honours nothing but a JSON object, so nothing a
//!   player writes by hand is ever expanded: every rule checked at definition
//!   is true of every macro the stage will ever expand.
//! - A line's VERB is fixed at definition: its first word may hold no `$`
//!   ([`DOLLAR_VERB`]), so `@wait` and the account verbs, refused there, can
//!   never be typed in through an argument. The stage refuses them once more
//!   on the expansion itself, whatever the document holds.
//!
//! The server keeps one record of an expansion, a `tracing::debug!` on the
//! target `mush_core::macros` naming the actor, the macro and the verb it
//! expanded to — never the arguments, which may be a password (the
//! discipline of the `@newpassword` audit line in the world task).

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::Value;

use crate::attr::{Attr, AttrFlags, AttrValue};
use crate::cmd::{
    AliasOutcome, AliasStage, ArgShape, CmdError, CommandSpec, Ctx, Registry, RegistryError,
};
use crate::flags::{Flags, Powers};
use crate::module::WorldEvent;
use crate::obj::ObjId;
use crate::output::Outbox;
use crate::perm;
use crate::world::World;

use super::movement::{exit_name_matches, find_exit};

/// The core attribute holding a player's macros: a JSON object from each
/// folded name to its line (`{"lk": "lock $1"}`), on the player's OWN object.
/// Removed when the last macro is deleted.
pub const MACROS_ATTR: &str = "macros";

/// At most this many macros per character (16c N14).
pub const MAX_MACROS: usize = 20;

/// A name is 1 to this many characters (comsys's channel-alias length).
pub const MAX_MACRO_NAME_LEN: usize = 16;

/// A macro's line is at most this many CHARACTERS (not bytes).
pub const MAX_MACRO_LINE_LEN: usize = 200;

/// An expansion is at most this many characters: the figure a player could
/// type as one line. It mirrors `mush_net_ssh::MAX_LINE_CHARS` (8192, which
/// the gateway also enforces on the web path); `mush-core` cannot import it,
/// so the number is repeated here and must move with it.
pub const MAX_MACRO_EXPANSION_CHARS: usize = 8192;

/// The bare words answered before the command table, which a macro may not
/// be named: `home` (dispatch step 4's keyword; it is ALSO a registered verb,
/// so `Registry::spec_of` refuses it too — it is listed for the keyword, which
/// holds whatever the table does), `quit` (the gateway closes on a whole-line
/// `QUIT` before forwarding) and `respond` (`Engine::step` answers it before
/// dispatch while a request is pending). `quit` and `respond` are registered
/// nowhere, so `Registry::spec_of` alone would miss them.
pub const KERNEL_WORDS: &[&str] = &["home", "quit", "respond"];

/// The verb's usage line (any form it does not recognise).
pub const USAGE: &str = "Usage: @macro <name>=<line> to make one, @macro to list them, @macro/del <name> to delete one.";
/// A name whose CHARACTERS are unusable — checked before anything else, so an
/// `@`/`+` word reads the same whether a verb of that name exists or not.
pub const BAD_NAME: &str = "That is not a usable macro name.  (1-16 letters, digits, - _ .)";
/// A well-formed name that already means something when typed: one sentence
/// whatever it collided with (a verb of any role, a kernel word, one of the
/// player's channel aliases).
pub const TAKEN: &str = "That word already means something when you type it; pick another name.";
/// A definition with nothing after the `=`.
pub const EMPTY_LINE: &str = "A macro needs a line: @macro <name>=<line>.";
/// A line over [`MAX_MACRO_LINE_LEN`] characters.
pub const TOO_LONG: &str = "A macro's line is at most 200 characters.";
/// A line starting with a speech prefix (16c `Q-AD1`: a prefix-expanded
/// rewrite would have to skip the later stages, and a rewrite carries no such
/// flag).
pub const PREFIX_LINE: &str =
    "A macro's line starts with a word (say, pose, semipose, @emit), not with \" : ; or \\.";
/// A line whose first word holds a `$`: the verb would be typed text, which
/// no definition-time rule could check (`@macro y=$1 $1`, typed
/// `y @wait 0=y`, would queue itself forever past [`NO_QUEUE`]). So a
/// macro's verb is fixed when it is made.
pub const DOLLAR_VERB: &str = "A macro's first word cannot hold a $; the words after it can.";
/// A line whose verb is `@wait` (16c `Q-AD3`); also the stage's answer to an
/// expansion whose verb is `@wait`.
pub const NO_QUEUE: &str = "A macro cannot queue a command.";
/// A line whose verb is an account or credential verb; also the stage's
/// answer to such an expansion.
pub const NO_ACCOUNT: &str = "A macro cannot touch your account, password or keys.";
/// A line that would run another macro, or a name that another macro's line
/// runs.
pub const CHAIN: &str = "A macro cannot run another macro.";
/// A new name past [`MAX_MACROS`].
pub const CAP: &str = "You already have 20 macros; @macro/del one first.";
/// Bare `@macro` with no macros.
pub const LIST_EMPTY: &str =
    "You have no macros.  (@macro <name>=<line> makes one; help macros explains.)";
/// From the stage: a macro whose line uses no `$n`, typed with words after
/// its name (typed text is never dropped silently).
pub const NO_ARGS: &str = "That macro takes nothing after its name.";
/// From the stage: an expansion over [`MAX_MACRO_EXPANSION_CHARS`].
pub const EXPANSION_TOO_LONG: &str = "That macro's line comes out too long.";

/// The suffix LIST puts on a row whose name is now taken (a verb or channel
/// alias that arrived after the macro was made; the stage cannot see the
/// registry, so the macro still wins for its owner — this is where it shows).
const NOW_TAKEN_SUFFIX: &str = "  (now a command; @macro/del it)";

/// The account and credential verbs a macro's line may not start with
/// (belt and braces under the bot's own first-word guard).
const ACCOUNT_VERBS: &[&str] = &["@sshkey", "@newpassword", "@newchar"];

/// The verbs whose line re-dispatches a stored line later (`@wait`): the one
/// way a stored macro could re-arm itself across scheduler turns.
const QUEUE_VERBS: &[&str] = &["@wait"];

/// The verb of `line` as the command table will look it up: the first
/// whitespace-delimited word, cut at the first `/` (switch-stripped), folded
/// with `to_lowercase` — the registry's own fold, so no Unicode letter that
/// lowercases to ASCII (the Kelvin sign `K` is `k`) slips a verb past the
/// lists below.
fn verb_of(line: &str) -> String {
    first_word(line)
        .split('/')
        .next()
        .unwrap_or("")
        .to_lowercase()
}

/// The refusal for a line whose verb (as [`verb_of`] folds it) a macro may
/// not run: [`NO_QUEUE`] for `@wait`, [`NO_ACCOUNT`] for an account verb.
fn verb_refusal(verb: &str) -> Option<&'static str> {
    if QUEUE_VERBS.contains(&verb) {
        Some(NO_QUEUE)
    } else if ACCOUNT_VERBS.contains(&verb) {
        Some(NO_ACCOUNT)
    } else {
        None
    }
}

/// Whether `name` is an exit the room view lists to `actor`: one of the
/// actor's location's OWN exits, not GOING, and not DARK unless the actor
/// controls it or holds `SEE_HIDDEN` — the rule `output::room_view` applies.
/// The definition's "also an exit" note is told only for such an exit, so it
/// is never a silent probe for a hidden one (a DARK exit, a zone master's, a
/// global one on Limbo). The stage's exit-wins rule is unchanged: typing a
/// hidden exit's name takes it, which is what typing it always did.
fn exit_listed_here(w: &World, actor: ObjId, name: &str) -> bool {
    let Some(loc) = w.get(actor).and_then(|o| o.location) else {
        return false;
    };
    let see_hidden = perm::has_power(w, actor, Powers::SEE_HIDDEN);
    w.exits_of(loc).iter().any(|&e| {
        w.get(e).is_some_and(|o| {
            !o.flags.contains(Flags::GOING)
                && (!o.flags.contains(Flags::DARK) || see_hidden || perm::controls(w, actor, e))
                && exit_name_matches(o, name)
        })
    })
}

/// A usable name's characters: after trimming, 1 to [`MAX_MACRO_NAME_LEN`]
/// characters, each an ASCII letter or digit or one of `-` `_` `.` (comsys's
/// `valid_alias` rule). No `@`, no `+`: that absence is what keeps every
/// staff verb out of reach of [`TAKEN`].
fn valid_name(name: &str) -> bool {
    let name = name.trim();
    let len = name.chars().count();
    (1..=MAX_MACRO_NAME_LEN).contains(&len)
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Why `name` cannot be a macro of `actor`'s, or `None` when it can: exactly
/// the verb's name checks, in their load-bearing order.
///
/// 1. [`BAD_NAME`] on the characters alone (`valid_name`), before anything
///    else is consulted.
/// 2. [`TAKEN`] when the folded name is one of [`KERNEL_WORDS`], the exact
///    name or alias of any registered command whatever its role, power,
///    privilege or lock ([`Registry::spec_of`]), or a word an alias stage
///    claims for `actor` ([`Registry::alias_stage_claims_word`]: their own
///    channel aliases).
///
/// NOT refused: a word that only prefix-resolves (`f`, `lo`), an exit's name,
/// a behavior command, an object `$`-command, the actor's own existing macro
/// (a replacement). Public so the contract tests assert the same code the
/// verb runs.
#[must_use]
pub fn name_refusal(reg: &Registry, w: &World, actor: ObjId, name: &str) -> Option<&'static str> {
    let name = name.trim();
    if !valid_name(name) {
        return Some(BAD_NAME);
    }
    let folded = name.to_ascii_lowercase();
    if KERNEL_WORDS.contains(&folded.as_str())
        || reg.spec_of(&folded).is_some()
        || reg.alias_stage_claims_word(w, actor, &folded)
    {
        return Some(TAKEN);
    }
    None
}

/// One piece of a macro line after the substitution walk.
enum Piece {
    /// Text copied (or unescaped) from the line.
    Text(String),
    /// `$1`, `$2` or `$3`, as a zero-based argument index.
    Arg(usize),
}

/// Substitute `args` (what was typed after the macro's name) into `line`.
///
/// One walk of `line` by characters: `$$` writes one `$`; `$1`, `$2`, `$3`
/// write their argument; any other `$` (a trailing one, `$4`, `$x`) is copied
/// verbatim. Let `n` be the HIGHEST of 1, 2, 3 the walk meets as an argument
/// (a `$1` right after a `$$` escape is the literal text `1` and does not
/// count). The typed text is split into at most `n` pieces: `$1` … `$n-1`
/// are single words and **`$n` is the rest of the typed line, verbatim**, so
/// nothing typed is dropped; a missing piece is empty. A line that uses no
/// `$n` typed with words after it is `None` (the stage answers [`NO_ARGS`]);
/// with nothing typed it is the line as is. The result is trimmed.
///
/// `say [$1|$2|$3|$$|$4]` with `x y z w` is `say [x|y|z w|$|$4]`; with `x`
/// it is `say [x|||$|$4]`.
#[must_use]
pub fn expand(line: &str, args: &str) -> Option<String> {
    // The walk: literal text and argument slots, and the highest slot seen.
    let mut pieces: Vec<Piece> = Vec::new();
    let mut text = String::with_capacity(line.len());
    let mut n = 0usize;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '$' {
            text.push(c);
            continue;
        }
        let slot = match chars.peek() {
            Some('$') => {
                chars.next();
                text.push('$');
                continue;
            }
            Some('1') => 1,
            Some('2') => 2,
            Some('3') => 3,
            // A trailing `$`, `$4`, `$x`: copied verbatim.
            _ => {
                text.push('$');
                continue;
            }
        };
        chars.next();
        pieces.push(Piece::Text(std::mem::take(&mut text)));
        pieces.push(Piece::Arg(slot - 1));
        n = n.max(slot);
    }
    pieces.push(Piece::Text(text));

    let typed = args.trim();
    // No `$n`: words typed after the name have nowhere to go. With nothing
    // typed the line is the walk's text (a `$$` still writes one `$`).
    if n == 0 && !typed.is_empty() {
        return None;
    }
    // `$1` … `$n-1` take one word each; `$n` takes the rest, verbatim. Word
    // by word rather than `splitn`, so a doubled space between two typed
    // words never makes an empty argument.
    let mut words: Vec<&str> = Vec::with_capacity(n);
    let mut rest = typed;
    while !rest.is_empty() && words.len() + 1 < n {
        match rest.split_once(char::is_whitespace) {
            Some((word, tail)) => {
                words.push(word);
                rest = tail.trim_start();
            }
            None => {
                words.push(rest);
                rest = "";
            }
        }
    }
    if !rest.is_empty() {
        words.push(rest);
    }

    let mut expanded = String::with_capacity(line.len() + typed.len());
    for piece in &pieces {
        match piece {
            Piece::Text(t) => expanded.push_str(t),
            Piece::Arg(i) => expanded.push_str(words.get(*i).copied().unwrap_or("")),
        }
    }
    Some(expanded.trim().to_owned())
}

/// The first whitespace-delimited word of `s` (empty when there is none).
fn first_word(s: &str) -> &str {
    s.split_whitespace().next().unwrap_or("")
}

/// The line of `actor`'s macro named `head` (already folded), or `None`.
///
/// ONE probe of the actor's OWN attribute map — never `World::attr`, which
/// walks `@parent`: a parent's macros are not the typist's. A `macros`
/// attribute that is text (a player's `@set me/macros=…`) or any JSON other
/// than an object holds no macros. Because `@set` can only write text, this
/// is what keeps every definition-time rule true of every macro the stage
/// will ever expand.
fn macro_line(w: &World, actor: ObjId, head: &str) -> Option<String> {
    w.get(actor)
        .and_then(|o| o.attrs.get(MACROS_ATTR))
        .and_then(Attr::as_json)
        .and_then(Value::as_object)
        .and_then(|m| m.get(head))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// Every macro of `actor`'s (the whole document, for the verb), in byte order
/// of the folded names. Non-string entries are skipped; a text or non-object
/// attribute is an empty map (and the next definition replaces it).
fn macros_of(w: &World, actor: ObjId) -> BTreeMap<String, String> {
    w.get(actor)
        .and_then(|o| o.attrs.get(MACROS_ATTR))
        .and_then(Attr::as_json)
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned())))
                .collect()
        })
        .unwrap_or_default()
}

/// The kernel's alias stage: expands the typist's own macros. Registered by
/// [`register`] from `register_core_commands`, so it is the FIRST alias stage
/// on every composition (before comsys's channel aliases).
///
/// It does NOT override [`AliasStage::claims_word`] (the default `false`): a
/// name that is already the player's macro is a replacement, the verb's own
/// business, not a collision.
pub struct MacroStage;

impl AliasStage for MacroStage {
    fn try_alias(&self, w: &mut World, actor: ObjId, line: &str, out: &mut Outbox) -> AliasOutcome {
        let line = line.trim();
        let (head, rest) = match line.split_once(char::is_whitespace) {
            Some((head, rest)) => (head, rest.trim_start()),
            None => (line, ""),
        };
        if head.is_empty() {
            return AliasOutcome::NotMine;
        }
        let head = head.to_ascii_lowercase();
        let Some(body) = macro_line(w, actor, &head) else {
            return AliasOutcome::NotMine;
        };
        // 1. The exit wins on the WHOLE line. The TUI's exit keys and the
        //    web's exit buttons submit the exit's name as the line (a
        //    multi-word one whole), and a bare key must never run a macro:
        //    `north` takes the exit, `north x` runs the macro (nothing submits
        //    an exit's name with words after it). One exit lookup, only on a
        //    macro hit.
        if find_exit(w, actor, line).is_some() {
            return AliasOutcome::NotMine;
        }
        // 2. A line with no `$n` typed with words after it: say so rather
        //    than drop what was typed.
        let Some(expanded) = expand(&body, rest) else {
            out.error(actor, NO_ARGS);
            return AliasOutcome::Handled;
        };
        // 3. The cap. Ten `$1`s fit in a 200-character line and an
        //    8,000-character word is typeable, so without this one typed line
        //    would be 80 KB to every listener — past the gateway's own line
        //    cap — and into a channel history the store keeps.
        if expanded.chars().count() > MAX_MACRO_EXPANSION_CHARS {
            out.error(actor, EXPANSION_TOO_LONG);
            return AliasOutcome::Handled;
        }
        // 4. The expansion's verb, as the table will fold it, is refused the
        //    way the definition refuses a line: never `@wait` (a macro that
        //    queues itself would re-arm on every fire — the one way a mortal
        //    could make the world task loop), never an account verb. The
        //    verb refuses `$` in a line's first word, so for every document it
        //    wrote this re-states a definition-time rule; it is here so the
        //    rule holds whatever the document holds, as the single expansion
        //    does.
        if let Some(refusal) = verb_refusal(&verb_of(&expanded)) {
            out.error(actor, refusal);
            return AliasOutcome::Handled;
        }
        // 5. The one record of an expansion: actor, macro and VERB, never the
        //    arguments. When the line's own first word holds a `$` (a
        //    document the verb did not write), the verb IS typed text, so the
        //    unexpanded word is logged instead.
        let body_verb = first_word(&body);
        let verb = if body_verb.contains('$') {
            body_verb
        } else {
            first_word(&expanded)
        };
        tracing::debug!(
            target: "mush_core::macros",
            actor = %actor,
            macro_name = %head,
            verb = %verb,
            "macro expanded"
        );
        AliasOutcome::Rewrite(expanded)
    }
}

/// Write `actor`'s whole macro document back: the JSON object for a
/// non-empty map, removal of the attribute for an empty one; then the
/// post-mutation `AttrSet` signal exactly as `@set` emits it. The store's
/// ordinary snapshot path (`set_attr`/`remove_attr` mark the object dirty):
/// no state slot, no module namespace.
fn save_macros(ctx: &mut Ctx<'_>, map: BTreeMap<String, String>) -> Result<(), CmdError> {
    if map.is_empty() {
        ctx.w.remove_attr(ctx.actor, MACROS_ATTR)?;
    } else {
        let doc: serde_json::Map<String, Value> = map
            .into_iter()
            .map(|(k, v)| (k, Value::String(v)))
            .collect();
        ctx.w.set_attr(
            ctx.actor,
            MACROS_ATTR,
            Attr {
                value: AttrValue::Json(Value::Object(doc)),
                owner: ctx.actor,
                flags: AttrFlags::empty(),
            },
        )?;
    }
    ctx.emit_world_event(&WorldEvent::AttrSet {
        obj: ctx.actor,
        name: MACROS_ATTR.to_string(),
        setter: ctx.actor,
    });
    Ok(())
}

/// Bare `@macro`: the actor's macros in byte order of their names, a row
/// marked when its name is now taken.
fn list_macros(ctx: &mut Ctx<'_>) {
    let map = macros_of(ctx.w, ctx.actor);
    if map.is_empty() {
        ctx.out.info(ctx.actor, LIST_EMPTY);
        return;
    }
    ctx.out.info(
        ctx.actor,
        format!("Your macros ({} of {MAX_MACROS}):", map.len()),
    );
    for (name, line) in &map {
        let mut row = format!("  {name:<16}  {line}");
        if name_refusal(ctx.reg, ctx.w, ctx.actor, name).is_some() {
            row.push_str(NOW_TAKEN_SUFFIX);
        }
        ctx.out.info(ctx.actor, row);
    }
}

/// `@macro/del <name>`: never checks the name's shape — fold, look up, and a
/// miss echoes the folded name whatever it is, so `@macro/del @arena` and
/// `@macro/del zzz` read alike.
fn delete_macro(ctx: &mut Ctx<'_>, name: &str) -> Result<(), CmdError> {
    let name = name.trim().to_ascii_lowercase();
    let mut map = macros_of(ctx.w, ctx.actor);
    if map.remove(&name).is_none() {
        return Err(CmdError::msg(format!("You have no macro named {name}.")));
    }
    save_macros(ctx, map)?;
    ctx.out.info(ctx.actor, format!("Macro {name} deleted."));
    Ok(())
}

/// `@macro <name>=<line>`: the name checks (shape, then taken), the line
/// checks, the cap, the write. The order is load-bearing: a bad line never
/// reveals whether a name was taken.
fn define_macro(ctx: &mut Ctx<'_>, name: &str, line: &str) -> Result<(), CmdError> {
    if let Some(refusal) = name_refusal(ctx.reg, ctx.w, ctx.actor, name) {
        return Err(CmdError::msg(refusal));
    }
    let name = name.trim().to_ascii_lowercase();
    let line = line.trim();
    if line.is_empty() {
        return Err(CmdError::msg(EMPTY_LINE));
    }
    if line.chars().count() > MAX_MACRO_LINE_LEN {
        return Err(CmdError::msg(TOO_LONG));
    }
    if line.starts_with(['"', ':', ';', '\\']) {
        return Err(CmdError::msg(PREFIX_LINE));
    }
    // The verb is fixed at definition: a `$` in the first word would make it
    // typed text (`@macro y=$1 $1`, then `y @wait 0=y`), past every check
    // below and past any client's first-word filter.
    let head = first_word(line).to_ascii_lowercase();
    if head.contains('$') {
        return Err(CmdError::msg(DOLLAR_VERB));
    }
    if let Some(refusal) = verb_refusal(&verb_of(line)) {
        return Err(CmdError::msg(refusal));
    }
    // No chains, both ways: the new line may not start with this or any other
    // of the player's macros, and the new NAME may not be the first word of
    // another macro's line (the one being replaced excluded — its old line
    // goes away). Whole first words, folded; the stage never re-expands, so
    // this is what keeps the list honest about what a name does.
    let mut map = macros_of(ctx.w, ctx.actor);
    if head == name || map.contains_key(&head) {
        return Err(CmdError::msg(CHAIN));
    }
    if map
        .iter()
        .any(|(other, body)| *other != name && first_word(body).to_ascii_lowercase() == name)
    {
        return Err(CmdError::msg(CHAIN));
    }
    let replacing = map.contains_key(&name);
    if !replacing && map.len() >= MAX_MACROS {
        return Err(CmdError::msg(CAP));
    }
    map.insert(name.clone(), line.to_owned());
    save_macros(ctx, map)?;
    let verb_done = if replacing { "replaced" } else { "set" };
    ctx.out
        .info(ctx.actor, format!("Macro {name} {verb_done}: {line}"));
    // Only for an exit this actor's room view lists: the note must never
    // tell of a DARK, zone or global exit they have not perceived.
    if exit_listed_here(ctx.w, ctx.actor, &name) {
        ctx.out.info(
            ctx.actor,
            format!("Note: {name} is also an exit here; typed alone, the exit wins."),
        );
    }
    Ok(())
}

/// The `@macro` verb: LIST, DEFINE (or replace), DELETE, or the usage line.
fn macro_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("del") {
        let name = ctx.args.lhs().to_owned();
        if name.is_empty() || ctx.args.rhs().is_some() {
            return Err(CmdError::msg(USAGE));
        }
        return delete_macro(ctx, &name);
    }
    if ctx.args.raw().is_empty() {
        list_macros(ctx);
        return Ok(());
    }
    let Some(line) = ctx.args.rhs().map(str::to_owned) else {
        return Err(CmdError::msg(USAGE));
    };
    let name = ctx.args.lhs().to_owned();
    define_macro(ctx, &name, &line)
}

/// Register `@macro`, then the kernel's [`MacroStage`] — the first alias
/// stage on every composition, since `register_core_commands` runs before
/// any module composes.
pub fn register(reg: &mut Registry) -> Result<(), RegistryError> {
    reg.register(
        CommandSpec {
            name: "@macro".into(),
            switches: vec!["del".into()],
            shape: ArgShape::EqPair,
            help: "@macro <name>=<line>  --  Name a line you type often; typing the \
                   name types the line, with $1, $2 and $3 standing for the words \
                   you type after it (the last one used takes the rest of the line; \
                   $$ is a plain $). @macro lists yours; @macro/del <name> removes \
                   one. A name is 1-16 letters, digits, - _ . and cannot be a \
                   command's name or one of its short forms, home, quit, respond, \
                   or one of your channel aliases; a line starts with a word of its \
                   own (no $ in it), not a speech prefix, and cannot be @wait, an \
                   account command, or \
                   another macro. Up to 20 per character, 200 characters each; kept \
                   on your character across reconnects and restarts. See help \
                   macros."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(macro_cmd),
    )?;
    reg.add_alias_stage(Arc::new(MacroStage));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_name_boundaries() {
        assert!(!valid_name(""));
        assert!(!valid_name("   "));
        assert!(valid_name("a"));
        assert!(valid_name("abcdefghijklmnop")); // 16
        assert!(!valid_name("abcdefghijklmnopq")); // 17
        assert!(valid_name("  lk  ")); // trimmed first
        assert!(valid_name("a-b"));
        assert!(valid_name("a_b"));
        assert!(valid_name("a.b"));
        assert!(valid_name("LK9"));
        assert!(!valid_name("a b"));
        assert!(!valid_name("$x"));
        assert!(!valid_name("@dig"));
        assert!(!valid_name("+x"));
        assert!(!valid_name("é"));
    }

    #[test]
    fn a_verb_is_folded_the_way_the_table_folds_it() {
        assert_eq!(verb_of("@WAIT/x 1=say hi"), "@wait");
        assert_eq!(verb_of("  say hi"), "say");
        assert_eq!(verb_of(""), "");
        // The Kelvin sign lowercases to an ASCII `k`, as the registry's
        // lookup folds it: `@sshKey` IS `@sshkey` to the command table.
        assert_eq!(verb_of("@ssh\u{212A}ey/add x"), "@sshkey");
        assert_eq!(verb_refusal("@wait"), Some(NO_QUEUE));
        assert_eq!(
            verb_refusal(&verb_of("@ssh\u{212A}ey/add x")),
            Some(NO_ACCOUNT)
        );
        assert_eq!(verb_refusal("@newpassword"), Some(NO_ACCOUNT));
        assert_eq!(verb_refusal("@newchar"), Some(NO_ACCOUNT));
        assert_eq!(verb_refusal("say"), None);
    }

    #[test]
    fn expand_table() {
        let t = "say [$1|$2|$3|$$|$4]";
        assert_eq!(expand(t, "x y z w").as_deref(), Some("say [x|y|z w|$|$4]"));
        assert_eq!(expand(t, "x").as_deref(), Some("say [x|||$|$4]"));
        assert_eq!(expand("lock $1", "BA").as_deref(), Some("lock BA"));
        assert_eq!(expand("lock $1", "").as_deref(), Some("lock"));
        assert_eq!(
            expand("say $1", "moving to B now").as_deref(),
            Some("say moving to B now")
        );
        // A trailing `$` and an unknown `$x` are copied verbatim.
        assert_eq!(expand("say cost $", "").as_deref(), Some("say cost $"));
        assert_eq!(expand("say $x $1", "a b").as_deref(), Some("say $x a b"));
        // `$$$1`: an escaped `$` then the argument.
        assert_eq!(expand("say $$$1", "5").as_deref(), Some("say $5"));
        // `$$1`: the literal text `$1`, which does not count toward n — so
        // the line uses no argument and typed words are refused.
        assert_eq!(expand("say $$1", "").as_deref(), Some("say $1"));
        assert_eq!(expand("say $$1", "x"), None);
        // `$3` alone: the first two words are dropped by the line's own
        // choice, and the third takes the rest.
        assert_eq!(expand("say $3", "a b c d").as_deref(), Some("say c d"));
        // A doubled space between words never makes an empty argument.
        assert_eq!(expand("say $1|$2", "a   b c").as_deref(), Some("say a|b c"));
        // Trimmed: leading and trailing blanks go, the line's inner ones stay.
        assert_eq!(expand("  say $1  ", "  hi  ").as_deref(), Some("say hi"));
        assert_eq!(expand("$1", "").as_deref(), Some(""));
        // No `$n`: nothing typed is the line as is; words typed is None.
        assert_eq!(expand("fire cannon", "").as_deref(), Some("fire cannon"));
        assert_eq!(expand("fire cannon", "  ").as_deref(), Some("fire cannon"));
        assert_eq!(expand("fire cannon", "BA"), None);
    }
}
