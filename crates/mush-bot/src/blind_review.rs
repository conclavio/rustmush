//! Blinded human scene ratings. The reveal key is a separate operator artifact.
use crate::{
    Result,
    experiment::{load_json, revision, save_json},
    memory::{Event, Source},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourceScene {
    pub checkpoint: PathBuf,
    pub observer: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    pub version: u32,
    pub context: String,
    pub rubric: Rubric,
    pub scenes: Vec<SourceScene>,
    #[serde(default)]
    pub redact_terms: Vec<String>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Rubric {
    pub id: String,
    pub revision: String,
    pub dimensions: Vec<Dimension>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Dimension {
    pub id: String,
    pub question: String,
    pub low: String,
    pub high: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Scene {
    pub slot: String,
    pub text: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Packet {
    pub version: u32,
    pub id: String,
    pub context: String,
    pub rubric: Rubric,
    pub scenes: Vec<Scene>,
}
impl Packet {
    fn digest(&self) -> Result<String> {
        let mut p = self.clone();
        p.id.clear();
        Ok(revision(&serde_json::to_vec(&p)?))
    }
    pub fn validate(&self) -> Result<()> {
        if self.version != 1
            || self.id != self.digest()?
            || !(2..=16).contains(&self.scenes.len())
            || !(1..=8).contains(&self.rubric.dimensions.len())
            || serde_json::to_vec(self)?.len() > 2_000_000
        {
            return Err("invalid blind packet version, digest or limits".into());
        }
        crate::evidence::bounded(&self.context, 8000)?;
        crate::evidence::bounded(&self.rubric.id, 100)?;
        crate::evidence::bounded(&self.rubric.revision, 100)?;
        let mut ids = BTreeSet::new();
        for d in &self.rubric.dimensions {
            if !crate::organization::identifier(&d.id) || !ids.insert(&d.id) {
                return Err("duplicate or invalid rubric dimension".into());
            }
            for s in [&d.question, &d.low, &d.high] {
                crate::evidence::bounded(s, 2000)?;
            }
        }
        ids.clear();
        for s in &self.scenes {
            if !crate::organization::identifier(&s.slot)
                || !ids.insert(&s.slot)
                || s.text.trim().is_empty()
                || s.text.len() > 100_000
                || crate::terminal::plain(&s.text) != s.text
            {
                return Err("invalid blind scene slot/text".into());
            }
        }
        Ok(())
    }
    pub fn load(path: &Path) -> Result<Self> {
        let p: Self = load_json(path, 2_000_000)?;
        p.validate()?;
        Ok(p)
    }
}
pub fn build(path: &Path, output: &Path) -> Result<Packet> {
    let spec: Spec = load_json(path, 128_000)?;
    if spec.version != 1 || !(2..=16).contains(&spec.scenes.len()) || spec.redact_terms.len() > 100
    {
        return Err("blind review requires 2..16 scenes".into());
    }
    let base = std::fs::canonicalize(path)?
        .parent()
        .ok_or("blind spec parent absent")?
        .to_owned();
    let seed = crate::campaign::new_epoch()?;
    let mut items = vec![];
    let mut terms: BTreeSet<String> = spec.redact_terms.iter().cloned().collect();
    let mut names = BTreeSet::new();
    for source in &spec.scenes {
        let c = crate::campaign::Checkpoint::load(&base.join(&source.checkpoint))?;
        terms.insert(c.branch.clone());
        terms.insert(c.campaign.clone());
        let bot = c
            .bots
            .get(&source.observer)
            .ok_or("unknown blind scene observer")?;
        for b in c.bots.values() {
            names.insert(b.character.name.clone());
            if let Some(m) = &b.model {
                terms.insert(m.settings.name.clone());
                terms.insert(m.profile.clone());
                terms.insert(b.prompts.name.clone());
            }
        }
        let mut text = String::new();
        for r in &bot.memory.records {
            if let Event::Episode(e) = &r.event
                && e.source == Source::Observed
            {
                text.push_str(&e.text);
                text.push('\n');
            }
        }
        if text.len() > 100_000 {
            return Err(
                "blind scene exceeds 100000 bytes; select a smaller scene checkpoint".into(),
            );
        }
        items.push((crate::experiment::revision(format!("{seed}:{}",items.len()).as_bytes()),text,serde_json::json!({"checkpoint":source.checkpoint,"checkpoint_id":c.id,"observer":source.observer,"campaign":c.campaign,"branch":c.branch,"model":bot.model})));
    }
    for term in &terms {
        crate::evidence::bounded(term, 200)?;
        if names.iter().any(|n| {
            !n.is_empty()
                && (n.to_lowercase().contains(&term.to_lowercase())
                    || term.to_lowercase().contains(&n.to_lowercase()))
        }) {
            return Err(
                "blinding term collides with a character name; select neutral branch/model labels"
                    .into(),
            );
        }
    }
    // Context/rubric are reviewer-visible too; reject revealing labels instead
    // of silently changing the operator's rubric.
    let metadata = serde_json::to_string(&(&spec.context, &spec.rubric))?.to_lowercase();
    if terms.iter().any(|t| metadata.contains(&t.to_lowercase())) {
        return Err("blind context/rubric contains a source identity or redaction term".into());
    }
    items.sort_by(|a, b| a.0.cmp(&b.0));
    let mut scenes = vec![];
    let mut mapping = vec![];
    let mut ordered: Vec<_> = terms.iter().collect();
    ordered.sort_by_key(|s| std::cmp::Reverse(s.len()));
    for (_, mut text, source) in items {
        for term in &ordered {
            text = text.replace(term.as_str(), "[identity hidden]");
        }
        if terms
            .iter()
            .any(|t| text.to_lowercase().contains(&t.to_lowercase()))
        {
            return Err("scene contains a differently cased source label; add its exact spelling to redact_terms".into());
        }
        let slot = format!("scene-{}", scenes.len() + 1);
        scenes.push(Scene {
            slot: slot.clone(),
            text: crate::terminal::plain(&text),
        });
        mapping.push(serde_json::json!({"slot":slot,"source":source}));
    }
    let mut p = Packet {
        version: 1,
        id: String::new(),
        context: spec.context,
        rubric: spec.rubric,
        scenes,
    };
    p.id = p.digest()?;
    p.validate()?;
    std::fs::create_dir(output)?;
    save_json(&output.join("packet.json"), &p)?;
    save_json(
        &output.join("operator-reveal.json"),
        &serde_json::json!({"version":1,"packet":p.id,"order_seed":seed,"mapping":mapping,"redacted_terms":terms}),
    )?;
    Ok(p)
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Score {
    pub slot: String,
    pub dimension: String,
    pub rating: u8,
    pub note: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Ratings {
    pub version: u32,
    pub packet: String,
    pub rubric_revision: String,
    pub reviewer: String,
    pub complete: bool,
    pub scores: Vec<Score>,
}
impl Ratings {
    pub fn validate(&self, p: &Packet) -> Result<()> {
        p.validate()?;
        crate::evidence::bounded(&self.reviewer, 200)?;
        if self.version != 1
            || self.packet != p.id
            || self.rubric_revision != revision(&serde_json::to_vec(&p.rubric)?)
            || self.scores.len() > p.scenes.len() * p.rubric.dimensions.len()
        {
            return Err("rating packet/rubric binding or count mismatch".into());
        }
        let mut seen = BTreeSet::new();
        for s in &self.scores {
            if !(1..=5).contains(&s.rating)
                || s.note.len() > 4000
                || s.note.chars().any(char::is_control)
                || !seen.insert((&s.slot, &s.dimension))
                || !p.scenes.iter().any(|v| v.slot == s.slot)
                || !p.rubric.dimensions.iter().any(|d| d.id == s.dimension)
            {
                return Err("invalid or duplicate scene/dimension score".into());
            }
        }
        if self.complete && self.scores.len() != p.scenes.len() * p.rubric.dimensions.len() {
            return Err("incomplete ratings marked complete".into());
        }
        Ok(())
    }
}
fn draw(
    f: &mut ratatui::Frame<'_>,
    p: &Packet,
    r: &Ratings,
    scene: usize,
    dimension: usize,
    scroll: u16,
) {
    use ratatui::{
        layout::{Constraint, Layout},
        widgets::{Block, Paragraph, Wrap},
    };
    let areas = Layout::vertical([Constraint::Min(1), Constraint::Length(8)]).split(f.area());
    let s = &p.scenes[scene];
    let d = &p.rubric.dimensions[dimension];
    let score = r
        .scores
        .iter()
        .find(|v| v.slot == s.slot && v.dimension == d.id)
        .map(|s| s.rating.to_string())
        .unwrap_or_else(|| "unrated".into());
    f.render_widget(
        Paragraph::new(crate::terminal::plain(&format!(
            "{}\n\n{}",
            p.context, s.text
        )))
        .block(Block::bordered().title(s.slot.clone()))
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0)),
        areas[0],
    );
    f.render_widget(Paragraph::new(crate::terminal::plain(&format!("{}: {}\n1: {} | 5: {}\nRating: {} | Reviewer: {}\nLeft/right: scene; Tab: dimension; 1-5: rate; up/down: scroll\nS: save (partial allowed); Q: cancel. Objective checks remain separate.",d.id,d.question,d.low,d.high,score,r.reviewer))).block(Block::bordered().title("Blind human review")).wrap(Wrap{trim:false}),areas[1]);
}
pub fn rate(p: &Packet, reviewer: String, output: &Path) -> Result<()> {
    use crossterm::{
        event::{self, Event, KeyCode, KeyEventKind},
        execute,
        terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
    };
    use std::io::{IsTerminal, stdin, stdout};
    if !stdin().is_terminal() || !stdout().is_terminal() {
        return Err("blind rating TUI requires an interactive terminal; use --check-ratings for headless validation".into());
    }
    if output.exists() {
        return Err("rating output already exists".into());
    }
    let mut ratings = Ratings {
        version: 1,
        packet: p.id.clone(),
        rubric_revision: revision(&serde_json::to_vec(&p.rubric)?),
        reviewer,
        complete: false,
        scores: vec![],
    };
    ratings.validate(p)?;
    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            let _ = disable_raw_mode();
            let _ = execute!(stdout(), LeaveAlternateScreen, crossterm::cursor::Show);
        }
    }
    enable_raw_mode()?;
    let _restore = Restore;
    execute!(stdout(), EnterAlternateScreen)?;
    let mut terminal = ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(stdout()))?;
    let (mut scene, mut dimension, mut scroll) = (0, 0, 0_u16);
    loop {
        terminal.draw(|f| draw(f, p, &ratings, scene, dimension, scroll))?;
        if let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                KeyCode::Char('s') => {
                    ratings.complete =
                        ratings.scores.len() == p.scenes.len() * p.rubric.dimensions.len();
                    ratings.validate(p)?;
                    save_json(output, &ratings)?;
                    return Ok(());
                }
                KeyCode::Right => {
                    scene = (scene + 1) % p.scenes.len();
                    scroll = 0;
                }
                KeyCode::Left => {
                    scene = (scene + p.scenes.len() - 1) % p.scenes.len();
                    scroll = 0;
                }
                KeyCode::Tab => dimension = (dimension + 1) % p.rubric.dimensions.len(),
                KeyCode::Down => scroll = scroll.saturating_add(1),
                KeyCode::Up => scroll = scroll.saturating_sub(1),
                KeyCode::Char(c @ '1'..='5') => {
                    let slot = p.scenes[scene].slot.clone();
                    let d = p.rubric.dimensions[dimension].id.clone();
                    ratings
                        .scores
                        .retain(|s| s.slot != slot || s.dimension != d);
                    ratings.scores.push(Score {
                        slot,
                        dimension: d,
                        rating: c as u8 - b'0',
                        note: String::new(),
                    });
                }
                _ => {}
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn small_review_and_ratings_binding() {
        let mut p = Packet {
            version: 1,
            id: String::new(),
            context: "Neutral context".into(),
            rubric: Rubric {
                id: "human".into(),
                revision: "1".into(),
                dimensions: vec![Dimension {
                    id: "coherence".into(),
                    question: "Does it make sense?".into(),
                    low: "No".into(),
                    high: "Yes".into(),
                }],
            },
            scenes: vec![
                Scene {
                    slot: "scene-1".into(),
                    text: "Marco says hello.".into(),
                },
                Scene {
                    slot: "scene-2".into(),
                    text: "Marco waves.".into(),
                },
            ],
        };
        p.id = p.digest().unwrap();
        let mut r = Ratings {
            version: 1,
            packet: p.id.clone(),
            rubric_revision: revision(&serde_json::to_vec(&p.rubric).unwrap()),
            reviewer: "human-reviewer".into(),
            complete: false,
            scores: vec![],
        };
        r.validate(&p).unwrap();
        for (w, h) in [(12, 5), (80, 24)] {
            let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
            t.draw(|f| draw(f, &p, &r, 0, 0, 0)).unwrap();
        }
        r.complete = true;
        assert!(r.validate(&p).is_err());
        r.complete = false;
        r.packet = "wrong".into();
        assert!(r.validate(&p).is_err());
    }
}
