//! Offline laboratory commands dispatch before credentials or default config.
use crate::{
    Result,
    experiment::{load_json, save_json},
};
use std::path::PathBuf;
pub const MODES: &[&str] = &[
    "--check-mission",
    "--make-refutation-docket",
    "--make-refutation-dockets",
    "--consolidate-playtest",
    "--check-reproduction",
    "--reproduce",
    "--minimize",
    "--blind-packet",
    "--rate-blind",
    "--check-ratings",
];
pub fn launch() -> Result<u8> {
    let mut args = std::env::args().skip(1);
    let mode = args.next().ok_or("lab mode required")?;
    let path = PathBuf::from(args.next().ok_or("lab mode needs a JSON path")?);
    let (mut output, mut reviewer, mut ratings) = (None, None, None);
    while let Some(a) = args.next() {
        match a.as_str() {
        "--output" if output.is_none()=>output=Some(PathBuf::from(args.next().ok_or("output needs path")?)),
        "--reviewer" if reviewer.is_none() && mode=="--rate-blind"=>reviewer=Some(args.next().ok_or("reviewer needs identity")?),
        "--ratings" if ratings.is_none() && mode=="--check-ratings"=>ratings=Some(PathBuf::from(args.next().ok_or("ratings needs path")?)),
        _=>return Err("lab modes accept --output; --rate-blind also --reviewer; --check-ratings requires --ratings".into()),
    }
    }
    let required = || {
        output
            .as_deref()
            .ok_or_else(|| -> crate::Error { "this lab mode requires --output <new-path>".into() })
    };
    let value = match mode.as_str() {
        "--check-mission" => serde_json::to_value(crate::mission::Spec::load(&path)?)?,
        "--make-refutation-dockets" => {
            crate::playtest::make_dockets(&path, required()?)?;
            return Ok(0);
        }
        "--make-refutation-docket" => {
            crate::playtest::make_docket(&path, required()?)?;
            return Ok(0);
        }
        "--consolidate-playtest" => crate::playtest::consolidate(&path)?,
        "--check-reproduction" => {
            let (spec, _) = crate::reproduction::Spec::load(&path)?;
            serde_json::to_value(spec)?
        }
        "--reproduce" | "--minimize" => {
            let (spec, m) = crate::reproduction::Spec::load(&path)?;
            let report = tokio::runtime::Runtime::new()?.block_on(crate::reproduction::run(
                &spec,
                &m,
                required()?,
                &std::env::current_exe()?,
                mode == "--minimize",
            ))?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            return Ok(
                if report.status == crate::reproduction::Status::Reproduced {
                    0
                } else {
                    1
                },
            );
        }
        "--blind-packet" => {
            crate::blind_review::build(&path, required()?)?;
            println!(
                "Share packet.json only. Keep operator-reveal.json outside the reviewer workflow."
            );
            return Ok(0);
        }
        "--rate-blind" => {
            let p = crate::blind_review::Packet::load(&path)?;
            crate::blind_review::rate(&p, reviewer.ok_or("--reviewer is required")?, required()?)?;
            return Ok(0);
        }
        "--check-ratings" => {
            let p = crate::blind_review::Packet::load(&path)?;
            let r: crate::blind_review::Ratings =
                load_json(&ratings.ok_or("--ratings is required")?, 256_000)?;
            r.validate(&p)?;
            serde_json::json!({"valid":true,"complete":r.complete,"reviewer":r.reviewer,"packet":r.packet,"scores":r.scores.len()})
        }
        _ => return Err("unknown lab mode".into()),
    };
    if let Some(output) = output {
        save_json(&output, &value)?;
    } else {
        println!("{}", serde_json::to_string_pretty(&value)?);
    }
    Ok(0)
}
