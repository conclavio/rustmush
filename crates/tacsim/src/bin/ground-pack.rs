//! `ground-pack` — **the world generator v2 pack writer and validator**
//! (doc 30 §4, D-30-2; cut WG.1).
//!
//! Subcommands:
//! - `ground-pack build <draft.json> --out <dir>`
//! - `ground-pack check <draft.json>`
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

use tacsim::ground::{GroundDraft, build, check, from_packs, generate};

fn usage() -> &'static str {
    "Usage:\n  \
     ground-pack build <draft.json> --out <dir>\n  \
     ground-pack check <draft.json>\n  \
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
            let mut i = 2_usize;
            while i < args.len() {
                match args.get(i).map(String::as_str) {
                    Some("--out") => {
                        out_dir = args.get(i + 1).map(String::as_str);
                        i += 2;
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
            let built = build(&draft).map_err(|e| e.to_string())?;

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
