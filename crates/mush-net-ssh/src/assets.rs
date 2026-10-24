//! **The asset pack over the player's own connection** (doc 21 cut G.11).
//!
//! The pack is the repository's `assets/` directory — `manifest.json` and
//! the files it lists (the first pack: `sounds/<name>.wav`, one a cockpit
//! notice kind, G.10's names) — compiled into the binary ([`SHIPPED`]), so a
//! deploy ships it with the gateway that serves it and nothing on the box can
//! put another file in it. At first use it is read and checked whole against
//! w11term's manifest rules (`docs/game-client-protocol.md` there): schema
//! `version` 1, the pack version and every name 1–64 of `[A-Za-z0-9_-]` and no
//! DOS device name, unique names and paths, paths of two or more plain
//! segments, `.wav` for a sound, every size exact and nonzero, every SHA-256
//! the file's own, no unknown field. A pack that fails is not served, and the
//! failure is logged once.
//!
//! **Served read-only over `sftp`** ([`AssetsSftp`], SFTP v3): a virtual tree
//! of `/`, `/assets`, the pack's directories and its files, answered from
//! memory — no request ever names a path on the box. `open` (read only),
//! `read`, `close`, `opendir`, `readdir`, `stat`, `lstat`, `fstat` and
//! `realpath` are answered; everything else — a write, a create, a rename, a
//! remove, a link — is refused. A read is at most [`MAX_READ`] bytes, a
//! session at most [`BYTES_PER_SEC`] a second and [`MAX_HANDLES`] open
//! handles.
//!
//! **Announced to w11term** by the TUI (`tui::sound`), as OSC 440's `pack`
//! verb with [`AssetPack::pack_version`], once its `XTVERSION` names it; the
//! client then opens the `sftp` subsystem on a second channel of the same
//! connection and syncs what changed.

use std::collections::{BTreeSet, HashMap};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use russh_sftp::protocol::{
    Attrs, Data, File, FileAttributes, Handle, Name, OpenFlags, Status, StatusCode, Version,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Where the pack is, in the `sftp` tree.
pub const ROOT: &str = "/assets";

/// The manifest's path under [`ROOT`].
pub const MANIFEST: &str = "manifest.json";

/// The most bytes one `read` answers (the client reads 16 KiB).
pub const MAX_READ: u32 = 64 * 1024;

/// The most bytes a second one `sftp` session is sent: the client reads at
/// about 1 MiB a second; this keeps a pack sync from starving the picture.
pub const BYTES_PER_SEC: usize = 1024 * 1024;

/// The most handles one `sftp` session holds open.
pub const MAX_HANDLES: usize = 32;

// The limits w11term's client enforces, held here so a pack it would refuse
// is never served.
const MAX_MANIFEST: usize = 256 * 1024;
const MAX_FILES: usize = 256;
const MAX_FILE: usize = 8 * 1024 * 1024;
const MAX_PACK: usize = 64 * 1024 * 1024;
const MAX_PATH: usize = 200;

/// **The shipped pack**: every file under `assets/`, by its path there. A
/// test holds this table to the directory, so a file added to `assets/` and
/// not here (or here and not there) fails the build's tests.
pub const SHIPPED: &[(&str, &[u8])] = &[
    (MANIFEST, include_bytes!("../../../assets/manifest.json")),
    (
        "sounds/acquired.wav",
        include_bytes!("../../../assets/sounds/acquired.wav"),
    ),
    (
        "sounds/blocked.wav",
        include_bytes!("../../../assets/sounds/blocked.wav"),
    ),
    (
        "sounds/classified.wav",
        include_bytes!("../../../assets/sounds/classified.wav"),
    ),
    (
        "sounds/destroyed.wav",
        include_bytes!("../../../assets/sounds/destroyed.wav"),
    ),
    (
        "sounds/disabled.wav",
        include_bytes!("../../../assets/sounds/disabled.wav"),
    ),
    (
        "sounds/drive_arrived.wav",
        include_bytes!("../../../assets/sounds/drive_arrived.wav"),
    ),
    (
        "sounds/hit.wav",
        include_bytes!("../../../assets/sounds/hit.wav"),
    ),
    (
        "sounds/kill.wav",
        include_bytes!("../../../assets/sounds/kill.wav"),
    ),
    (
        "sounds/lock_lost.wav",
        include_bytes!("../../../assets/sounds/lock_lost.wav"),
    ),
    (
        "sounds/locked.wav",
        include_bytes!("../../../assets/sounds/locked.wav"),
    ),
    (
        "sounds/lost.wav",
        include_bytes!("../../../assets/sounds/lost.wav"),
    ),
    (
        "sounds/miss.wav",
        include_bytes!("../../../assets/sounds/miss.wav"),
    ),
    (
        "sounds/overheat.wav",
        include_bytes!("../../../assets/sounds/overheat.wav"),
    ),
    (
        "sounds/power_online.wav",
        include_bytes!("../../../assets/sounds/power_online.wav"),
    ),
    (
        "sounds/receiver_degraded.wav",
        include_bytes!("../../../assets/sounds/receiver_degraded.wav"),
    ),
    (
        "sounds/refused.wav",
        include_bytes!("../../../assets/sounds/refused.wav"),
    ),
    (
        "sounds/section_lost.wav",
        include_bytes!("../../../assets/sounds/section_lost.wav"),
    ),
    (
        "sounds/struck.wav",
        include_bytes!("../../../assets/sounds/struck.wav"),
    ),
    (
        "sounds/target_section_lost.wav",
        include_bytes!("../../../assets/sounds/target_section_lost.wav"),
    ),
];

/// **A checked asset pack**: its version and its files, the manifest among
/// them, by path under [`ROOT`].
#[derive(Clone, Debug)]
pub struct AssetPack {
    pack_version: String,
    files: HashMap<String, &'static [u8]>,
    dirs: BTreeSet<String>,
    sounds: BTreeSet<String>,
}

impl AssetPack {
    /// The shipped pack, checked once; `None` (logged once) when it fails.
    #[must_use]
    pub fn shipped() -> Option<&'static AssetPack> {
        static PACK: OnceLock<Option<AssetPack>> = OnceLock::new();
        PACK.get_or_init(|| match AssetPack::from_files(SHIPPED) {
            Ok(pack) => Some(pack),
            Err(why) => {
                tracing::error!(%why, "the shipped asset pack fails its manifest; not served");
                None
            }
        })
        .as_ref()
    }

    /// Read and check a pack from its files (the manifest at [`MANIFEST`]).
    ///
    /// # Errors
    ///
    /// What is wrong with it, in words: the first rule it breaks.
    pub fn from_files(files: &[(&str, &'static [u8])]) -> Result<AssetPack, String> {
        let by_path: HashMap<String, &'static [u8]> = files
            .iter()
            .map(|(path, bytes)| ((*path).to_string(), *bytes))
            .collect();
        if by_path.len() != files.len() {
            return Err("a path is shipped twice".into());
        }
        let manifest = by_path
            .get(MANIFEST)
            .ok_or_else(|| "no manifest.json".to_string())?;
        if manifest.len() > MAX_MANIFEST {
            return Err("the manifest is over 256 KiB".into());
        }
        let doc: Value =
            serde_json::from_slice(manifest).map_err(|e| format!("the manifest: {e}"))?;
        let top = doc.as_object().ok_or("the manifest is not an object")?;
        only_keys(top, &["version", "pack_version", "files"], "the manifest")?;
        if top.get("version").and_then(Value::as_u64) != Some(1) {
            return Err("the manifest's version is not 1".into());
        }
        let pack_version = top
            .get("pack_version")
            .and_then(Value::as_str)
            .filter(|v| valid_token(v))
            .ok_or("the pack version is not a token")?
            .to_string();
        let entries = top
            .get("files")
            .and_then(Value::as_array)
            .ok_or("the manifest has no files list")?;
        if entries.len() > MAX_FILES {
            return Err("more than 256 files".into());
        }
        let (mut names, mut paths, mut total) = (BTreeSet::new(), BTreeSet::new(), 0usize);
        let mut sounds = BTreeSet::new();
        for entry in entries {
            let e = entry.as_object().ok_or("a file entry is not an object")?;
            only_keys(
                e,
                &["name", "path", "kind", "sha256", "size"],
                "a file entry",
            )?;
            let text = |k: &str| e.get(k).and_then(Value::as_str).unwrap_or("");
            let (name, path, kind, sha) =
                (text("name"), text("path"), text("kind"), text("sha256"));
            if !valid_token(name) || !names.insert(name.to_string()) {
                return Err(format!("the name {name:?} is not a token or is used twice"));
            }
            if !["sound", "sprite", "font", "help"].contains(&kind) {
                return Err(format!("{name}: no kind {kind:?}"));
            }
            if kind == "sound" && (name == "system" || name == "off" || !path.ends_with(".wav")) {
                return Err(format!("{name}: a sound is a .wav and never system or off"));
            }
            if kind == "sound" {
                sounds.insert(name.to_string());
            }
            if !valid_path(path) || !paths.insert(path.to_string()) {
                return Err(format!(
                    "{name}: the path {path:?} is not one or is used twice"
                ));
            }
            let bytes = by_path
                .get(path)
                .ok_or_else(|| format!("{name}: {path} is not shipped"))?;
            let size = e.get("size").and_then(Value::as_u64).unwrap_or(0);
            if size == 0
                || usize::try_from(size).ok() != Some(bytes.len())
                || bytes.len() > MAX_FILE
            {
                return Err(format!("{name}: the size is not the file's"));
            }
            if sha != sha256_hex(bytes) {
                return Err(format!("{name}: the sha256 is not the file's"));
            }
            total += bytes.len();
        }
        if total > MAX_PACK {
            return Err("the pack is over 64 MiB".into());
        }
        if let Some(stray) = by_path
            .keys()
            .find(|p| p.as_str() != MANIFEST && !paths.contains(p.as_str()))
        {
            return Err(format!("{stray} is shipped and not in the manifest"));
        }
        let mut dirs = BTreeSet::new();
        for path in &paths {
            let mut at = String::new();
            for segment in path
                .split('/')
                .take(path.split('/').count().saturating_sub(1))
            {
                if !at.is_empty() {
                    at.push('/');
                }
                at.push_str(segment);
                dirs.insert(at.clone());
            }
        }
        Ok(AssetPack {
            pack_version,
            files: by_path,
            dirs,
            sounds,
        })
    }

    /// The pack's version: what the manifest says and the announcement names.
    #[must_use]
    pub fn pack_version(&self) -> &str {
        &self.pack_version
    }

    /// What a client's `hello` names (the web's, through `WebConfig`): the
    /// manifest's schema version, this pack's version and the manifest's
    /// path in the `sftp` tree.
    #[must_use]
    pub fn hello_info(&self) -> mush_proto::AssetPackInfo {
        mush_proto::AssetPackInfo {
            version: 1,
            pack_version: self.pack_version.clone(),
            manifest: format!("{ROOT}/{MANIFEST}"),
        }
    }

    /// The pack's sound names (the manifest's `name` of each `sound`): what a
    /// w11term can play from it, and so all it is sent (`tui::sound`).
    #[must_use]
    pub fn sound_names(&self) -> &BTreeSet<String> {
        &self.sounds
    }

    /// A file's bytes, by its path under [`ROOT`].
    #[must_use]
    pub fn file(&self, path: &str) -> Option<&'static [u8]> {
        self.files.get(path).copied()
    }

    /// What `path` (absolute, normalised) is in the tree.
    fn node(&self, path: &str) -> Option<Node> {
        if path == "/" || path == ROOT {
            return Some(Node::Dir);
        }
        let rel = path.strip_prefix(ROOT)?.strip_prefix('/')?;
        if let Some(bytes) = self.file(rel) {
            Some(Node::File(bytes))
        } else if self.dirs.contains(rel) {
            Some(Node::Dir)
        } else {
            None
        }
    }

    /// The names in directory `path` (absolute, normalised), sorted.
    fn listing(&self, path: &str) -> Vec<(String, Node)> {
        if path == "/" {
            return vec![("assets".to_string(), Node::Dir)];
        }
        let Some(rel) = path.strip_prefix(ROOT) else {
            return Vec::new();
        };
        let prefix = rel
            .strip_prefix('/')
            .map_or(String::new(), |r| format!("{r}/"));
        let mut out: Vec<(String, Node)> = Vec::new();
        let child = |p: &str| -> Option<String> {
            let rest = p.strip_prefix(prefix.as_str())?;
            (!rest.is_empty() && !rest.contains('/')).then(|| rest.to_string())
        };
        for dir in &self.dirs {
            if let Some(name) = child(dir) {
                out.push((name, Node::Dir));
            }
        }
        for (file, bytes) in &self.files {
            if let Some(name) = child(file) {
                out.push((name, Node::File(bytes)));
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }
}

/// A node of the tree.
#[derive(Clone, Copy, Debug)]
enum Node {
    Dir,
    File(&'static [u8]),
}

impl Node {
    fn attrs(self) -> FileAttributes {
        let mut attrs = FileAttributes::empty();
        match self {
            Node::Dir => {
                attrs.set_dir(true);
                attrs.permissions = Some(attrs.permissions.unwrap_or(0) | 0o555);
                attrs.size = Some(0);
            }
            Node::File(bytes) => {
                attrs.set_regular(true);
                attrs.permissions = Some(attrs.permissions.unwrap_or(0) | 0o444);
                attrs.size = Some(bytes.len() as u64);
            }
        }
        attrs
    }
}

fn only_keys(
    obj: &serde_json::Map<String, Value>,
    keys: &[&str],
    what: &str,
) -> Result<(), String> {
    match obj.keys().find(|k| !keys.contains(&k.as_str())) {
        Some(k) => Err(format!("{what} has an unknown field {k:?}")),
        None if obj.len() == keys.len() => Ok(()),
        None => Err(format!("{what} is missing a field")),
    }
}

/// OSC 440's argument grammar, which the pack version and every name share:
/// 1–64 ASCII letters, digits, `_` or `-`, and no Windows device name.
#[must_use]
pub fn valid_token(s: &str) -> bool {
    const DOS: &[&str] = &["CON", "PRN", "AUX", "NUL"];
    let upper = s.to_ascii_uppercase();
    let device = DOS.contains(&upper.as_str())
        || ((upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.len() == 4
            && upper
                .as_bytes()
                .get(3)
                .is_some_and(|b| (b'1'..=b'9').contains(b)));
    (1..=64).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        && !device
}

/// A manifest path: at most 200 bytes, two or more nonempty segments of
/// ASCII letters, digits, `_`, `-`, spaces and `.`, none `.` or `..`.
#[must_use]
pub fn valid_path(path: &str) -> bool {
    let segments: Vec<&str> = path.split('/').collect();
    path.len() <= MAX_PATH
        && segments.len() >= 2
        && segments.iter().all(|s| {
            !s.is_empty()
                && *s != "."
                && *s != ".."
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_- .".contains(&b))
        })
}

/// SHA-256 as 64 lower-case hex digits.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A path as the tree reads it: absolute, `.` and empty segments dropped, a
/// relative path taken from [`ROOT`]; `None` for one with `..` (no request
/// climbs, and none needs to).
fn normalise(path: &str) -> Option<String> {
    let base = if path.starts_with('/') { "" } else { ROOT };
    let mut out = Vec::new();
    for segment in base.split('/').chain(path.split('/')) {
        match segment {
            "" | "." => {}
            ".." => return None,
            s => out.push(s),
        }
    }
    Some(format!("/{}", out.join("/")))
}

/// An open handle.
#[derive(Debug)]
enum Open {
    File(&'static [u8]),
    Dir {
        entries: Vec<(String, Node)>,
        done: bool,
    },
}

/// **The `sftp` server of one connection** (see the module): the pack's tree
/// read-only, from memory.
#[derive(Debug)]
pub struct AssetsSftp {
    pack: &'static AssetPack,
    handles: HashMap<String, Open>,
    next: u64,
    window: (Instant, usize),
}

impl AssetsSftp {
    /// A session over `pack`.
    #[must_use]
    pub fn new(pack: &'static AssetPack) -> AssetsSftp {
        AssetsSftp {
            pack,
            handles: HashMap::new(),
            next: 0,
            window: (Instant::now(), 0),
        }
    }

    fn handle(&mut self, open: Open) -> Result<String, StatusCode> {
        if self.handles.len() >= MAX_HANDLES {
            return Err(StatusCode::Failure);
        }
        self.next += 1;
        let handle = format!("h{}", self.next);
        self.handles.insert(handle.clone(), open);
        Ok(handle)
    }

    fn node(&self, path: &str) -> Result<(String, Node), StatusCode> {
        let path = normalise(path).ok_or(StatusCode::PermissionDenied)?;
        let node = self.pack.node(&path).ok_or(StatusCode::NoSuchFile)?;
        Ok((path, node))
    }

    /// Wait out the session's byte budget for `n` more bytes.
    async fn pace(&mut self, n: usize) {
        let now = Instant::now();
        if now.saturating_duration_since(self.window.0) >= Duration::from_secs(1) {
            self.window = (now, 0);
        }
        if self.window.1 + n > BYTES_PER_SEC {
            let wait =
                Duration::from_secs(1).saturating_sub(now.saturating_duration_since(self.window.0));
            tokio::time::sleep(wait).await;
            self.window = (Instant::now(), 0);
        }
        self.window.1 += n;
    }
}

impl russh_sftp::server::Handler for AssetsSftp {
    type Error = StatusCode;

    /// Whatever is not answered is refused.
    fn unimplemented(&self) -> StatusCode {
        StatusCode::PermissionDenied
    }

    async fn init(
        &mut self,
        _version: u32,
        _extensions: HashMap<String, String>,
    ) -> Result<Version, StatusCode> {
        Ok(Version::new())
    }

    async fn open(
        &mut self,
        id: u32,
        filename: String,
        pflags: OpenFlags,
        _attrs: FileAttributes,
    ) -> Result<Handle, StatusCode> {
        if pflags.bits() != OpenFlags::READ.bits() {
            return Err(StatusCode::PermissionDenied);
        }
        let Node::File(bytes) = self.node(&filename)?.1 else {
            return Err(StatusCode::Failure);
        };
        let handle = self.handle(Open::File(bytes))?;
        Ok(Handle { id, handle })
    }

    async fn close(&mut self, id: u32, handle: String) -> Result<Status, StatusCode> {
        self.handles.remove(&handle).ok_or(StatusCode::Failure)?;
        Ok(Status {
            id,
            status_code: StatusCode::Ok,
            error_message: "Ok".into(),
            language_tag: "en-US".into(),
        })
    }

    async fn read(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        len: u32,
    ) -> Result<Data, StatusCode> {
        let Some(Open::File(bytes)) = self.handles.get(&handle) else {
            return Err(StatusCode::Failure);
        };
        let bytes: &'static [u8] = bytes;
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        if start >= bytes.len() {
            return Err(StatusCode::Eof);
        }
        let n = usize::try_from(len.min(MAX_READ))
            .unwrap_or(0)
            .min(bytes.len() - start);
        self.pace(n).await;
        let data = bytes
            .get(start..start + n)
            .ok_or(StatusCode::Failure)?
            .to_vec();
        Ok(Data { id, data })
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, StatusCode> {
        let (_, node) = self.node(&path)?;
        Ok(Attrs {
            id,
            attrs: node.attrs(),
        })
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, StatusCode> {
        let (_, node) = self.node(&path)?;
        Ok(Attrs {
            id,
            attrs: node.attrs(),
        })
    }

    async fn fstat(&mut self, id: u32, handle: String) -> Result<Attrs, StatusCode> {
        let node = match self.handles.get(&handle) {
            Some(Open::File(bytes)) => Node::File(bytes),
            Some(Open::Dir { .. }) => Node::Dir,
            None => return Err(StatusCode::Failure),
        };
        Ok(Attrs {
            id,
            attrs: node.attrs(),
        })
    }

    async fn opendir(&mut self, id: u32, path: String) -> Result<Handle, StatusCode> {
        let (path, node) = self.node(&path)?;
        let Node::Dir = node else {
            return Err(StatusCode::Failure);
        };
        let entries = self.pack.listing(&path);
        let handle = self.handle(Open::Dir {
            entries,
            done: false,
        })?;
        Ok(Handle { id, handle })
    }

    async fn readdir(&mut self, id: u32, handle: String) -> Result<Name, StatusCode> {
        let Some(Open::Dir { entries, done }) = self.handles.get_mut(&handle) else {
            return Err(StatusCode::Failure);
        };
        if *done {
            return Err(StatusCode::Eof);
        }
        *done = true;
        let files = entries
            .iter()
            .map(|(name, node)| File::new(name.clone(), node.attrs()))
            .collect();
        Ok(Name { id, files })
    }

    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, StatusCode> {
        let path = normalise(&path).ok_or(StatusCode::PermissionDenied)?;
        Ok(Name {
            id,
            files: vec![File::dummy(path)],
        })
    }
}

/// **A stream whose read errors end it** (G.11): `russh_sftp`'s server loop
/// logs a read error and reads again, so a stream that keeps failing would
/// spin; here any read error is the end of the stream, which ends the loop.
#[derive(Debug)]
pub struct EndOnError<S>(pub S);

impl<S: tokio::io::AsyncRead + Unpin> tokio::io::AsyncRead for EndOnError<S> {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match std::pin::Pin::new(&mut self.0).poll_read(cx, buf) {
            std::task::Poll::Ready(Err(_)) => std::task::Poll::Ready(Ok(())),
            other => other,
        }
    }
}

impl<S: tokio::io::AsyncWrite + Unpin> tokio::io::AsyncWrite for EndOnError<S> {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.0).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests;
