//! WS01-S7: a faulted world is a FAILURE exit, and the shutdown BGSAVE is
//! gated on it.
//!
//! Before this step every one of `serve`'s three world-join sites threw the
//! join result away (`let _ = world_join.await`, `_ = &mut world_join`). A
//! world task that unwound mid-command was therefore indistinguishable from an
//! in-game `@shutdown`: `mushd` logged "world task ended on its own" at INFO
//! and `main` returned [`ExitCode::SUCCESS`](std::process::ExitCode). Nothing
//! downstream — a supervisor, a systemd `Restart=on-failure`, or a test harness
//! watching for a fault signal — could tell that the world had died.
//!
//! The proof has to be a REAL process, because an exit code is the deliverable.
//! Both halves boot the actual `mushd` binary over a throwaway in-memory store,
//! connect over real SSH as a seeded God account, and type one line:
//!
//! - **The fault.** One world turn is armed to panic through `mush-server`'s
//!   `process_fault` seam (the arming environment WS01-S8 introduced, enabled
//!   here by the `[dev-dependencies]` edge on `mush-server`'s
//!   `fault-injection` feature). The process must exit non-zero, log the fault
//!   at ERROR, and skip the shutdown BGSAVE with a line that names it — because
//!   the world's RAM at that moment is a torn mid-command state, and a snapshot
//!   would canonize it as the on-disk truth.
//! - **The contrast.** The same binary, the same account, an in-game
//!   `@shutdown`. Exit zero, exactly one shutdown BGSAVE, and not one fault
//!   line. A false positive here would restart-loop a live box, so the
//!   ordinary path is pinned as hard as the faulting one.
//!
//! The arming is one-shot and inert without the environment, so an ordinary
//! `cargo build -p mushd` carries no seam at all.

use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use mushd::accounts::hash_password;
use mushd::logging;
use russh::ChannelMsg;

/// How long one client step (a login, a line, an expected banner) may take.
const STEP_TIMEOUT: Duration = Duration::from_secs(120);
/// How long the child gets to bind its SSH listener. A debug-build boot walks
/// twenty-odd modules, their migrations and two key rings first.
const BOOT_TIMEOUT: Duration = Duration::from_secs(180);
/// How long the child gets to run its whole shutdown tail and exit. The tail
/// itself is bounded (writer drain 15 s, three store steps 10 s each).
const EXIT_TIMEOUT: Duration = Duration::from_secs(120);

const ACCOUNT: &str = "boss";
const PASSWORD: &str = "gm-secret-1";

/// The command line whose world turn is armed to panic. It is deliberately not
/// a registered command: the arming sits at the world task's dispatch boundary,
/// ahead of `engine.step`, so what the line *means* never matters.
const FAULT_LINE: &str = "@ws01s7-fault";

/// `process_fault`'s arming contract, named for the step that introduced it
/// (WS01-S8). Reused verbatim here rather than duplicated — the seam is the
/// same one, and this step is proving what the *other* half of the fork does
/// with the fault it produces.
const PANIC_CMD_ENV: &str = "RUSTMUSH_WS01_S8_PANIC_CMD";
const PANIC_MARKER_ENV: &str = "RUSTMUSH_WS01_S8_PANIC_MARKER";

// ---------------------------------------------------------------------------
// The child server
// ---------------------------------------------------------------------------

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mushd-ws01s7-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Reserve a localhost port and hand back its number. The listener is dropped
/// immediately; the child rebinds it.
fn reserve_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("reserve a port");
    let port = listener.local_addr().expect("reserved address").port();
    drop(listener);
    port
}

/// TOML paths are written with forward slashes so a Windows scratch path needs
/// no backslash escaping.
fn toml_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

/// Write a `mushd.toml` for a throwaway server: memory store, no rulesets (the
/// staff `@shutdown` this test drives is a server command, not a ruleset one),
/// file logging on so the parent can read the shutdown decision back, and both
/// listeners on ports reserved above.
fn write_config(dir: &Path) -> (PathBuf, SocketAddr) {
    let ssh_port = reserve_port();
    let web_port = reserve_port();
    let path = dir.join("mushd.toml");
    std::fs::write(
        &path,
        format!(
            "[server]\n\
             name = \"ws01s7\"\n\
             data_dir = \"{dir}\"\n\
             help_dir = \"{dir}/help\"\n\
             \n\
             [store]\n\
             store = \"memory\"\n\
             \n\
             [ssh]\n\
             listen = \"127.0.0.1:{ssh_port}\"\n\
             \n\
             [web]\n\
             listen = \"127.0.0.1:{web_port}\"\n\
             \n\
             [log]\n\
             level = \"info\"\n\
             file = true\n",
            dir = toml_path(dir),
        ),
    )
    .expect("write mushd.toml");
    let addr = format!("127.0.0.1:{ssh_port}").parse().expect("ssh addr");
    (path, addr)
}

/// Seed a staff account through the boot-time `accounts.json` import — the one
/// hook that puts credentials into a child process's own in-memory store. It
/// links `#1` (God, seeded by `load_or_seed_world` in the same boot), so the
/// connection this test makes can drive `@shutdown`.
fn write_accounts_json(dir: &Path) {
    let hash = hash_password(PASSWORD).expect("hash the test password");
    std::fs::write(
        dir.join("accounts.json"),
        format!(
            "{{\"version\":1,\"accounts\":[{{\"name\":\"{ACCOUNT}\",\"hash\":\"{hash}\",\
             \"characters\":[{{\"id\":1,\"name\":\"God\"}}]}}]}}"
        ),
    )
    .expect("write accounts.json");
}

/// Boot a real `mushd` in its own process, with stdout/stderr captured to files
/// (never pipes: a debug boot logs more than a pipe buffer holds, and nobody is
/// reading it until the child exits).
fn spawn_mushd(dir: &Path, config: &Path, env: &[(&str, String)]) -> tokio::process::Child {
    let stdout = std::fs::File::create(dir.join("stdout.txt")).expect("stdout file");
    let stderr = std::fs::File::create(dir.join("stderr.txt")).expect("stderr file");
    let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_mushd"));
    cmd.arg("--config")
        .arg(config)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .kill_on_drop(true);
    for (key, value) in env {
        cmd.env(key, value);
    }
    cmd.spawn().expect("spawn mushd")
}

/// Wait until the child's SSH listener accepts, failing loudly (with whatever
/// the child managed to say) if it never does.
async fn wait_for_ssh(addr: SocketAddr, dir: &Path, child: &mut tokio::process::Child) {
    let deadline = tokio::time::Instant::now() + BOOT_TIMEOUT;
    loop {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            return;
        }
        if let Ok(Some(status)) = child.try_wait() {
            panic!(
                "mushd exited during boot with {status:?}\n--- stderr ---\n{}",
                read_or_empty(&dir.join("stderr.txt"))
            );
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "mushd never bound {addr}\n--- stderr ---\n{}",
            read_or_empty(&dir.join("stderr.txt"))
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Wait for the child to exit, killing it (and failing) if it never does — a
/// wedged shutdown tail is itself a failure of this step.
async fn wait_for_exit(child: &mut tokio::process::Child, dir: &Path) -> std::process::ExitStatus {
    match tokio::time::timeout(EXIT_TIMEOUT, child.wait()).await {
        Ok(status) => status.expect("wait on mushd"),
        Err(_) => {
            let _ = child.kill().await;
            panic!(
                "mushd did not exit within {EXIT_TIMEOUT:?}\n--- log ---\n{}",
                read_log(dir)
            );
        }
    }
}

fn read_or_empty(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// The one rotated `mushd.log*` the child wrote under its data dir.
fn read_log(dir: &Path) -> String {
    let logs = dir.join("logs");
    let prefix = logging::log_file_name(logging::DEFAULT_ROLE);
    let Ok(entries) = std::fs::read_dir(&logs) else {
        return String::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&prefix))
        })
        .map(|p| read_or_empty(&p))
        .collect::<Vec<_>>()
        .join("\n")
}

fn count(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

// ---------------------------------------------------------------------------
// The SSH client
// ---------------------------------------------------------------------------

struct AcceptAll;

impl russh::client::Handler for AcceptAll {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

/// Drop ANSI CSI sequences so assertions match what a player reads.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for f in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&f) {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

struct Term {
    channel: russh::Channel<russh::client::Msg>,
    seen: String,
}

impl Term {
    /// Read until one of the two needles shows up, and say which. The
    /// char-select screen is skipped for a single-character account on some
    /// paths, so the login has to tolerate either landing.
    async fn expect_either(&mut self, a: &str, b: &str) -> bool {
        let deadline = tokio::time::Instant::now() + STEP_TIMEOUT;
        loop {
            let text = strip_ansi(&self.seen);
            if text.contains(a) {
                return true;
            }
            if text.contains(b) {
                return false;
            }
            let msg = tokio::time::timeout_at(deadline, self.channel.wait())
                .await
                .unwrap_or_else(|_| {
                    panic!("timeout waiting for {a:?} or {b:?}; seen: {:?}", self.seen)
                });
            match msg {
                Some(ChannelMsg::Data { data }) => {
                    self.seen.push_str(&String::from_utf8_lossy(&data));
                }
                Some(_) => {}
                None => panic!(
                    "channel closed waiting for {a:?} or {b:?}; seen: {:?}",
                    self.seen
                ),
            }
        }
    }

    async fn expect(&mut self, needle: &str) {
        let deadline = tokio::time::Instant::now() + STEP_TIMEOUT;
        while !strip_ansi(&self.seen).contains(needle) {
            let msg = tokio::time::timeout_at(deadline, self.channel.wait())
                .await
                .unwrap_or_else(|_| {
                    panic!("timeout waiting for {needle:?}; seen: {:?}", self.seen)
                });
            match msg {
                Some(ChannelMsg::Data { data }) => {
                    self.seen.push_str(&String::from_utf8_lossy(&data));
                }
                Some(_) => {}
                None => panic!(
                    "channel closed waiting for {needle:?}; seen: {:?}",
                    self.seen
                ),
            }
        }
    }

    async fn send_line(&mut self, line: &str) {
        let bytes = format!("{line}\r").into_bytes();
        self.channel.data(&bytes[..]).await.expect("send line");
    }
}

/// Password-login the seeded staff account and land in God, through the real
/// SSH front door.
async fn connect_as_god(addr: SocketAddr) -> (russh::client::Handle<AcceptAll>, Term) {
    let config = Arc::new(russh::client::Config::default());
    let mut handle = russh::client::connect(config, addr, AcceptAll)
        .await
        .expect("connect");
    assert!(
        handle
            .authenticate_password(ACCOUNT, PASSWORD)
            .await
            .expect("auth")
            .success(),
        "the imported account must authenticate"
    );
    let channel = handle.channel_open_session().await.expect("open channel");
    channel
        .request_pty(true, "xterm-256color", 80, 24, 0, 0, &[])
        .await
        .expect("pty");
    channel.request_shell(true).await.expect("shell");
    let mut term = Term {
        channel,
        seen: String::new(),
    };
    if !term
        .expect_either("Connected as God", "Characters on")
        .await
    {
        term.send_line("God").await;
        term.expect("Connected as God").await;
    }
    (handle, term)
}

// ---------------------------------------------------------------------------
// The proofs
// ---------------------------------------------------------------------------

/// The failure this step closes, end to end against a real process.
///
/// One world turn really panics. `mushd` must (a) exit non-zero — before this
/// step it exited `SUCCESS`, so nothing upstream could tell a dead world from
/// an operator shutdown; (b) say so at ERROR; and (c) refuse the shutdown
/// BGSAVE with a line naming the fault, so the torn mid-command RAM never
/// becomes the on-disk truth.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_faulted_world_exits_non_zero_and_skips_the_shutdown_bgsave() {
    let dir = scratch_dir("fault");
    let (config, addr) = write_config(&dir);
    write_accounts_json(&dir);

    let marker = dir.join("panic-marker");
    let mut child = spawn_mushd(
        &dir,
        &config,
        &[
            (PANIC_CMD_ENV, FAULT_LINE.to_owned()),
            (PANIC_MARKER_ENV, marker.display().to_string()),
        ],
    );
    wait_for_ssh(addr, &dir, &mut child).await;

    let (handle, mut term) = connect_as_god(addr).await;
    term.send_line(FAULT_LINE).await;

    let status = wait_for_exit(&mut child, &dir).await;
    drop(handle);

    let log = read_log(&dir);
    assert!(
        marker.exists(),
        "the one-shot arming never fired — is `mush-server/fault-injection` on for \
         `cargo test -p mushd`?\n--- log ---\n{log}"
    );
    // WS22-S3: not merely non-zero — exactly `1`, the shared vocabulary's
    // catch-all (`mush_config::exit`). The code an operator's supervisor reads
    // has to separate a dead world (1) from a world that would not stop (3),
    // and a test that accepts any failure would not notice the two collapsing.
    assert_eq!(
        status.code(),
        Some(1),
        "a faulted world must exit 1, got {status:?}\n--- log ---\n{log}"
    );

    // (d) the fault was logged at ERROR, naming the turn that died.
    let fault_lines: Vec<&str> = log
        .lines()
        .filter(|l| l.contains("world task faulted"))
        .collect();
    assert!(
        !fault_lines.is_empty(),
        "the fault must be on the record: {log}"
    );
    assert!(
        fault_lines.iter().all(|l| l.contains("ERROR")),
        "the fault is an ERROR, not an INFO: {fault_lines:?}"
    );
    assert!(
        fault_lines.iter().any(|l| l.contains(FAULT_LINE)),
        "the fault detail must name the armed turn: {fault_lines:?}"
    );
    // ...and the pre-step INFO line is exactly what must NOT be said.
    assert!(
        !log.contains("world task ended on its own"),
        "a fault is not an ordinary self-ending exit: {log}"
    );

    // (c) not one BGSAVE, and the skip names the fault.
    assert_eq!(
        count(&log, "shutdown BGSAVE triggered"),
        0,
        "a faulted world must never be snapshotted: {log}"
    );
    assert!(
        log.contains("shutdown BGSAVE skipped: the world task faulted"),
        "the skip must name the fault: {log}"
    );

    // (b) the integrity latch is set: the caught fault froze persistence on its
    // way out. Both refusals must hold at once — the latch and the fault gate
    // are separately derived, and the truth table in `main.rs` pins that
    // neither is redundant.
    assert!(
        log.contains("PERSISTENCE INTEGRITY BREACH"),
        "the caught fault freezes persistence: {log}"
    );

    // `main`'s failure text reaches the operator on stderr, not only the file.
    let stderr = read_or_empty(&dir.join("stderr.txt"));
    assert!(
        stderr.contains("mushd: world task faulted"),
        "stderr: {stderr}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// The contrast, and the reason this gate cannot be derived from "the world
/// stopped": an in-game `@shutdown` is an ordinary exit. Zero, one BGSAVE, no
/// fault. A false positive here would restart-loop a live box.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_in_game_shutdown_still_exits_zero_with_one_bgsave() {
    let dir = scratch_dir("shutdown");
    let (config, addr) = write_config(&dir);
    write_accounts_json(&dir);

    // No arming environment at all: the same binary, the ordinary path.
    let mut child = spawn_mushd(&dir, &config, &[]);
    wait_for_ssh(addr, &dir, &mut child).await;

    let (handle, mut term) = connect_as_god(addr).await;
    term.send_line("@shutdown").await;

    let status = wait_for_exit(&mut child, &dir).await;
    drop(handle);

    let log = read_log(&dir);
    assert_eq!(
        status.code(),
        Some(0),
        "an in-game @shutdown is an ordinary exit, got {status:?}\n--- log ---\n{log}"
    );
    assert!(
        log.contains("world task ended on its own"),
        "the ordinary self-ending exit still logs as one: {log}"
    );
    assert!(
        !log.contains("world task faulted"),
        "a clean shutdown must never be read as a fault: {log}"
    );
    assert_eq!(
        count(&log, "shutdown BGSAVE triggered"),
        1,
        "exactly one shutdown BGSAVE: {log}"
    );
    assert!(
        !log.contains("shutdown BGSAVE skipped"),
        "nothing may skip the ordinary snapshot: {log}"
    );
    assert!(
        log.contains("mushd exiting at"),
        "the clean tail ran: {log}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
