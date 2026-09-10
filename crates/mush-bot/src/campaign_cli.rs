//! Campaign CLI and offline comparison UI. No provider is consulted by inspection.
use crate::{Result, campaign};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Layout},
    widgets::{Block, Paragraph, Wrap},
};
use std::{
    io::{IsTerminal, stdin, stdout},
    path::PathBuf,
    time::Duration,
};

pub fn launch() -> Result<u8> {
    let mut args = std::env::args().skip(1);
    let mode = args.next().ok_or("campaign mode required")?;
    let path = PathBuf::from(args.next().ok_or("campaign mode requires JSON path")?);
    let mut output = None;
    let mut branch = None;
    let mut tui = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--output" if output.is_none() => output = Some(PathBuf::from(args.next().ok_or("--output requires path")?)),
            "--branch" if branch.is_none() => branch = Some(args.next().ok_or("--branch requires name")?),
            "--tui" => tui = true,
            _ => return Err("campaign mode accepts --output, --branch (selection only), --tui (inspection only)".into()),
        }
    }
    if (tui && mode != "--inspect-campaign") || (branch.is_some() && mode != "--select-campaign") {
        return Err(
            "--tui requires --inspect-campaign; --branch requires --select-campaign".into(),
        );
    }
    match mode.as_str() {
        "--campaign" => {
            let manifest = campaign::Manifest::load(&path)?;
            let runtime = tokio::runtime::Runtime::new()?;
            let result = runtime.block_on(campaign::run(
                &manifest,
                output
                    .as_deref()
                    .ok_or("--campaign requires --output <new-directory>")?,
                &std::env::current_exe()?,
            ))?;
            println!(
                "Completed {} restored world branches; provider spend $0 (local tapes).",
                result.branches.len()
            );
        }
        "--check-campaign" => {
            let manifest = campaign::Manifest::load(&path)?;
            if let Some(output) = output {
                crate::experiment::save_json(&output, &manifest)?;
            } else {
                println!("{}", serde_json::to_string_pretty(&manifest)?);
            }
        }
        "--inspect-campaign" => {
            let value = campaign::inspect(&path)?;
            if tui {
                if output.is_some() {
                    return Err("TUI inspection does not accept --output".into());
                }
                view(&value)?;
            } else if let Some(output) = output {
                crate::experiment::save_json(&output, &value)?;
            } else {
                println!("{}", serde_json::to_string_pretty(&value)?);
            }
        }
        "--select-campaign" => campaign::select(
            &path,
            &branch.ok_or("selection requires --branch")?,
            output
                .as_deref()
                .ok_or("selection requires --output <new.json>")?,
        )?,
        _ => return Err("unknown campaign mode".into()),
    }
    Ok(0)
}
struct Restore;
impl Drop for Restore {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(stdout(), LeaveAlternateScreen, crossterm::cursor::Show);
    }
}
fn detail(branch: &serde_json::Value) -> String {
    let mut text = format!(
        "{} | parent {}\nWorld commands: {} | reserved requests: {}\nCheckpoint: {}\n\n",
        branch["name"],
        branch["parent"],
        branch["world_command_sequence"],
        branch["reserved_requests_cumulative"],
        branch["checkpoint"]["id"]
    );
    if let Some(actors) = branch["actors"].as_array() {
        for actor in actors {
            text.push_str(&format!(
                "{}\nGoal: {}\nOutcome: {}\nNew requests: {} | Cost: {}\nNew probes: {}\nTrace: {}\nRecent scene (up to 12000 bytes):\n{}\n\n",
                actor["actor"],
                actor["goal"],
                actor["outcome"],
                actor["delta_costs"]["totals"]["requests"],
                actor["delta_costs"]["totals"]["provider_cost_by_currency"],
                actor["probes"].as_array().map_or(0,Vec::len),
                actor["trace"],
                crate::tail(actor["recent_scene"].as_str().unwrap_or_default(),12000)
            ));
            if !actor["social_perspective"].is_null() {
                text.push_str(&format!(
                    "Operator-only social perspective ({}):\n{}\n\n",
                    actor["actor"],
                    serde_json::to_string_pretty(&actor["social_perspective"]).unwrap_or_default()
                ));
            }
        }
    }
    // World observations can contain terminal escapes; never render them as controls.
    crate::terminal::plain(&text)
}

fn draw(
    frame: &mut ratatui::Frame<'_>,
    value: &serde_json::Value,
    selected: usize,
    anchor: usize,
    scroll: u16,
) {
    let branches = value["branches"]
        .as_array()
        .expect("validated campaign branch list");
    let areas = Layout::horizontal([
        Constraint::Percentage(22),
        Constraint::Percentage(39),
        Constraint::Percentage(39),
    ])
    .split(frame.area());
    let tree = branches
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let mut depth = 0;
            let mut parent = b["parent"].as_str();
            while let Some(p) = parent {
                depth += 1;
                if depth > 32 {
                    break;
                }
                parent = branches
                    .iter()
                    .find(|v| v["name"] == p)
                    .and_then(|v| v["parent"].as_str());
            }
            format!(
                "{} {}{}{}",
                if i == selected { ">" } else { " " },
                "  ".repeat(depth),
                b["name"].as_str().unwrap_or("?"),
                if i == anchor { " [compare]" } else { "" }
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    frame.render_widget(Paragraph::new(format!("{}\nrestored world branches\n\n{tree}\n\n↑↓ branch\nSpace set comparison\nPgUp/PgDn scroll both\nq quit\n\nNo automatic ranking.\nSelection: --select-campaign", value["campaign"])).block(Block::bordered().title("Campaign tree")).wrap(Wrap { trim: false }), areas[0]);
    frame.render_widget(
        Paragraph::new(detail(&branches[anchor]))
            .block(Block::bordered().title("Comparison anchor"))
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        areas[1],
    );
    frame.render_widget(
        Paragraph::new(detail(&branches[selected]))
            .block(Block::bordered().title("Selected continuation"))
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        areas[2],
    );
}
fn view(value: &serde_json::Value) -> Result<()> {
    if !stdin().is_terminal() || !stdout().is_terminal() {
        return Err("campaign TUI requires interactive terminal stdin/stdout".into());
    }
    let branches = value["branches"]
        .as_array()
        .ok_or("campaign has no branch list")?;
    if branches.is_empty() {
        return Err("campaign has no completed branches".into());
    }
    let mut selected = 0usize;
    let mut anchor = 0usize;
    let mut scroll = 0u16;
    enable_raw_mode()?;
    let _restore = Restore;
    execute!(stdout(), EnterAlternateScreen, crossterm::cursor::Hide)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    loop {
        terminal.draw(|frame| draw(frame, value, selected, anchor, scroll))?;
        if event::poll(Duration::from_millis(200))?
            && let Event::Key(key) = event::read()?
            && key.kind != KeyEventKind::Release
        {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break,
                KeyCode::Up => {
                    selected = selected.saturating_sub(1);
                    scroll = 0;
                }
                KeyCode::Down => {
                    selected = (selected + 1).min(branches.len() - 1);
                    scroll = 0;
                }
                KeyCode::Char(' ') => {
                    anchor = selected;
                    scroll = 0;
                }
                KeyCode::PageDown => scroll = scroll.saturating_add(12),
                KeyCode::PageUp => scroll = scroll.saturating_sub(12),
                _ => {}
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn branch_tree_and_both_sibling_scenes_render_and_resize_without_terminal_controls() {
        let value = serde_json::json!({"campaign":"band","branches":[
            {"name":"parent","actors":[]},
            {"name":"private","parent":"parent","actors":[{"actor":"Bass","recent_scene":"Private discussion\u{1b}[31m","delta_costs":{"totals":{"requests":1,"provider_cost_by_currency":{"USD":"0"}}}}]},
            {"name":"public","parent":"parent","actors":[{"actor":"Bass","recent_scene":"Public confrontation"}]}
        ]});
        assert!(!detail(&value["branches"][1]).contains('\u{1b}'));
        for (width, height) in [(200, 50), (60, 12), (1, 1)] {
            let mut terminal =
                Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            terminal.draw(|f| draw(f, &value, 2, 1, 0)).unwrap();
            if width == 200 {
                let buffer = terminal.backend().buffer();
                let text = buffer
                    .content
                    .iter()
                    .map(|c| c.symbol())
                    .collect::<String>();
                for expected in [
                    "Campaign tree",
                    "Comparison anchor",
                    "Selected continuation",
                    "Private discussion",
                    "Public confrontation",
                ] {
                    assert!(text.contains(expected), "missing {expected}");
                }
            }
        }
    }
}
