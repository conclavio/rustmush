//! Recompute checked content-pack digests with the shipping canonical encoder.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    for path in std::env::args().skip(1) {
        let source = std::fs::read_to_string(&path)?;
        let digest = mush_content::parse_pack(&source)?
            .computed_digest()?
            .to_hex();
        let mut value: serde_json::Value = serde_json::from_str(&source)?;
        value["manifest"]["digest"] = digest.clone().into();
        std::fs::write(
            &path,
            format!("{}\n", serde_json::to_string_pretty(&value)?),
        )?;
        mush_content::load_pack(&std::fs::read_to_string(&path)?)?;
        println!("{path}: {digest}");
    }
    Ok(())
}
