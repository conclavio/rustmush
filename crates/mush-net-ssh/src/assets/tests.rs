//! The pack's tests, and the two blessers that make it:
//!
//! - `ASSETS_TONES=1 cargo test -p mush-net-ssh --lib assets::tests::bless`
//!   writes the placeholder tones (`assets/sounds/<name>.wav`, one a notice
//!   kind G.10 sounds, synthesised here, deterministic) and the manifest;
//! - `ASSETS_BLESS=1 …` writes the manifest alone, from whatever files are in
//!   `assets/` — the step after a real sound replaces a placeholder.

#![cfg(test)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use russh_sftp::client::SftpSession;
use russh_sftp::protocol::OpenFlags;
use tokio::io::AsyncReadExt;

use super::*;

fn assets_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets")
}

/// Every file under `dir`, as paths relative to it with `/`.
fn walk(dir: &Path, rel: &str, out: &mut BTreeSet<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = if rel.is_empty() {
            name.clone()
        } else {
            format!("{rel}/{name}")
        };
        if entry.path().is_dir() {
            walk(&entry.path(), &path, out);
        } else {
            out.insert(path);
        }
    }
}

// ---- the placeholder tones ---------------------------------------------------

const RATE: u32 = 22_050;

/// A deterministic noise source.
struct Lcg(u32);

impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.0 >> 9) as f32 / (1u32 << 23) as f32 * 2.0 - 1.0
    }
}

/// A piece of a tone: a sweep from `f0` to `f1` Hz over `ms`, at `vol`, with
/// `noise` of white noise mixed in, the whole decaying by `decay` (0 holds).
#[derive(Clone, Copy)]
struct Piece {
    ms: u32,
    f0: f32,
    f1: f32,
    vol: f32,
    noise: f32,
    decay: f32,
}

const fn tone(ms: u32, f: f32, vol: f32) -> Piece {
    Piece {
        ms,
        f0: f,
        f1: f,
        vol,
        noise: 0.0,
        decay: 0.0,
    }
}

const fn sweep(ms: u32, f0: f32, f1: f32, vol: f32) -> Piece {
    Piece {
        ms,
        f0,
        f1,
        vol,
        noise: 0.0,
        decay: 0.0,
    }
}

const fn burst(ms: u32, f: f32, noise: f32, vol: f32, decay: f32) -> Piece {
    Piece {
        ms,
        f0: f,
        f1: f,
        vol,
        noise,
        decay,
    }
}

const fn rest(ms: u32) -> Piece {
    tone(ms, 0.0, 0.0)
}

/// The placeholder sounds, one a notice kind (G.10's names): short, distinct,
/// and meant to be replaced.
fn placeholder_tones() -> Vec<(&'static str, Vec<Piece>)> {
    vec![
        (
            "acquired",
            vec![tone(70, 880.0, 0.5), rest(40), tone(90, 1320.0, 0.5)],
        ),
        (
            "classified",
            vec![tone(50, 1175.0, 0.4), rest(30), tone(50, 1175.0, 0.4)],
        ),
        (
            "lost",
            vec![tone(70, 1320.0, 0.4), rest(40), tone(90, 880.0, 0.4)],
        ),
        ("locked", vec![tone(140, 1760.0, 0.45)]),
        ("lock_lost", vec![sweep(260, 1760.0, 660.0, 0.45)]),
        ("hit", vec![burst(90, 180.0, 0.6, 0.8, 30.0)]),
        ("miss", vec![burst(120, 0.0, 0.35, 0.4, 20.0)]),
        ("kill", vec![burst(450, 70.0, 0.5, 0.9, 6.0)]),
        ("struck", vec![burst(160, 110.0, 0.5, 0.9, 18.0)]),
        (
            "section_lost",
            vec![
                burst(140, 90.0, 0.4, 0.85, 14.0),
                rest(50),
                burst(160, 70.0, 0.4, 0.85, 12.0),
            ],
        ),
        (
            "target_section_lost",
            vec![tone(60, 990.0, 0.4), rest(30), tone(60, 740.0, 0.4)],
        ),
        ("refused", vec![tone(160, 150.0, 0.5)]),
        (
            "blocked",
            vec![tone(90, 200.0, 0.45), rest(40), tone(90, 200.0, 0.45)],
        ),
        (
            "power_online",
            vec![sweep(380, 220.0, 880.0, 0.45), tone(80, 880.0, 0.45)],
        ),
        ("destroyed", vec![burst(800, 55.0, 0.6, 1.0, 3.5)]),
        ("disabled", vec![sweep(500, 660.0, 110.0, 0.5)]),
        ("receiver_degraded", vec![burst(300, 0.0, 0.5, 0.35, 0.0)]),
        (
            "drive_arrived",
            vec![tone(90, 784.0, 0.35), tone(140, 1046.0, 0.35)],
        ),
        (
            "overheat",
            vec![
                tone(150, 700.0, 0.5),
                tone(150, 930.0, 0.5),
                tone(150, 700.0, 0.5),
            ],
        ),
    ]
}

/// A tone as 16-bit mono PCM at [`RATE`], each piece faded in and out over
/// 4 ms so nothing clicks.
fn synth(pieces: &[Piece], seed: u32) -> Vec<i16> {
    let mut noise = Lcg(seed);
    let mut out = Vec::new();
    let mut phase = 0.0f32;
    for p in pieces {
        let n = (RATE * p.ms / 1_000) as usize;
        let fade = (RATE as usize * 4 / 1_000).max(1);
        for i in 0..n {
            let t = i as f32 / RATE as f32;
            let f = p.f0 + (p.f1 - p.f0) * (i as f32 / n.max(1) as f32);
            phase += 2.0 * std::f32::consts::PI * f / RATE as f32;
            let tonal = if f > 0.0 { phase.sin() } else { 0.0 };
            let mix = tonal * (1.0 - p.noise) + noise.next() * p.noise;
            let env = (-p.decay * t).exp() * (i.min(n - 1 - i) as f32 / fade as f32).min(1.0);
            out.push((mix * env * p.vol * 0.8 * f32::from(i16::MAX)) as i16);
        }
    }
    out
}

/// RIFF/WAVE, PCM format 1, mono, 16-bit: the 44-byte header and the data,
/// the RIFF length exact (w11term checks it).
fn wav(samples: &[i16]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// The manifest for the files under `dir`: a sound per `sounds/*.wav`, named
/// by its stem, sorted; the pack version the hash of the names and hashes.
fn manifest_for(dir: &Path) -> String {
    let mut files = BTreeSet::new();
    walk(dir, "", &mut files);
    let mut entries = Vec::new();
    let mut digest_input = String::new();
    for path in files.iter().filter(|p| p.as_str() != MANIFEST) {
        let bytes = std::fs::read(dir.join(path)).expect("an asset file");
        let name = path
            .rsplit('/')
            .next()
            .and_then(|f| f.strip_suffix(".wav"))
            .expect("a sound");
        let sha = sha256_hex(&bytes);
        digest_input.push_str(&format!("{name}:{sha}\n"));
        entries.push(serde_json::json!({
            "name": name,
            "path": path,
            "kind": "sound",
            "sha256": sha,
            "size": bytes.len(),
        }));
    }
    let pack_version = format!("pack_{}", &sha256_hex(digest_input.as_bytes())[..16]);
    let doc = serde_json::json!({
        "version": 1,
        "pack_version": pack_version,
        "files": entries,
    });
    serde_json::to_string_pretty(&doc).expect("json") + "\n"
}

/// The blessers (see the module): tones and the manifest, or the manifest.
#[test]
fn bless() {
    let dir = assets_dir();
    if std::env::var("ASSETS_TONES").is_ok() {
        std::fs::create_dir_all(dir.join("sounds")).expect("assets/sounds");
        for (seed, (name, pieces)) in placeholder_tones().into_iter().enumerate() {
            let bytes = wav(&synth(&pieces, seed as u32 + 1));
            std::fs::write(dir.join(format!("sounds/{name}.wav")), bytes).expect("a tone");
        }
    }
    if std::env::var("ASSETS_TONES").is_ok() || std::env::var("ASSETS_BLESS").is_ok() {
        std::fs::write(dir.join(MANIFEST), manifest_for(&dir)).expect("the manifest");
    }
}

// ---- the pack ----------------------------------------------------------------

/// The shipped table is the `assets/` directory, file for file, and the
/// pack passes w11term's rules.
#[test]
fn the_shipped_pack_is_the_directory_and_passes() {
    let mut on_disk = BTreeSet::new();
    walk(&assets_dir(), "", &mut on_disk);
    let shipped: BTreeSet<String> = SHIPPED.iter().map(|(p, _)| (*p).to_string()).collect();
    assert_eq!(shipped, on_disk, "SHIPPED and assets/ list the same files");
    let pack = AssetPack::shipped().expect("the shipped pack passes");
    assert!(pack.pack_version().starts_with("pack_"));
    assert!(valid_token(pack.pack_version()));
    assert_eq!(
        manifest_for(&assets_dir()).as_bytes(),
        pack.file(MANIFEST).expect("manifest"),
        "the manifest is the blesser's (ASSETS_BLESS=1 after a file changes)"
    );
    // Every sound is a notice kind's name or the alarm, and every one G.10
    // rings a bell for has one.
    for kind in crate::tui::sound::BELL_KINDS {
        assert!(pack.file(&format!("sounds/{kind}.wav")).is_some(), "{kind}");
    }
}

/// The tones are what w11term plays: RIFF/WAVE PCM, mono 16-bit, the RIFF
/// length exact, under five seconds.
#[test]
fn the_tones_are_wav_files_w11term_accepts() {
    for (path, bytes) in SHIPPED.iter().filter(|(p, _)| p.ends_with(".wav")) {
        assert_eq!(bytes.get(..4), Some(&b"RIFF"[..]), "{path}");
        let riff = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
        assert_eq!(riff + 8, bytes.len(), "{path}: the RIFF length");
        assert_eq!(&bytes[8..16], b"WAVEfmt ", "{path}");
        assert_eq!(u16::from_le_bytes([bytes[20], bytes[21]]), 1, "{path}: PCM");
        let data = u32::from_le_bytes(bytes[40..44].try_into().unwrap()) as usize;
        assert!(
            data > 0 && data / 2 <= (RATE * 5) as usize,
            "{path}: 0 < length <= 5 s"
        );
    }
}

/// The manifest's rules refuse what w11term would.
#[test]
fn a_pack_that_breaks_a_rule_is_refused() {
    static WAV: &[u8] = b"RIFF\x04\x00\x00\x00WAVE";
    let sha = sha256_hex(WAV);
    let pack = |manifest: String, extra: &[(&'static str, &'static [u8])]| {
        let manifest: &'static [u8] = Box::leak(manifest.into_bytes().into_boxed_slice());
        let mut files: Vec<(&str, &'static [u8])> = vec![(MANIFEST, manifest)];
        files.extend_from_slice(extra);
        AssetPack::from_files(&files)
    };
    let entry = |name: &str, path: &str, sha: &str, size: usize| {
        format!(
            r#"{{"version":1,"pack_version":"p1","files":[{{"name":"{name}","path":"{path}","kind":"sound","sha256":"{sha}","size":{size}}}]}}"#
        )
    };
    let wav = [("sounds/a.wav", WAV)];
    assert!(pack(entry("a", "sounds/a.wav", &sha, WAV.len()), &wav).is_ok());
    for (why, manifest, files) in [
        (
            "a wrong hash",
            entry("a", "sounds/a.wav", &"0".repeat(64), WAV.len()),
            &wav[..],
        ),
        (
            "a wrong size",
            entry("a", "sounds/a.wav", &sha, 3),
            &wav[..],
        ),
        (
            "a one-segment path",
            entry("a", "a.wav", &sha, WAV.len()),
            &[("a.wav", WAV)][..],
        ),
        (
            "a climbing path",
            entry("a", "sounds/../a.wav", &sha, WAV.len()),
            &wav[..],
        ),
        (
            "a reserved name",
            entry("off", "sounds/a.wav", &sha, WAV.len()),
            &wav[..],
        ),
        (
            "a device name",
            entry("COM1", "sounds/a.wav", &sha, WAV.len()),
            &wav[..],
        ),
        (
            "a file not shipped",
            entry("a", "sounds/b.wav", &sha, WAV.len()),
            &wav[..],
        ),
        (
            "an unknown field",
            entry("a", "sounds/a.wav", &sha, WAV.len())
                .replace("\"version\":1", "\"version\":1,\"x\":0"),
            &wav[..],
        ),
        (
            "a stray file",
            entry("a", "sounds/a.wav", &sha, WAV.len()),
            &[("sounds/a.wav", WAV), ("sounds/b.wav", WAV)][..],
        ),
    ] {
        assert!(pack(manifest, files).is_err(), "{why} is refused");
    }
}

// ---- the sftp tree -------------------------------------------------------------

async fn session() -> SftpSession {
    let (client, server) = tokio::io::duplex(256 * 1024);
    let pack = AssetPack::shipped().expect("pack");
    russh_sftp::server::run(EndOnError(server), AssetsSftp::new(pack)).await;
    SftpSession::new(client).await.expect("an sftp session")
}

/// **Served read-only**: the manifest read whole and the same bytes as
/// shipped, every listed file the size and hash the manifest says, the tree
/// listed, `.` the pack's root; a write, a create, a rename, a remove, a
/// directory made and a climb out are all refused.
#[tokio::test]
async fn the_tree_is_served_read_only() {
    let sftp = session().await;
    assert_eq!(sftp.canonicalize(".").await.expect("realpath"), ROOT);
    let mut manifest = Vec::new();
    sftp.open("/assets/manifest.json")
        .await
        .expect("open")
        .read_to_end(&mut manifest)
        .await
        .expect("read");
    let pack = AssetPack::shipped().expect("pack");
    assert_eq!(Some(manifest.as_slice()), pack.file(MANIFEST));
    let doc: Value = serde_json::from_slice(&manifest).expect("json");
    for entry in doc["files"].as_array().expect("files") {
        let path = format!("/assets/{}", entry["path"].as_str().expect("path"));
        let mut bytes = Vec::new();
        sftp.open(&path)
            .await
            .expect("open")
            .read_to_end(&mut bytes)
            .await
            .expect("read");
        assert_eq!(
            bytes.len() as u64,
            entry["size"].as_u64().expect("size"),
            "{path}"
        );
        assert_eq!(
            sha256_hex(&bytes),
            entry["sha256"].as_str().expect("sha"),
            "{path}"
        );
        assert_eq!(
            sftp.metadata(&path).await.expect("stat").len(),
            bytes.len() as u64
        );
    }
    let names: Vec<String> = sftp
        .read_dir("/assets")
        .await
        .expect("readdir")
        .map(|e| e.file_name())
        .collect();
    assert_eq!(names, ["manifest.json", "sounds"]);
    assert!(
        sftp.metadata("/assets/sounds")
            .await
            .expect("stat")
            .is_dir()
    );
    // Refused, every one.
    assert!(
        sftp.open_with_flags("/assets/manifest.json", OpenFlags::WRITE)
            .await
            .is_err()
    );
    assert!(
        sftp.open_with_flags("/assets/new.wav", OpenFlags::CREATE | OpenFlags::WRITE)
            .await
            .is_err()
    );
    assert!(
        sftp.rename("/assets/manifest.json", "/assets/x")
            .await
            .is_err()
    );
    assert!(sftp.remove_file("/assets/manifest.json").await.is_err());
    assert!(sftp.create_dir("/assets/x").await.is_err());
    assert!(sftp.open("/assets/../etc/passwd").await.is_err());
    assert!(sftp.open("/etc/passwd").await.is_err());
    assert!(sftp.metadata("/assets/sounds/nothing.wav").await.is_err());
}

#[test]
fn paths_normalise_and_never_climb() {
    assert_eq!(normalise(".").as_deref(), Some(ROOT));
    assert_eq!(
        normalise("sounds/hit.wav").as_deref(),
        Some("/assets/sounds/hit.wav")
    );
    assert_eq!(
        normalise("/assets//./sounds/").as_deref(),
        Some("/assets/sounds")
    );
    assert_eq!(normalise("/"), Some("/".to_string()));
    assert_eq!(normalise("../x"), None);
    assert_eq!(normalise("/assets/sounds/../../etc"), None);
    assert!(valid_token("pack_0123") && !valid_token("con") && !valid_token("lpt3"));
    assert!(valid_token("COM0") && valid_token("COM10"));
    assert!(valid_path("sounds/a b.wav") && !valid_path("sounds\\a.wav") && !valid_path("a.wav"));
}
