//! 17g C2a (C2 plan §2.2, the C2a brief's words): every sentence the author
//! commands print, verbatim. `{reference}` is the resolved canonical
//! `name@rev`, `{arguments}` the command's argument text, `{filed}` and
//! `{licence}` the typed decimal number, `{charter}` and `{name}` the names
//! the command uses. A refusal names only what the actor typed or owns, so a
//! foreign number reads exactly like one that does not exist.
//!
//! The usage lines are the `@mind` `HELP` lines (`workshop::commands`).

pub(crate) const NOT_OPEN: &str = "The mind market is not open here.";
pub(crate) const BUSY: &str =
    "Your last market request is still being recorded; try again in a moment.";
pub(crate) const ACCOUNT: &str = "This needs exactly one personal credit account in your name.";
pub(crate) const NO_REVISION: &str = "You have no saved revision {reference}.";
pub(crate) const NOT_RATED: &str =
    "{reference} has no finished Registry rating. Rate it with @mind/rate {reference}.";
pub(crate) const RATING_BUSY: &str =
    "The Registry is already working on one of your revisions; wait until it says it is finished.";
pub(crate) const RESERVING: &str = "Reserving a charter for this filing first. Once that is confirmed, type @mind/file {arguments} again.";
pub(crate) const FILING: &str = "Filing {reference} as {charter}.";
pub(crate) const NO_FILED: &str = "You own no filed mind {filed}.";
pub(crate) const UPDATING: &str = "Filing {reference} on the line of filed mind {filed}.";
pub(crate) const WITHDRAWING: &str = "Withdrawing filed revision {filed}.";
pub(crate) const ATTESTING: &str = "Renewing your owner's licence for filed mind {filed}.";
pub(crate) const FORKING: &str = "Forking licence {licence} as {name}.";

/// The Registry service's own refusal, which `@mind/rate` shares with
/// `@registry/rate` (`registry_service::command`).
pub(crate) const RATING_UNAVAILABLE: &str = "The owned Registry computation is unavailable.";

/// `template` with each `{key}` replaced by its value, in one pass over the
/// template, so text the player typed is never itself substituted.
pub(crate) fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let (before, tail) = rest.split_at(open);
        out.push_str(before);
        let value = tail.find('}').and_then(|close| {
            let key = tail.get(1..close)?;
            let (_, value) = values.iter().find(|(name, _)| *name == key)?;
            Some((*value, close))
        });
        match value {
            Some((value, close)) => {
                out.push_str(value);
                rest = tail.get(close + 1..).unwrap_or_default();
            }
            None => {
                out.push('{');
                rest = tail.get(1..).unwrap_or_default();
            }
        }
    }
    out.push_str(rest);
    out
}
