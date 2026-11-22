//! `GET /api/client-downloads` — advertises matching standalone 3D client
//! archives stored in [`crate::WebConfig::client_downloads_dir`] and served directly
//! by the front-end static file server (e.g. Caddy `file_server`).
//!
//! For each platform target (`windows-x64`, `linux-x64`, `macos-universal`)
//! present on disk, this endpoint selects:
//! 1. The newest archive whose commit matches the running instance's source
//!    commit (`exact_match: true`), or
//! 2. The most recent archive for that platform (`exact_match: false`) if no
//!    exact commit match is present.
//!
//! Archive bytes are never read or streamed here — only filenames, metadata,
//! and optional tiny `<archive>.zip.json` sidecar manifests — so the endpoint
//! stays sub-millisecond and immune to large-download timeouts.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};

use crate::app::AppState;
use crate::assets;

/// Upper bound on directory entries inspected per request (prevents unbounded
/// scans if a misconfigured path points at a huge directory).
const MAX_DIR_ENTRIES: usize = 256;

/// Upper bound on optional `<archive>.zip.json` sidecar manifest size, bytes.
const MAX_MANIFEST_BYTES: u64 = 16 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum PlatformTarget {
    Windows64,
    Linux64,
    MacOs,
}

impl PlatformTarget {
    const ALL: [Self; 3] = [Self::Windows64, Self::Linux64, Self::MacOs];

    const fn slug(self) -> &'static str {
        match self {
            Self::Windows64 => "windows-x64",
            Self::Linux64 => "linux-x64",
            Self::MacOs => "macos-universal",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Windows64 => "Windows (x64)",
            Self::Linux64 => "Linux (x64)",
            Self::MacOs => "macOS (Universal)",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "windows-x64" | "windows64" | "win64" => Some(Self::Windows64),
            "linux-x64" | "linux64" => Some(Self::Linux64),
            "macos-universal" | "macos" | "osx" => Some(Self::MacOs),
            _ => None,
        }
    }
}

#[derive(Debug, Deserialize)]
struct SidecarManifest {
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    commit: Option<String>,
    #[serde(default)]
    short_commit: Option<String>,
    #[serde(default)]
    unity_version: Option<String>,
    #[serde(default)]
    built_at_utc: Option<String>,
    #[serde(default)]
    sha256: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct ClientDownloadEntry {
    pub target: &'static str,
    pub label: &'static str,
    pub filename: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    pub exact_match: bool,
    pub size_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unity_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub built_at_utc: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct ClientDownloadsResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_commit: Option<String>,
    pub clients: Vec<ClientDownloadEntry>,
}

#[derive(Clone, Debug)]
struct Candidate {
    filename: String,
    full_commit: Option<String>,
    display_commit: Option<String>,
    size_bytes: u64,
    sha256: Option<String>,
    unity_version: Option<String>,
    built_at_utc: Option<String>,
    modified_unix_secs: u64,
}

/// `GET /api/client-downloads` — unauthenticated metadata list of downloadable
/// standalone 3D client archives matching the running server version.
pub(crate) async fn client_downloads(State(state): State<AppState>) -> impl IntoResponse {
    let dir = state
        .config
        .client_downloads_dir
        .clone()
        .filter(|p| !p.as_os_str().is_empty());
    let url_base = state.config.client_downloads_url_base.clone();
    let server_commit = state
        .config
        .server_commit
        .clone()
        .and_then(|s| normalize_hex_commit(&s))
        .or_else(assets::embedded_bundle_commit);

    let payload = match dir {
        Some(dir_path) => {
            let commit_for_scan = server_commit.clone();
            tokio::task::spawn_blocking(move || {
                discover_clients(&dir_path, &url_base, commit_for_scan.as_deref())
            })
            .await
            .unwrap_or_else(|_| ClientDownloadsResponse {
                server_commit: server_commit.as_deref().map(short_commit_str),
                clients: Vec::new(),
            })
        }
        None => ClientDownloadsResponse {
            server_commit: server_commit.as_deref().map(short_commit_str),
            clients: Vec::new(),
        },
    };

    (
        StatusCode::OK,
        [(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"))],
        Json(payload),
    )
}

fn normalize_hex_commit(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if (7..=64).contains(&trimmed.len()) && trimmed.bytes().all(|b| b.is_ascii_hexdigit()) {
        Some(trimmed.to_ascii_lowercase())
    } else {
        None
    }
}

fn short_commit_str(commit: &str) -> String {
    commit.chars().take(9).collect()
}

fn commit_matches(server_commit: &str, candidate_commit: &str) -> bool {
    let Some(a) = normalize_hex_commit(server_commit) else {
        return false;
    };
    let Some(b) = normalize_hex_commit(candidate_commit) else {
        return false;
    };
    a.starts_with(&b) || b.starts_with(&a)
}

fn is_safe_archive_filename(name: &str) -> bool {
    if name.is_empty()
        || name.len() > 160
        || name.starts_with('.')
        || name.contains("..")
        || !name.ends_with(".zip")
    {
        return false;
    }
    name.bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

fn parse_filename_hint(filename: &str) -> (Option<PlatformTarget>, Option<String>) {
    let stem = filename.strip_suffix(".zip").unwrap_or(filename);
    let lower = stem.to_ascii_lowercase();
    for target in PlatformTarget::ALL {
        let needle = format!("-{}", target.slug());
        if let Some(pos) = lower.find(&needle) {
            let rest = stem.get(pos + needle.len()..).unwrap_or("");
            let commit = rest.strip_prefix('-').and_then(normalize_hex_commit);
            return (Some(target), commit);
        }
    }
    (None, None)
}

fn read_sidecar_manifest(archive_path: &Path) -> Option<SidecarManifest> {
    let mut os = archive_path.as_os_str().to_os_string();
    os.push(".json");
    let manifest_path = PathBuf::from(os);
    let meta = fs::symlink_metadata(&manifest_path).ok()?;
    if !meta.file_type().is_file() || meta.len() > MAX_MANIFEST_BYTES {
        return None;
    }
    let bytes = fs::read(&manifest_path).ok()?;
    let trimmed = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
    serde_json::from_slice::<SidecarManifest>(trimmed).ok()
}

fn clean_optional_field(raw: Option<String>, max_len: usize) -> Option<String> {
    let s = raw?.trim().to_string();
    if s.is_empty() || s.len() > max_len || s.bytes().any(|b| b.is_ascii_control()) {
        return None;
    }
    Some(s)
}

fn join_download_url(url_base: &str, filename: &str) -> String {
    let trimmed = url_base.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        format!("/downloads/clients/{filename}")
    } else {
        format!("{trimmed}/{filename}")
    }
}

pub(crate) fn discover_clients(
    dir: &Path,
    url_base: &str,
    server_commit: Option<&str>,
) -> ClientDownloadsResponse {
    let display_server_commit = server_commit
        .and_then(normalize_hex_commit)
        .map(|s| short_commit_str(&s));

    let Ok(read_dir) = fs::read_dir(dir) else {
        return ClientDownloadsResponse {
            server_commit: display_server_commit,
            clients: Vec::new(),
        };
    };

    let mut by_target: HashMap<PlatformTarget, Vec<Candidate>> = HashMap::new();

    for entry in read_dir.take(MAX_DIR_ENTRIES).flatten() {
        let path = entry.path();
        let Ok(file_name) = entry.file_name().into_string() else {
            continue;
        };
        if !is_safe_archive_filename(&file_name) {
            continue;
        }
        // Reject symlinks and non-regular files (defense-in-depth against escapes).
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.file_type().is_file() || meta.len() == 0 {
            continue;
        }

        let modified_unix_secs = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs());

        let (hint_target, hint_commit) = parse_filename_hint(&file_name);
        let sidecar = read_sidecar_manifest(&path);

        let target = sidecar
            .as_ref()
            .and_then(|m| m.target.as_deref())
            .and_then(PlatformTarget::parse)
            .or(hint_target);
        let Some(target) = target else {
            continue;
        };

        let full_commit = sidecar
            .as_ref()
            .and_then(|m| m.commit.as_deref())
            .and_then(normalize_hex_commit)
            .or_else(|| {
                sidecar
                    .as_ref()
                    .and_then(|m| m.short_commit.as_deref())
                    .and_then(normalize_hex_commit)
            })
            .or(hint_commit);

        let display_commit = sidecar
            .as_ref()
            .and_then(|m| m.short_commit.as_deref())
            .and_then(normalize_hex_commit)
            .or_else(|| full_commit.as_deref().map(short_commit_str));

        let sha256 = sidecar
            .as_ref()
            .and_then(|m| m.sha256.as_deref())
            .map(str::trim)
            .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            .map(str::to_ascii_lowercase);

        let unity_version =
            clean_optional_field(sidecar.as_ref().and_then(|m| m.unity_version.clone()), 32);
        let built_at_utc =
            clean_optional_field(sidecar.as_ref().and_then(|m| m.built_at_utc.clone()), 32);

        by_target.entry(target).or_default().push(Candidate {
            filename: file_name,
            full_commit,
            display_commit,
            size_bytes: meta.len(),
            sha256,
            unity_version,
            built_at_utc,
            modified_unix_secs,
        });
    }

    let mut clients = Vec::new();
    for target in PlatformTarget::ALL {
        let Some(candidates) = by_target.get(&target) else {
            continue;
        };
        let Some((best, exact_match)) = select_best_candidate(candidates, server_commit) else {
            continue;
        };
        clients.push(ClientDownloadEntry {
            target: target.slug(),
            label: target.label(),
            url: join_download_url(url_base, &best.filename),
            filename: best.filename.clone(),
            commit: best.display_commit.clone(),
            exact_match,
            size_bytes: best.size_bytes,
            sha256: best.sha256.clone(),
            unity_version: best.unity_version.clone(),
            built_at_utc: best.built_at_utc.clone(),
        });
    }

    ClientDownloadsResponse {
        server_commit: display_server_commit,
        clients,
    }
}

fn recency_key(c: &Candidate) -> (&str, u64, &str) {
    (
        c.built_at_utc.as_deref().unwrap_or(""),
        c.modified_unix_secs,
        c.filename.as_str(),
    )
}

fn select_best_candidate<'a>(
    candidates: &'a [Candidate],
    server_commit: Option<&str>,
) -> Option<(&'a Candidate, bool)> {
    if let Some(sc) = server_commit {
        let matched = candidates
            .iter()
            .filter(|c| {
                c.full_commit
                    .as_deref()
                    .is_some_and(|cc| commit_matches(sc, cc))
            })
            .max_by(|a, b| recency_key(a).cmp(&recency_key(b)));
        if let Some(hit) = matched {
            return Some((hit, true));
        }
    }

    candidates
        .iter()
        .max_by(|a, b| recency_key(a).cmp(&recency_key(b)))
        .map(|best| (best, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filename_validation_and_hints_reject_traversal_and_parse_targets() {
        assert!(is_safe_archive_filename(
            "RustMUSH_TacSim-windows-x64-1922318d8.zip"
        ));
        assert!(!is_safe_archive_filename(".hidden-windows-x64.zip"));
        assert!(!is_safe_archive_filename(
            "RustMUSH_TacSim-windows-x64.zip.part"
        ));
        assert!(!is_safe_archive_filename("../evil-windows-x64.zip"));
        assert!(!is_safe_archive_filename("bad/path-windows-x64.zip"));

        assert_eq!(
            parse_filename_hint("RustMUSH_TacSim-windows-x64-1922318d8.zip"),
            (Some(PlatformTarget::Windows64), Some("1922318d8".into()))
        );
        assert_eq!(
            parse_filename_hint("RustMUSH_TacSim-linux-x64.zip"),
            (Some(PlatformTarget::Linux64), None)
        );
        assert_eq!(
            parse_filename_hint("RustMUSH_TacSim-macos-universal-abcdef123.zip"),
            (Some(PlatformTarget::MacOs), Some("abcdef123".into()))
        );
    }

    #[test]
    fn commit_prefix_matching_links_short_and_full_shas() {
        assert!(commit_matches(
            "1922318d88b721095bf48dfa579c4d19becd399e",
            "1922318d8"
        ));
        assert!(commit_matches(
            "1922318d8",
            "1922318d88b721095bf48dfa579c4d19becd399e"
        ));
        assert!(!commit_matches("1922318d8", "deadbeef0"));
        assert!(!commit_matches("123", "123456789"));
    }

    #[test]
    fn exact_match_beats_newer_other_commit_and_falls_back_to_newest() {
        let dir =
            std::env::temp_dir().join(format!("rustmush-client-dl-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp test dir");

        // Older Windows build that matches the running server commit
        let win_match = dir.join("RustMUSH_TacSim-windows-x64-111111111.zip");
        fs::write(&win_match, b"zip-matching").expect("write win_match");
        fs::write(
            dir.join("RustMUSH_TacSim-windows-x64-111111111.zip.json"),
            r#"{"target":"Windows64","commit":"111111111aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","short_commit":"111111111","unity_version":"6000.6.7f1","built_at_utc":"2026-11-20T10:00:00Z"}"#,
        )
        .expect("write win_match json");

        // Newer Windows build for a DIFFERENT commit
        let win_newer = dir.join("RustMUSH_TacSim-windows-x64-222222222.zip");
        fs::write(&win_newer, b"zip-newer-other").expect("write win_newer");
        fs::write(
            dir.join("RustMUSH_TacSim-windows-x64-222222222.zip.json"),
            r#"{"target":"windows-x64","commit":"222222222bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","short_commit":"222222222","unity_version":"6000.6.7f1","built_at_utc":"2026-11-22T10:00:00Z"}"#,
        )
        .expect("write win_newer json");

        // Two Linux builds, neither matching server commit -> should pick newer
        let lin_old = dir.join("RustMUSH_TacSim-linux-x64-aaaaaaa01.zip");
        fs::write(&lin_old, b"lin-old").expect("write lin_old");
        fs::write(
            dir.join("RustMUSH_TacSim-linux-x64-aaaaaaa01.zip.json"),
            r#"{"target":"linux-x64","short_commit":"aaaaaaa01","built_at_utc":"2026-11-19T10:00:00Z"}"#,
        )
        .expect("write lin_old json");

        let lin_new = dir.join("RustMUSH_TacSim-linux-x64-bbbbbbb02.zip");
        fs::write(&lin_new, b"lin-new").expect("write lin_new");
        fs::write(
            dir.join("RustMUSH_TacSim-linux-x64-bbbbbbb02.zip.json"),
            r#"{"target":"linux-x64","short_commit":"bbbbbbb02","built_at_utc":"2026-11-21T10:00:00Z"}"#,
        )
        .expect("write lin_new json");

        // Partial upload must be ignored
        fs::write(
            dir.join("RustMUSH_TacSim-macos-universal-111111111.zip.part"),
            b"half-uploaded",
        )
        .expect("write part");

        let res = discover_clients(
            &dir,
            "/downloads/clients",
            Some("111111111aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
        );
        let _ = fs::remove_dir_all(&dir);

        assert_eq!(res.server_commit.as_deref(), Some("111111111"));
        assert_eq!(res.clients.len(), 2);

        let win = res.clients.first().expect("windows entry");
        assert_eq!(win.target, "windows-x64");
        assert_eq!(win.filename, "RustMUSH_TacSim-windows-x64-111111111.zip");
        assert_eq!(
            win.url,
            "/downloads/clients/RustMUSH_TacSim-windows-x64-111111111.zip"
        );
        assert!(win.exact_match);

        let lin = res.clients.get(1).expect("linux entry");
        assert_eq!(lin.target, "linux-x64");
        assert_eq!(lin.filename, "RustMUSH_TacSim-linux-x64-bbbbbbb02.zip");
        assert!(!lin.exact_match);
    }
}
