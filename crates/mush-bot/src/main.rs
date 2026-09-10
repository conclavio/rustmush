use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};

use mush_bot::{
    Result,
    character::Character,
    config::Config,
    runner::{Options, run},
    script::Scenario,
};

const USAGE: &str = "mush-bot — standalone RustMUSH SSH player\n\
Usage: mush-bot --config <file> [--goal <text>] [--role <text>]\n\
                [--paused | --headless] [--script <file.json>]\n\
                [--transcript <new.jsonl>] [--report <new.json>]\n\
                [--character <file.json>]\n\
                [--affiliations <file.json>]\n\
                [--models <profiles.json>] [--model <profile>]\n\
                [--tui | --json-events] [--prompts <file.json>]\n\
                [--context <file.json>] [--debug <file.json>] [--budget <file.json>]\n\
                [--conversation <file.json>]\n\
                [--login <account+character>] [--model-tape <file.json>]\n\
                [--test-controls] [--isolate-memory <new-directory>]\n\
       mush-bot --config <file> --replay <capture.json> --report <new.json> [--model <profile>]\n\
       mush-bot --config <file> --studio <concept.json> --output <new-directory> [--tui | --headless]\n\
       mush-bot --config <file> --studio-session <session.json> --output <new-directory> [--tui]\n\
       mush-bot --harness <cast.json> --output <new-directory> [--tui [--harness-debug <settings.json>]]\n\
       mush-bot --check-harness-debug <settings.json> [--report <new.json>]\n\
       mush-bot --check-budget <settings.json> [--report <new.json>]\n\
       mush-bot --check-conversation <settings.json> [--report <new.json>]\n\
       mush-bot --check-bookmark <bookmark.json> [--report <new.json>]\n\
       mush-bot --campaign <campaign.json> --output <new-directory>\n\
       mush-bot --check-campaign <campaign.json> [--output <new.json>]\n\
       mush-bot --inspect-campaign <summary.json> [--tui | --output <new.json>]\n\
       mush-bot --select-campaign <summary.json> --branch <name> --output <new.json>\n\
       mush-bot --harness <cast.json> --check-harness\n\
       mush-bot --harness <cast.json> --plan-harness [--report <new.json>]\n\
       mush-bot --analyze-harness <summary.json|directory> [--report <new.json>] [--junit <new.xml>]\n\
       mush-bot --inspect-trace <trace.jsonl> [--trace-query <query.json>]\n\
                [--report <new.json>] [--include-content]\n\
       mush-bot --inspect-trace <trace.jsonl> --tui [--trace-query <query.json>]\n\
       mush-bot --compare-captures <left.json> <right.json> [--report <new.json>] [--include-content]\n\
       mush-bot --script <file.json> --check-script\n\
       mush-bot --character <file.json> --check-character\n\
       mush-bot --prompts <file.json> --check-prompts\n\
       mush-bot --check-probe <probe.json>\n\
       mush-bot --grade-probe <probe.json> <capture.json> <answer.json> [--report <new.json>]\n\
       mush-bot --check-regression <policy.json> [--report <new.json>]\n\
       mush-bot --compare-runs <policy.json> <baseline-summary.json|directory> <candidate-summary.json|directory>\n\
                [--report <new.json>] [--junit <new.xml>]\n\
Use --tui for the local terminal UI; otherwise stdin is a control REPL (/help).\n\
--json-events emits JSONL events while retaining the control input stream. Script mode is headless\n\
and needs no model or API key. --scenario is an alias for --script.\n\
--check-script validates and prints the normalized script without connecting.\n\
--model-tape simulates responses locally. --test-controls enables /fault disconnect.\n\
Artifact paths must not already exist.\n\
Exit codes: 0 completed (or interactive quit), 1 error/assertion failure,\n\
            2 headless blocked/incomplete/budget exhausted.\n\
See docs/bot.md and mush-bot.toml.example.";

fn main() -> ExitCode {
    match launch() {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("mush-bot: {error}");
            ExitCode::FAILURE
        }
    }
}

fn launch() -> Result<u8> {
    if std::env::args().nth(1).is_some_and(|s| {
        [
            "--campaign",
            "--check-campaign",
            "--inspect-campaign",
            "--select-campaign",
        ]
        .contains(&s.as_str())
    }) {
        return mush_bot::campaign_cli::launch();
    }
    let mut args = std::env::args().skip(1);
    let mut config_path = PathBuf::from("mush-bot.toml");
    let mut options = Options::default();
    let mut goal = None;
    let mut role = None;
    let mut check_script = false;
    let mut character = None;
    let mut affiliations = None;
    let mut check_character = false;
    let mut check_prompts = false;
    let mut model_profiles = None;
    let mut model_profile = None;
    let mut tui = false;
    let mut prompts = None;
    let mut context = None;
    let mut debug = None;
    let mut budget = None;
    let mut conversation = None;
    let mut check_conversation = None;
    let mut check_bookmark = None;
    let mut check_budget = None;
    let mut replay = None;
    let mut tape = None;
    let mut isolate = None;
    let mut harness = None;
    let mut harness_debug = None;
    let mut check_harness_debug = None;
    let mut output = None;
    let mut check_harness = false;
    let mut plan_harness = false;
    let mut analyze_harness = None;
    let mut inspect_trace = None;
    let mut trace_query = None;
    let mut compare_captures = None;
    let mut junit = None;
    let mut include_content = false;
    let mut check_probe = None;
    let mut grade_probe = None;
    let mut check_regression = None;
    let mut compare_runs = None;
    let mut login = None;
    let mut studio = None;
    let mut studio_session = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                println!("{USAGE}");
                return Ok(0);
            }
            "--config" => config_path = args.next().ok_or("--config requires a path")?.into(),
            "--studio" => {
                if studio.is_some() {
                    return Err("supply only one --studio".into());
                }
                studio = Some(PathBuf::from(
                    args.next().ok_or("--studio requires a concept JSON path")?,
                ));
            }
            "--studio-session" => {
                if studio_session.is_some() {
                    return Err("supply only one --studio-session".into());
                }
                studio_session = Some(PathBuf::from(
                    args.next()
                        .ok_or("--studio-session requires a session JSON path")?,
                ));
            }
            "--goal" => goal = Some(args.next().ok_or("--goal requires text")?),
            "--role" => role = Some(args.next().ok_or("--role requires text")?),
            "--transcript" => {
                options.transcript = Some(args.next().ok_or("--transcript requires a path")?.into())
            }
            "--report" => {
                options.report = Some(args.next().ok_or("--report requires a path")?.into())
            }
            "--script" | "--scenario" => {
                if options.scenario.is_some() {
                    return Err("supply only one --script or --scenario".into());
                }
                options.scenario = Some(
                    args.next()
                        .ok_or("--script/--scenario requires a path")?
                        .into(),
                )
            }
            "--check-script" => check_script = true,
            "--character" => {
                character = Some(PathBuf::from(
                    args.next().ok_or("--character requires a path")?,
                ))
            }
            "--check-character" => check_character = true,
            "--check-prompts" => check_prompts = true,
            "--check-probe" => {
                check_probe = Some(PathBuf::from(
                    args.next()
                        .ok_or("--check-probe requires a probe JSON path")?,
                ))
            }
            "--grade-probe" => {
                let spec = PathBuf::from(
                    args.next()
                        .ok_or("--grade-probe requires probe, capture and answer JSON paths")?,
                );
                let capture = PathBuf::from(
                    args.next()
                        .ok_or("--grade-probe requires probe, capture and answer JSON paths")?,
                );
                let answer = PathBuf::from(
                    args.next()
                        .ok_or("--grade-probe requires probe, capture and answer JSON paths")?,
                );
                grade_probe = Some((spec, capture, answer));
            }
            "--affiliations" => {
                affiliations = Some(PathBuf::from(
                    args.next().ok_or("--affiliations requires a JSON path")?,
                ))
            }
            "--models" => {
                model_profiles = Some(PathBuf::from(
                    args.next().ok_or("--models requires a JSON path")?,
                ))
            }
            "--model" => {
                model_profile = Some(args.next().ok_or("--model requires a profile name")?)
            }
            "--headless" => options.headless = true,
            "--tui" => tui = true,
            "--json-events" => options.json_events = true,
            "--prompts" => {
                prompts = Some(PathBuf::from(
                    args.next().ok_or("--prompts requires a JSON path")?,
                ))
            }
            "--context" => {
                context = Some(PathBuf::from(
                    args.next().ok_or("--context requires a JSON path")?,
                ))
            }
            "--debug" => {
                debug = Some(PathBuf::from(
                    args.next().ok_or("--debug requires a JSON path")?,
                ))
            }
            "--budget" => {
                if budget.is_some() {
                    return Err("supply only one --budget".into());
                }
                budget = Some(PathBuf::from(
                    args.next()
                        .ok_or("--budget requires a settings JSON path")?,
                ));
            }
            "--conversation" => {
                conversation = Some(PathBuf::from(
                    args.next().ok_or("--conversation requires JSON path")?,
                ))
            }
            "--check-conversation" => {
                check_conversation = Some(PathBuf::from(
                    args.next()
                        .ok_or("--check-conversation requires JSON path")?,
                ))
            }
            "--check-bookmark" => {
                check_bookmark = Some(PathBuf::from(
                    args.next().ok_or("--check-bookmark requires JSON path")?,
                ))
            }
            "--managed-floor" => options.managed_floor = true,
            "--check-budget" => {
                if check_budget.is_some() {
                    return Err("supply only one --check-budget".into());
                }
                check_budget = Some(PathBuf::from(
                    args.next()
                        .ok_or("--check-budget requires a settings JSON path")?,
                ));
            }
            "--replay" => {
                replay = Some(PathBuf::from(
                    args.next().ok_or("--replay requires a capture JSON path")?,
                ))
            }
            "--model-tape" => {
                tape = Some(PathBuf::from(
                    args.next().ok_or("--model-tape requires a JSON path")?,
                ))
            }
            "--test-controls" => options.test_controls = true,
            "--isolate-memory" => {
                isolate = Some(PathBuf::from(
                    args.next()
                        .ok_or("--isolate-memory requires a new directory")?,
                ))
            }
            "--harness" => {
                harness = Some(PathBuf::from(
                    args.next().ok_or("--harness requires a JSON manifest")?,
                ))
            }
            "--output" => {
                output = Some(PathBuf::from(
                    args.next().ok_or("--output requires a new directory")?,
                ))
            }
            "--check-harness" => check_harness = true,
            "--harness-debug" => {
                if harness_debug.is_some() {
                    return Err("supply only one --harness-debug".into());
                }
                harness_debug = Some(PathBuf::from(
                    args.next()
                        .ok_or("--harness-debug requires a settings JSON path")?,
                ));
            }
            "--check-harness-debug" => {
                if check_harness_debug.is_some() {
                    return Err("supply only one --check-harness-debug".into());
                }
                check_harness_debug = Some(PathBuf::from(
                    args.next()
                        .ok_or("--check-harness-debug requires a settings JSON path")?,
                ));
            }
            "--plan-harness" => plan_harness = true,
            "--analyze-harness" => {
                analyze_harness = Some(PathBuf::from(
                    args.next()
                        .ok_or("--analyze-harness requires a summary JSON path or directory")?,
                ))
            }
            "--inspect-trace" => {
                if inspect_trace.is_some() {
                    return Err("supply only one --inspect-trace".into());
                }
                inspect_trace = Some(PathBuf::from(
                    args.next().ok_or("--inspect-trace requires a JSONL path")?,
                ));
            }
            "--trace-query" => {
                if trace_query.is_some() {
                    return Err("supply only one --trace-query".into());
                }
                trace_query = Some(PathBuf::from(
                    args.next()
                        .ok_or("--trace-query requires a query JSON path")?,
                ));
            }
            "--compare-captures" => {
                let left = PathBuf::from(
                    args.next()
                        .ok_or("--compare-captures requires two capture JSON paths")?,
                );
                let right = PathBuf::from(
                    args.next()
                        .ok_or("--compare-captures requires two capture JSON paths")?,
                );
                compare_captures = Some((left, right));
            }
            "--check-regression" => {
                if check_regression.is_some() {
                    return Err("supply only one --check-regression".into());
                }
                check_regression = Some(PathBuf::from(
                    args.next()
                        .ok_or("--check-regression requires a policy JSON path")?,
                ));
            }
            "--compare-runs" => {
                if compare_runs.is_some() {
                    return Err("supply only one --compare-runs".into());
                }
                let policy = PathBuf::from(
                    args.next()
                        .ok_or("--compare-runs requires policy, baseline and candidate paths")?,
                );
                let baseline = PathBuf::from(
                    args.next()
                        .ok_or("--compare-runs requires policy, baseline and candidate paths")?,
                );
                let candidate = PathBuf::from(
                    args.next()
                        .ok_or("--compare-runs requires policy, baseline and candidate paths")?,
                );
                compare_runs = Some((policy, baseline, candidate));
            }
            "--junit" => {
                junit = Some(PathBuf::from(
                    args.next().ok_or("--junit requires a new XML path")?,
                ))
            }
            "--include-content" => include_content = true,
            "--login" => {
                login = Some(
                    args.next()
                        .ok_or("--login requires an SSH account/character")?,
                )
            }
            "--paused" => options.paused = true,
            "--campaign-epoch" => {
                options.campaign_epoch =
                    Some(args.next().ok_or("--campaign-epoch requires a token")?)
            }
            "--campaign-quota" => {
                options.campaign_quota = Some(
                    args.next()
                        .ok_or("--campaign-quota requires a count")?
                        .parse()?,
                )
            }
            "--restore-bot" => {
                options.restore_bot = Some(args.next().ok_or("--restore-bot requires JSON")?.into())
            }
            _ => return Err(format!("unknown option {arg}; use --help").into()),
        }
    }
    let offline_modes = [
        check_character,
        check_prompts,
        check_harness,
        check_script,
        plan_harness,
        analyze_harness.is_some(),
        inspect_trace.is_some(),
        compare_captures.is_some(),
        check_probe.is_some(),
        grade_probe.is_some(),
        check_regression.is_some(),
        compare_runs.is_some(),
        check_harness_debug.is_some(),
        check_budget.is_some(),
        check_conversation.is_some(),
        check_bookmark.is_some(),
    ];
    if offline_modes.iter().filter(|v| **v).count() > 1 {
        return Err(
            "choose only one offline validation, planning, analysis or comparison mode".into(),
        );
    }
    if (conversation.is_some() || options.managed_floor)
        && (offline_modes.iter().any(|v| *v)
            || studio.is_some()
            || studio_session.is_some()
            || harness.is_some()
            || replay.is_some())
    {
        return Err("conversation session options cannot be combined with offline, studio, replay or harness modes; configure cast actors instead".into());
    }
    let studio_draft = if studio.is_some() || studio_session.is_some() {
        if offline_modes.iter().any(|v| *v)
            || harness.is_some()
            || replay.is_some()
            || options.scenario.is_some()
            || options.paused
            || options.json_events
            || options.transcript.is_some()
            || options.report.is_some()
            || goal.is_some()
            || role.is_some()
            || character.is_some()
            || affiliations.is_some()
            || prompts.is_some()
            || context.is_some()
            || debug.is_some()
            || isolate.is_some()
            || options.test_controls
            || login.is_some()
            || (tui && options.headless)
        {
            return Err("studio accepts model/config/budget/output settings and --tui or --headless; gameplay and inspection flags cannot be combined with authorship".into());
        }
        let destination = output
            .as_deref()
            .ok_or("--studio/--studio-session requires --output <new-directory>")?;
        if destination.exists() {
            return Err("studio output directory already exists".into());
        }
        Some(mush_bot::studio_runtime::load(
            studio.as_deref(),
            studio_session.as_deref(),
        )?)
    } else {
        None
    };
    if budget.is_some() && offline_modes.iter().any(|mode| *mode) {
        return Err("offline inspection cannot be combined with live session --budget".into());
    }
    if junit.is_some() && analyze_harness.is_none() && compare_runs.is_none() {
        return Err("--junit requires --analyze-harness or --compare-runs".into());
    }
    if harness_debug.is_some()
        && (harness.is_none() || !tui || offline_modes.iter().any(|mode| *mode))
    {
        return Err("--harness-debug requires a live --harness --tui session".into());
    }
    if include_content && compare_captures.is_none() && inspect_trace.is_none() {
        return Err("--include-content requires --compare-captures or --inspect-trace".into());
    }
    if trace_query.is_some() && inspect_trace.is_none() {
        return Err("--trace-query requires --inspect-trace".into());
    }
    if inspect_trace.is_some() && tui && (options.report.is_some() || include_content) {
        return Err("trace --tui cannot be combined with --report or --include-content".into());
    }
    // These tools consume only authored manifests or saved artifacts. In particular,
    // never load dotenv, credentials, a runtime, or a game connection to inspect a run.
    if plan_harness
        || analyze_harness.is_some()
        || inspect_trace.is_some()
        || compare_captures.is_some()
        || grade_probe.is_some()
        || check_probe.is_some()
        || check_regression.is_some()
        || compare_runs.is_some()
        || check_harness_debug.is_some()
        || check_budget.is_some()
        || check_conversation.is_some()
        || check_bookmark.is_some()
    {
        if replay.is_some()
            || options.scenario.is_some()
            || options.headless
            || options.paused
            || options.json_events
            || options.transcript.is_some()
            || goal.is_some()
            || role.is_some()
            || character.is_some()
            || affiliations.is_some()
            || model_profiles.is_some()
            || model_profile.is_some()
            || prompts.is_some()
            || context.is_some()
            || debug.is_some()
            || budget.is_some()
            || tape.is_some()
            || isolate.is_some()
            || options.test_controls
            || login.is_some()
            || (tui && inspect_trace.is_none())
            || output.is_some()
            || (harness.is_some() && !plan_harness)
        {
            return Err("offline inspection cannot be combined with live session options".into());
        }
        if let Some(path) = &options.report {
            validate_new_artifact(path, "json")?;
        }
        if let Some(path) = &junit {
            validate_new_artifact(path, "xml")?;
        }
        if let Some(path) = inspect_trace {
            let query = trace_query
                .as_deref()
                .map(mush_bot::trace::Query::load)
                .transpose()?
                .unwrap_or_default();
            let trace = mush_bot::trace::load(&path)?;
            if tui {
                mush_bot::trace_tui::run(&trace, &query)?;
            } else {
                let report = trace.report(&query, include_content)?;
                emit_offline_report(options.report.as_deref(), &report)?;
            }
            return Ok(0);
        }
        if let Some(path) = check_probe {
            let spec = mush_bot::probe::Spec::load(&path)?;
            emit_offline_report(options.report.as_deref(), &spec)?;
            return Ok(0);
        }
        if let Some(path) = check_harness_debug {
            let settings = mush_bot::cast_debug::Settings::load(&path)?;
            emit_offline_report(options.report.as_deref(), &settings)?;
            return Ok(0);
        }
        if let Some(path) = check_budget {
            let settings = mush_bot::budget::Settings::load(&path)?;
            emit_offline_report(options.report.as_deref(), &settings)?;
            return Ok(0);
        }
        if let Some(path) = check_conversation {
            let settings = mush_bot::conversation::Settings::load(&path)?;
            emit_offline_report(options.report.as_deref(), &settings)?;
            return Ok(0);
        }
        if let Some(path) = check_bookmark {
            let bookmark = mush_bot::scene::Bookmark::load(&path)?;
            emit_offline_report(
                options.report.as_deref(),
                &serde_json::json!({"valid":true,"id":bookmark.id,"kind":bookmark.kind,
                "watermark":bookmark.watermark,"modules":bookmark.modules.keys().collect::<Vec<_>>(),"world_restorable":false}),
            )?;
            return Ok(0);
        }
        if let Some(path) = check_regression {
            let policy = mush_bot::regression::Policy::load(&path)?;
            emit_offline_report(options.report.as_deref(), &policy)?;
            return Ok(0);
        }
        if let Some((policy, baseline, candidate)) = compare_runs {
            let policy = mush_bot::regression::Policy::load(&policy)?;
            let baseline = mush_bot::analysis::load(&baseline)?;
            let candidate = mush_bot::analysis::load(&candidate)?;
            let report = mush_bot::regression::evaluate(&policy, &baseline, &candidate)?;
            if let Some(path) = junit {
                let mut file = std::fs::File::options()
                    .write(true)
                    .create_new(true)
                    .open(path)?;
                file.write_all(mush_bot::regression::junit(&report).as_bytes())?;
            }
            emit_offline_report(options.report.as_deref(), &report)?;
            return Ok(if report.success { 0 } else { 1 });
        }
        if let Some((spec, capture, answer)) = grade_probe {
            let spec = mush_bot::probe::Spec::load(&spec)?;
            let capture = mush_bot::model::Capture::load(&capture)?;
            if !answer
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.eq_ignore_ascii_case("json"))
            {
                return Err("probe answer extension must be .json".into());
            }
            // Preserve duplicate keys and malformed response text for the same
            // strict parser used online; Value would silently normalize them.
            let mut source = String::new();
            std::fs::File::open(&answer)?
                .take(128_001)
                .read_to_string(&mut source)?;
            if source.len() > 128_000 {
                return Err("probe answer exceeds 128000 bytes".into());
            }
            let grade = mush_bot::probe::grade(
                &spec,
                &capture,
                source.strip_prefix('\u{feff}').unwrap_or(&source),
            )?;
            emit_offline_report(options.report.as_deref(), &grade)?;
            return Ok(if grade.success { 0 } else { 1 });
        }
        if let Some(path) = analyze_harness {
            let report = mush_bot::analysis::load(&path)?;
            if let Some(path) = junit {
                let mut file = std::fs::File::options()
                    .write(true)
                    .create_new(true)
                    .open(path)?;
                file.write_all(mush_bot::analysis::junit(&report).as_bytes())?;
            }
            emit_offline_report(options.report.as_deref(), &report)?;
            return Ok(if report.success { 0 } else { 1 });
        }
        if let Some((left, right)) = compare_captures {
            let left = mush_bot::model::Capture::load(&left)?;
            let right = mush_bot::model::Capture::load(&right)?;
            let report = mush_bot::comparison::compare_with_options(
                &left,
                &right,
                mush_bot::comparison::Options { include_content },
            )?;
            emit_offline_report(options.report.as_deref(), &report)?;
            return Ok(0);
        }
        let path = harness
            .as_deref()
            .ok_or("--plan-harness requires --harness")?;
        let report = mush_bot::harness::Manifest::load(path)?.plan()?;
        emit_offline_report(options.report.as_deref(), &report)?;
        return Ok(0);
    }
    if check_character {
        let path = character
            .as_deref()
            .ok_or("--check-character requires --character <path>")?;
        println!("{}", serde_json::to_string_pretty(&Character::load(path)?)?);
        return Ok(0);
    }
    if check_prompts {
        let path = prompts
            .as_deref()
            .ok_or("--check-prompts requires --prompts")?;
        println!(
            "{}",
            serde_json::to_string_pretty(&mush_bot::experiment::Prompts::load(path)?)?
        );
        return Ok(0);
    }
    if check_harness {
        let path = harness
            .as_deref()
            .ok_or("--check-harness requires --harness")?;
        println!(
            "{}",
            serde_json::to_string_pretty(&mush_bot::harness::Manifest::load(path)?)?
        );
        return Ok(0);
    }
    if check_script {
        let path = options
            .scenario
            .as_deref()
            .ok_or("--check-script requires --script <path>")?;
        println!("{}", serde_json::to_string_pretty(&Scenario::load(path)?)?);
        return Ok(0);
    }
    // Environment mutation must happen before the async runtime starts threads.
    // dotenvy preserves variables already supplied by the invoking shell.
    match dotenvy::dotenv() {
        Ok(_) => {}
        Err(error) if error.not_found() => {}
        // Parse errors include the offending line, which may contain a secret.
        Err(_) => return Err("could not load .env; check its syntax and permissions".into()),
    }
    if let Some(path) = harness {
        if replay.is_some()
            || options.scenario.is_some()
            || options.headless
            || options.paused
            || options.json_events
            || options.report.is_some()
            || options.transcript.is_some()
            || goal.is_some()
            || role.is_some()
            || character.is_some()
            || affiliations.is_some()
            || model_profiles.is_some()
            || model_profile.is_some()
            || prompts.is_some()
            || context.is_some()
            || debug.is_some()
            || budget.is_some()
            || tape.is_some()
            || isolate.is_some()
            || options.test_controls
            || login.is_some()
        {
            return Err(
                "harness session settings belong in its JSON actors, not session CLI flags".into(),
            );
        }
        let output = output
            .as_deref()
            .ok_or("--harness requires --output <new-directory>")?;
        let manifest = mush_bot::harness::Manifest::load(&path)?;
        let harness_debug = harness_debug
            .as_deref()
            .map(mush_bot::cast_debug::Settings::load)
            .transpose()?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        let report = if let Some(settings) = harness_debug {
            runtime.block_on(mush_bot::tui::run_cast_debugged(manifest, output, settings))?
        } else if tui {
            runtime.block_on(mush_bot::tui::run_cast(manifest, output))?
        } else {
            runtime.block_on(mush_bot::harness::run(manifest, output))?
        };
        println!(
            "Harness {}. Report: {}",
            if report["success"] == true {
                "passed"
            } else {
                "failed"
            },
            output.join("summary.json").display()
        );
        return Ok(if report["success"] == true { 0 } else { 1 });
    }
    if output.is_some() && studio_draft.is_none() {
        return Err("--output requires --harness, --studio or --studio-session".into());
    }
    let mut config = Config::load(&config_path)?;
    if let Some(login) = login {
        config.ssh.username = login;
    }
    if let Some(character) = character {
        config.character = Some(character);
    }
    if let Some(path) = affiliations {
        config.affiliations = Some(path);
    }
    if let Some(path) = model_profiles {
        config.model_profiles = Some(path);
    }
    let override_model = model_profile.is_some();
    if let Some(profile) = model_profile {
        config.model_profile = Some(profile);
    }
    if let Some(path) = prompts {
        config.prompts = Some(path);
    }
    if let Some(path) = context {
        config.context = Some(path);
    }
    if let Some(path) = debug {
        config.debug = Some(path);
    }
    if let Some(path) = budget {
        config.budget = Some(path);
    }
    if let Some(path) = conversation {
        config.conversation = Some(path);
    }
    if let Some(path) = tape {
        config.model_tape = Some(path);
    }
    if let Some(path) = isolate {
        std::fs::create_dir(&path)?;
        config.memory.directory = path;
        config.memory.file = None;
    }
    if let Some(goal) = goal {
        config.agent.goal = goal;
    }
    if let Some(role) = role {
        config.agent.role = role;
    }
    let headless = options.headless || options.scenario.is_some();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    if let Some(session) = studio_draft {
        let output = output.as_deref().unwrap();
        let report = if tui {
            runtime.block_on(mush_bot::studio_tui::run(
                config,
                session,
                output,
                studio.is_some(),
            ))?
        } else {
            runtime.block_on(mush_bot::studio_runtime::run(
                config,
                session,
                output,
                options.headless,
                studio.is_some(),
            ))?
        };
        println!("Studio report: {}", output.join("report.json").display());
        return Ok(if report["success"] == true { 0 } else { 1 });
    }
    if let Some(path) = replay {
        if tui
            || options.scenario.is_some()
            || options.headless
            || options.paused
            || options.json_events
            || options.transcript.is_some()
        {
            return Err(
                "--replay is a model-only operation and cannot be combined with session modes"
                    .into(),
            );
        }
        let output = options
            .report
            .as_deref()
            .ok_or("--replay requires --report <new.json>")?;
        let report = runtime.block_on(mush_bot::replay::run(
            &config,
            &path,
            output,
            override_model,
        ))?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(if report["success"] == true { 0 } else { 1 });
    }
    let report = if tui {
        runtime.block_on(mush_bot::tui::run(config, options))?
    } else {
        runtime.block_on(run(config, options))?
    };
    Ok(if report.state.status == "error" {
        1
    } else if headless && !report.success {
        2
    } else {
        0
    })
}

fn validate_new_artifact(path: &Path, extension: &str) -> Result<()> {
    if !path
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case(extension))
    {
        return Err(format!("artifact extension must be .{extension}").into());
    }
    if path.exists() {
        return Err(format!("artifact already exists: {}", path.display()).into());
    }
    Ok(())
}

fn emit_offline_report(path: Option<&Path>, report: &impl serde::Serialize) -> Result<()> {
    if let Some(path) = path {
        mush_bot::experiment::save_json(path, report)?;
        println!("Report saved: {}", path.display());
    } else {
        println!("{}", serde_json::to_string_pretty(report)?);
    }
    Ok(())
}
