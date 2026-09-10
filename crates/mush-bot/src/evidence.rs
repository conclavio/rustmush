//! Local transcript evidence helpers, with no engine knowledge.
use crate::{
    Result,
    memory::{Episode, Event, Evidence, Record, Source},
};

pub fn cite<'a>(records: &'a [Record], evidence: &Evidence, before: u64) -> Result<&'a Episode> {
    let record = evidence
        .episode
        .checked_sub(1)
        .and_then(|id| records.get(id as usize))
        .filter(|r| r.id == evidence.episode && r.id < before)
        .ok_or("evidence must cite an earlier local episode")?;
    let Event::Episode(episode) = &record.event else {
        return Err("evidence is not an episode".into());
    };
    if episode.source != Source::Observed
        || evidence.quote.trim().is_empty()
        || evidence.quote.len() > 4000
        || !episode.text.contains(&evidence.quote)
    {
        return Err("evidence requires an exact quote from observed output".into());
    }
    Ok(episode)
}

/// Remove speech frames and native hanging-indent continuations. This deliberately
/// makes no claim that all remaining prose is authoritative engine truth.
pub fn non_speech(text: &str) -> String {
    let mut speaking = false;
    text.lines()
        .filter(|line| {
            if line.trim_start() == *line {
                speaking = crate::memory::speech(line).is_some();
            }
            !speaking && crate::memory::speech(line.trim_start()).is_none()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn bounded(text: &str, limit: usize) -> Result<()> {
    if text.trim().is_empty() || text.len() > limit || text.chars().any(char::is_control) {
        return Err("empty, overlong or control-containing text".into());
    }
    Ok(())
}
