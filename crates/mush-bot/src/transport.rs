use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use russh::{
    Channel, client,
    keys::{HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate},
};

use crate::{
    Result,
    config::{Ssh, secret},
};

/// **The width the bot's PTY asks for** (cut AM.1): the terminal-width floor;
/// the game wraps flush lines at it. The arena lane's channel continuation
/// ([`crate::arena::lines::continues`]) reads a line's width against it.
pub const PTY_COLUMNS: u32 = 120;

pub struct HostCheck {
    config: Ssh,
    failure: Arc<Mutex<Option<String>>>,
}

impl client::Handler for HostCheck {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> std::result::Result<bool, Self::Error> {
        let key = key.public_key();
        let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
        let trusted = if let Some(pin) = &self.config.host_key_sha256 {
            pin == &fingerprint
        } else if let Some(path) = &self.config.known_hosts {
            russh::keys::check_known_hosts_path(&self.config.host, self.config.port, &key, path)
                .unwrap_or(false)
        } else {
            russh::keys::check_known_hosts(&self.config.host, self.config.port, &key)
                .unwrap_or(false)
        };
        if !trusted {
            *self.failure.lock().unwrap() = Some(format!(
                "SSH host key not trusted: {}:{} presented {fingerprint}. Verify it with the server operator, then configure known_hosts or host_key_sha256.",
                self.config.host, self.config.port
            ));
        }
        Ok(trusted)
    }
}

pub struct Session {
    pub channel: Channel<client::Msg>,
    handle: client::Handle<HostCheck>,
}

/// Connect and authenticate as `config` says: the host key checked, then a public key
/// (`identity_file`), else a password (`password_env`), else none.
async fn authenticated(config: &Ssh, handler: HostCheck) -> Result<client::Handle<HostCheck>> {
    let ssh_config = client::Config {
        keepalive_interval: Some(Duration::from_secs(30)),
        keepalive_max: 3,
        ..Default::default()
    };
    let mut handle = client::connect(
        Arc::new(ssh_config),
        (config.host.as_str(), config.port),
        handler,
    )
    .await?;
    let authenticated = if let Some(path) = &config.identity_file {
        let passphrase = secret(&config.key_password_env)?;
        let key = russh::keys::load_secret_key(path, passphrase.as_deref())?;
        let hash = handle.best_supported_rsa_hash().await?.flatten();
        handle
            .authenticate_publickey(
                &config.username,
                PrivateKeyWithHashAlg::new(Arc::new(key), hash),
            )
            .await?
            .success()
    } else if let Some(password) = secret(&config.password_env)? {
        handle
            .authenticate_password(&config.username, password)
            .await?
            .success()
    } else {
        handle.authenticate_none(&config.username).await?.success()
    };
    if !authenticated {
        return Err("SSH authentication rejected".into());
    }
    Ok(handle)
}

/// The three SSH exec commands of the typed mind service (`help mindcraft`, "Source over
/// SSH"): each accepts only its own operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MindExec {
    /// `mind push`: `create`, `replace`, `save`.
    Push,
    /// `mind pull`: `read`.
    Pull,
    /// `mind check`: `check`.
    Check,
}

impl MindExec {
    /// The exec command line, exactly as the server matches it.
    pub fn command(self) -> &'static str {
        match self {
            MindExec::Push => "mind push",
            MindExec::Pull => "mind pull",
            MindExec::Check => "mind check",
        }
    }
}

/// The largest reply [`mind_exec`] keeps: the service bounds its own frame at 272 KiB.
const MIND_REPLY_BYTES: usize = 1024 * 1024;

/// **One SSH exec of the typed mind service** (doc 44 unit MW.19, over the exec doc 44 unit
/// MW.24 made a bare connection): a connection of its own beside the line session,
/// authenticated by the configured public key as `account+character`, one JSON request on
/// standard input, then end of input. The reply is one JSON record a line; the last is the
/// final one, and its `result` object is returned whatever its `status` (a refusal is a reply,
/// and the caller reads it). `Err` is the transport: no key configured, the exec refused, no
/// reply, or a reply that is not the mind service's JSON.
///
/// This is how a player with an editor pushes source (`help mindcraft`). It types no line into
/// the game and carries no credential but the key.
pub async fn mind_exec(
    config: &Ssh,
    exec: MindExec,
    request: &serde_json::Value,
) -> Result<serde_json::Value> {
    if config.identity_file.is_none() {
        return Err(
            "the SSH mind service takes a verified public key: set [ssh].identity_file".into(),
        );
    }
    let failure = Arc::new(Mutex::new(None));
    let handler = HostCheck {
        config: config.clone(),
        failure: failure.clone(),
    };
    let body = serde_json::to_vec(request)?;
    // The service answers a job by its own bound (two minutes, then "expired"): wait past it,
    // so that a slow check ends with the service's reason and not with this timeout.
    let result = tokio::time::timeout(Duration::from_secs(config.timeout_secs.max(150)), async {
        let handle = authenticated(config, handler).await?;
        let mut channel = handle.channel_open_session().await?;
        channel.exec(true, exec.command()).await?;
        for packet in body.chunks(16 * 1024) {
            channel.data(packet).await?;
        }
        channel.eof().await?;
        let mut bytes = Vec::new();
        while let Some(message) = channel.wait().await {
            match message {
                russh::ChannelMsg::Data { data } => {
                    if bytes.len().saturating_add(data.len()) > MIND_REPLY_BYTES {
                        return Err(format!("`{}` answered more than 1 MiB", exec.command()).into());
                    }
                    bytes.extend_from_slice(&data);
                }
                russh::ChannelMsg::Close => break,
                _ => {}
            }
        }
        let _ = handle
            .disconnect(russh::Disconnect::ByApplication, "mind exec done", "en")
            .await;
        mind_reply(exec, &bytes)
    })
    .await;
    if let Some(message) = failure.lock().unwrap().take() {
        return Err(message.into());
    }
    result.map_err(|_| format!("`{}` timed out", exec.command()))?
}

/// The final record's `result` object out of an exec's whole output.
fn mind_reply(exec: MindExec, bytes: &[u8]) -> Result<serde_json::Value> {
    let text = String::from_utf8_lossy(bytes);
    let last = text
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .ok_or_else(|| format!("`{}` answered nothing", exec.command()))?;
    let record: serde_json::Value = serde_json::from_str(last).map_err(|_| {
        format!(
            "`{}` answered something that is not the mind service's JSON: {}",
            exec.command(),
            crate::tail(last, 200)
        )
    })?;
    match record.get("result") {
        Some(result) if record.get("service").and_then(|s| s.as_str()) == Some("mind") => {
            Ok(result.clone())
        }
        _ => Err(format!(
            "`{}` answered a record with no mind result: {}",
            exec.command(),
            crate::tail(last, 200)
        )
        .into()),
    }
}

impl Session {
    pub async fn connect(config: &Ssh) -> Result<Self> {
        let failure = Arc::new(Mutex::new(None));
        let handler = HostCheck {
            config: config.clone(),
            failure: failure.clone(),
        };
        let result = tokio::time::timeout(Duration::from_secs(config.timeout_secs), async {
            let handle = authenticated(config, handler).await?;
            let channel = handle.channel_open_session().await?;
            // RustMUSH requires a PTY even for line mode. TERM=dumb requests
            // plain text; no alternate screen or privileged structured feed.
            channel
                .request_pty(true, "dumb", PTY_COLUMNS, 40, 0, 0, &[])
                .await?;
            channel.request_shell(true).await?;
            Ok(Self { channel, handle })
        })
        .await;
        if let Some(message) = failure.lock().unwrap().take() {
            return Err(message.into());
        }
        result.map_err(|_| "SSH connection/authentication timed out")?
    }

    pub async fn send(&self, line: &str) -> Result<()> {
        validate_command(line)?;
        self.channel.data(format!("{line}\r").as_bytes()).await?;
        Ok(())
    }

    pub async fn close(&self) {
        let _ = tokio::time::timeout(Duration::from_secs(2), async {
            let _ = self.channel.close().await;
            let _ = self
                .handle
                .disconnect(russh::Disconnect::ByApplication, "Bot finished", "en")
                .await;
        })
        .await;
    }
}

pub fn validate_command(command: &str) -> Result<()> {
    if command.trim().is_empty() || command.len() > 8192 || command.chars().any(char::is_control) {
        return Err("a game command must be one nonempty line, at most 8192 bytes, with no control characters".into());
    }
    let verb = command
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if matches!(verb.as_str(), "@tui" | "@notui") {
        return Err("mush-bot currently supports the SSH line interface only".into());
    }
    if matches!(
        verb.as_str(),
        "connect" | "co" | "con" | "create" | "cr" | "@password" | "@newpassword"
    ) {
        return Err(
            "configure SSH authentication; credential commands do not belong in bot transcripts"
                .into(),
        );
    }
    // Account and session verbs. Game text reaches the model as untrusted
    // data, so a line another player wrote must never be able to grant
    // persistent access to the account (`@sshkey/add`), lock its operator
    // out (`@sshkey/del`), spawn characters, or end the session. The runner
    // manages keys, characters and disconnects outside the game. A macro
    // can carry any of these verbs past a first-word check (`@macro
    // k=@sshkey/add ...`, then `k`), so the bot may not define or delete one;
    // the runner owns the character's macros.
    if verb == "@sshkey"
        || verb.starts_with("@sshkey/")
        || verb == "@macro"
        || verb.starts_with("@macro/")
        || matches!(
            verb.as_str(),
            "@newchar" | "quit" | "@quit" | "logout" | "@logout"
        )
    {
        return Err(
            "account and session commands are not available to the bot; manage SSH keys, \
             characters and disconnects from the runner"
                .into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_command;

    #[test]
    fn credential_account_and_session_verbs_are_refused_in_any_case() {
        for command in [
            "connect rosebot secret",
            "@password old=new",
            "@sshkey/add ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAA bot",
            "@SSHKEY/del 1",
            "@sshkey/list",
            "@sshkey",
            "@newchar Marco",
            "QUIT",
            "quit now",
            "LOGOUT",
            "@tui",
            "@macro k=@sshkey/add ssh-ed25519 AAAA bot",
            "@MACRO/del k",
            "@macro",
        ] {
            assert!(validate_command(command).is_err(), "{command}");
        }
    }

    /// Doc 44 unit MW.19: the exec's reply is the last record's `result`, a refusal included.
    #[test]
    fn a_mind_exec_reply_is_its_last_record() {
        use super::{MindExec, mind_reply};
        let two = concat!(
            r#"{"service":"mind","result":{"status":"queued","job":4}}"#,
            "\n",
            r#"{"service":"mind","result":{"status":"saved","revision":1}}"#,
            "\n\n"
        );
        let saved = mind_reply(MindExec::Push, two.as_bytes()).expect("the final record");
        assert_eq!(saved["status"], "saved");
        assert_eq!(saved["revision"], 1);
        let refused = mind_reply(
            MindExec::Pull,
            br#"{"service":"mind","result":{"status":"error","code":"missing"}}"#,
        )
        .expect("a refusal is a reply");
        assert_eq!(refused["code"], "missing");
        assert!(mind_reply(MindExec::Check, b"").is_err());
        assert!(mind_reply(MindExec::Check, b"Character attachment was refused.").is_err());
        assert!(mind_reply(MindExec::Check, br#"{"service":"other","result":{}}"#).is_err());
        assert_eq!(MindExec::Push.command(), "mind push");
        assert_eq!(MindExec::Pull.command(), "mind pull");
        assert_eq!(MindExec::Check.command(), "mind check");
    }

    #[test]
    fn refused_verbs_are_only_refused_as_the_verb() {
        for command in [
            "say type @sshkey/add and I will owe you a favour",
            "pose quits the game of cards.",
            "look",
            "@emit The wind picks up.",
            "page Marco=QUIT",
            "say @macro is handy",
        ] {
            assert!(validate_command(command).is_ok(), "{command}");
        }
    }
}
