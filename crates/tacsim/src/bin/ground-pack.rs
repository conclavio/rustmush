//! `ground-pack` — **the world generator v2 pack writer and validator**
//! (doc 30 §4, D-30-2; cut WG.1).
//!
//! Subcommands:
//! - `ground-pack build <draft.json> --out <dir> [--repair] [--force]`
//! - `ground-pack check <draft.json>`
//! - `ground-pack route <draft.json> [--out <draft.json>]`
//! - `ground-pack render <draft.json> --viewshed <x>,<y> [--size-cm <cm>] [--out <file>]`
//! - `ground-pack gen <preset> [--seed <N>] [--biome <B>] [--out <draft.json>]`
//! - `ground-pack from-packs <atlas.json> <arena.json> [--out <draft.json>]`

#![deny(
    clippy::unwrap_used,
    clippy::panic,
    clippy::unreachable,
    clippy::panic_in_result_fn
)]

use std::env;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

use tacsim::ground::{
    GroundDraft, build_with_options, check, from_packs, generate, route, viewshed,
};

fn usage() -> &'static str {
    "Usage:\n  \
     ground-pack build <draft.json> --out <dir> [--repair] [--force]\n  \
     ground-pack check <draft.json>\n  \
     ground-pack route <draft.json> [--out <draft.json>]\n  \
     ground-pack render <draft.json> --viewshed <x>,<y> [--size-cm <cm>] [--out <file>]\n  \
     ground-pack gen <preset> [--seed <N>] [--biome <B>] [--out <draft.json>]\n  \
     ground-pack from-packs <atlas.json> <arena.json> [--out <draft.json>]"
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().skip(1).collect();
    let Some(cmd) = args.first().map(String::as_str) else {
        return Err(usage().to_owned());
    };
    match cmd {
        "build" => {
            let Some(draft_path) = args.get(1) else {
                return Err(format!("missing <draft.json>\n{}", usage()));
            };
            let mut out_dir: Option<&str> = None;
            let mut repair = false;
            let mut force = false;
            let mut i = 2_usize;
            while i < args.len() {
                match args.get(i).map(String::as_str) {
                    Some("--out") => {
                        out_dir = args.get(i + 1).map(String::as_str);
                        i += 2;
                    }
                    Some("--repair") => {
                        repair = true;
                        i += 1;
                    }
                    Some("--force") => {
                        force = true;
                        i += 1;
                    }
                    Some(other) => return Err(format!("unknown argument `{other}`\n{}", usage())),
                    None => break,
                }
            }
            let Some(out_dir) = out_dir else {
                return Err(format!("missing `--out <dir>`\n{}", usage()));
            };
            let raw = fs::read_to_string(draft_path)
                .map_err(|e| format!("failed to read `{draft_path}`: {e}"))?;
            let draft = GroundDraft::from_json(&raw).map_err(|e| e.to_string())?;
            let built = build_with_options(&draft, repair, force).map_err(|e| e.to_string())?;

            if built.draft != draft {
                let updated_draft = built.draft.to_json_pretty().map_err(|e| e.to_string())?;
                fs::write(draft_path, &updated_draft)
                    .map_err(|e| format!("failed to update `{draft_path}`: {e}"))?;
                println!("updated {draft_path}");
            }

            let base = Path::new(out_dir);
            let atlas_dir = base.join("atlas");
            let arenas_dir = base.join("arenas");
            fs::create_dir_all(&atlas_dir)
                .map_err(|e| format!("failed to create `{}`: {e}", atlas_dir.display()))?;
            fs::create_dir_all(&arenas_dir)
                .map_err(|e| format!("failed to create `{}`: {e}", arenas_dir.display()))?;

            let atlas_path = atlas_dir.join(format!("{}.json", draft.id));
            let arena_path = arenas_dir.join(format!("{}.json", draft.id));
            fs::write(&atlas_path, &built.atlas_json)
                .map_err(|e| format!("failed to write `{}`: {e}", atlas_path.display()))?;
            fs::write(&arena_path, &built.arena_json)
                .map_err(|e| format!("failed to write `{}`: {e}", arena_path.display()))?;

            println!("tacsim.{} {}", draft.id, built.atlas_digest.to_hex());
            println!("tacsim.pack.{} {}", draft.id, built.arena_digest.to_hex());
            Ok(())
        }
        "check" => {
            let Some(draft_path) = args.get(1) else {
                return Err(format!("missing <draft.json>\n{}", usage()));
            };
            let raw = fs::read_to_string(draft_path)
                .map_err(|e| format!("failed to read `{draft_path}`: {e}"))?;
            let draft = GroundDraft::from_json(&raw).map_err(|e| e.to_string())?;
            let report = check(&draft).map_err(|e| e.to_string())?;
            let text = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
            println!("{text}");
            if !report.passed {
                return Err(format!(
                    "draft `{}` failed check: {}",
                    report.id,
                    report.failures.join("; ")
                ));
            }
            Ok(())
        }
        "route" => {
            let Some(draft_path) = args.get(1) else {
                return Err(format!("missing <draft.json>\n{}", usage()));
            };
            let mut out_path: Option<&str> = None;
            let mut i = 2_usize;
            while i < args.len() {
                match args.get(i).map(String::as_str) {
                    Some("--out") => {
                        out_path = Some(
                            args.get(i + 1)
                                .ok_or_else(|| "missing value after --out".to_owned())?
                                .as_str(),
                        );
                        i += 2;
                    }
                    Some(other) => return Err(format!("unknown argument `{other}`\n{}", usage())),
                    None => break,
                }
            }
            let raw = fs::read_to_string(draft_path)
                .map_err(|e| format!("failed to read `{draft_path}`: {e}"))?;
            let mut draft = GroundDraft::from_json(&raw).map_err(|e| e.to_string())?;
            route(&mut draft).map_err(|e| e.to_string())?;
            let json = draft.to_json_pretty().map_err(|e| e.to_string())?;
            let target = out_path.unwrap_or(draft_path.as_str());
            fs::write(target, &json).map_err(|e| format!("failed to write `{target}`: {e}"))?;
            Ok(())
        }
        "render" => {
            let Some(draft_path) = args.get(1) else {
                return Err(format!("missing <draft.json>\n{}", usage()));
            };
            let mut origin: Option<(u16, u16)> = None;
            let mut size_cm = 300_i32;
            let mut out_path: Option<&str> = None;
            let mut i = 2_usize;
            while i < args.len() {
                match args.get(i).map(String::as_str) {
                    Some("--viewshed") => {
                        let s = args
                            .get(i + 1)
                            .ok_or_else(|| "missing <x>,<y> after --viewshed".to_owned())?;
                        let mut parts = s.split(',');
                        let ox_s = parts
                            .next()
                            .ok_or_else(|| format!("invalid --viewshed `{s}`"))?;
                        let oy_s = parts
                            .next()
                            .ok_or_else(|| format!("invalid --viewshed `{s}`"))?;
                        let ox = ox_s
                            .parse::<u16>()
                            .map_err(|e| format!("invalid --viewshed x `{ox_s}`: {e}"))?;
                        let oy = oy_s
                            .parse::<u16>()
                            .map_err(|e| format!("invalid --viewshed y `{oy_s}`: {e}"))?;
                        origin = Some((ox, oy));
                        i += 2;
                    }
                    Some("--size-cm") => {
                        let s = args
                            .get(i + 1)
                            .ok_or_else(|| "missing value after --size-cm".to_owned())?;
                        size_cm = s
                            .parse::<i32>()
                            .map_err(|e| format!("invalid --size-cm `{s}`: {e}"))?;
                        i += 2;
                    }
                    Some("--out") => {
                        out_path = Some(
                            args.get(i + 1)
                                .ok_or_else(|| "missing value after --out".to_owned())?
                                .as_str(),
                        );
                        i += 2;
                    }
                    Some(other) => return Err(format!("unknown argument `{other}`\n{}", usage())),
                    None => break,
                }
            }
            let Some((ox, oy)) = origin else {
                return Err(format!("missing `--viewshed <x>,<y>`\n{}", usage()));
            };
            let raw = fs::read_to_string(draft_path)
                .map_err(|e| format!("failed to read `{draft_path}`: {e}"))?;
            let draft = GroundDraft::from_json(&raw).map_err(|e| e.to_string())?;
            let cells = viewshed(&draft, ox, oy, size_cm).map_err(|e| e.to_string())?;
            let payload = serde_json::json!({
                "id": draft.id,
                "origin": [ox, oy],
                "size_cm": size_cm,
                "cols": draft.cols,
                "rows": draft.rows,
                "cells": cells,
            });
            let json = serde_json::to_string(&payload).map_err(|e| e.to_string())?;
            if let Some(path) = out_path {
                fs::write(path, &json).map_err(|e| format!("failed to write `{path}`: {e}"))?;
            } else {
                println!("{json}");
            }
            Ok(())
        }
        "gen" => {
            let Some(preset) = args.get(1).map(String::as_str) else {
                return Err(format!("missing <preset>\n{}", usage()));
            };
            let mut seed = 2350_u64;
            let mut biome: Option<&str> = None;
            let mut out_path: Option<&str> = None;
            let mut i = 2_usize;
            while i < args.len() {
                match args.get(i).map(String::as_str) {
                    Some("--seed") => {
                        let s = args
                            .get(i + 1)
                            .ok_or_else(|| "missing value after --seed".to_owned())?;
                        seed = s
                            .parse::<u64>()
                            .map_err(|e| format!("invalid --seed `{s}`: {e}"))?;
                        i += 2;
                    }
                    Some("--biome") => {
                        biome = Some(
                            args.get(i + 1)
                                .ok_or_else(|| "missing value after --biome".to_owned())?
                                .as_str(),
                        );
                        i += 2;
                    }
                    Some("--out") => {
                        out_path = Some(
                            args.get(i + 1)
                                .ok_or_else(|| "missing value after --out".to_owned())?
                                .as_str(),
                        );
                        i += 2;
                    }
                    Some(other) => return Err(format!("unknown argument `{other}`\n{}", usage())),
                    None => break,
                }
            }
            let draft = generate(preset, seed, biome).map_err(|e| e.to_string())?;
            let json = draft.to_json_pretty().map_err(|e| e.to_string())?;
            if let Some(path) = out_path {
                if let Some(parent) = Path::new(path).parent()
                    && !parent.as_os_str().is_empty()
                {
                    fs::create_dir_all(parent)
                        .map_err(|e| format!("failed to create `{}`: {e}", parent.display()))?;
                }
                fs::write(path, &json).map_err(|e| format!("failed to write `{path}`: {e}"))?;
            } else {
                print!("{json}");
            }
            Ok(())
        }
        "from-packs" => {
            let Some(atlas_path) = args.get(1) else {
                return Err(format!("missing <atlas.json>\n{}", usage()));
            };
            let Some(arena_path) = args.get(2) else {
                return Err(format!("missing <arena.json>\n{}", usage()));
            };
            let mut out_path: Option<&str> = None;
            let mut i = 3_usize;
            while i < args.len() {
                match args.get(i).map(String::as_str) {
                    Some("--out") => {
                        out_path = Some(
                            args.get(i + 1)
                                .ok_or_else(|| "missing value after --out".to_owned())?
                                .as_str(),
                        );
                        i += 2;
                    }
                    Some(other) => return Err(format!("unknown argument `{other}`\n{}", usage())),
                    None => break,
                }
            }
            let atlas_json = fs::read_to_string(atlas_path)
                .map_err(|e| format!("failed to read `{atlas_path}`: {e}"))?;
            let arena_json = fs::read_to_string(arena_path)
                .map_err(|e| format!("failed to read `{arena_path}`: {e}"))?;
            let draft = from_packs(&atlas_json, &arena_json).map_err(|e| e.to_string())?;
            let json = draft.to_json_pretty().map_err(|e| e.to_string())?;
            if let Some(path) = out_path {
                if let Some(parent) = Path::new(path).parent()
                    && !parent.as_os_str().is_empty()
                {
                    fs::create_dir_all(parent)
                        .map_err(|e| format!("failed to create `{}`: {e}", parent.display()))?;
                }
                fs::write(path, &json).map_err(|e| format!("failed to write `{path}`: {e}"))?;
            } else {
                print!("{json}");
            }
            Ok(())
        }
        other => Err(format!("unknown subcommand `{other}`\n{}", usage())),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("{msg}");
            ExitCode::FAILURE
        }
    }
}
