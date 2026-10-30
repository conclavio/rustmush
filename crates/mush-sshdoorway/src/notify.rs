//! `sd_notify` (doc 29 §9.2): `READY=1` once every address is bound, the
//! config parsed and every service answered a `Ping`; `STOPPING=1` when a
//! drain begins. A no-op outside systemd (no `$NOTIFY_SOCKET`).

/// Send one notification. Errors are logged, never fatal.
pub fn notify(state: &str) {
    #[cfg(unix)]
    {
        let Some(path) = std::env::var_os("NOTIFY_SOCKET") else {
            return;
        };
        if let Err(e) = send(&path, state) {
            tracing::warn!(error = %e, state, "sd_notify failed");
        }
    }
    #[cfg(not(unix))]
    {
        let _ = state;
    }
}

#[cfg(unix)]
fn send(path: &std::ffi::OsStr, state: &str) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::net::UnixDatagram;

    let sock = UnixDatagram::unbound()?;
    let bytes = path.as_bytes();
    if let Some(name) = bytes.strip_prefix(b"@") {
        #[cfg(target_os = "linux")]
        {
            use std::os::linux::net::SocketAddrExt;
            let addr = std::os::unix::net::SocketAddr::from_abstract_name(name)?;
            sock.send_to_addr(state.as_bytes(), &addr)?;
            return Ok(());
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = name;
            return Err(std::io::Error::other("abstract sockets need Linux"));
        }
    }
    sock.send_to(state.as_bytes(), std::path::Path::new(path))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    #[test]
    fn notifies_a_path_socket() {
        use super::send;
        let dir = std::env::temp_dir().join(format!("sshdoorway-notify-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("notify.sock");
        let _ = std::fs::remove_file(&path);
        let rx = std::os::unix::net::UnixDatagram::bind(&path).unwrap();
        send(path.as_os_str(), "READY=1").unwrap();
        let mut buf = [0u8; 64];
        let n = rx.recv(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"READY=1");
        std::fs::remove_dir_all(&dir).ok();
    }
}
