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

impl Session {
    pub async fn connect(config: &Ssh) -> Result<Self> {
        let failure = Arc::new(Mutex::new(None));
        let handler = HostCheck {
            config: config.clone(),
            failure: failure.clone(),
        };
        let result = tokio::time::timeout(Duration::from_secs(config.timeout_secs), async {
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
