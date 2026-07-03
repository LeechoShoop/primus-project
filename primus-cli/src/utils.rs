// primus-cli/src/utils.rs
//
// Shared utilities for the Primus CLI.

/// Format a raw public-key byte slice as a shortened hex address.
#[allow(dead_code)]
pub fn short_hex(bytes: &[u8]) -> String {
    let hex = hex::encode(bytes);
    if hex.len() > 16 {
        format!("{}…{}", &hex[..8], &hex[hex.len() - 8..])
    } else {
        hex
    }
}

pub fn get_secure_ipc_path(port: u16) -> anyhow::Result<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut path = if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
            std::path::PathBuf::from(runtime_dir)
        } else {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
            let mut p = std::path::PathBuf::from(home);
            p.push(".primus");
            p.push("run");
            p
        };
        std::fs::create_dir_all(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
        path.push(format!("primus-{}.sock", port));
        Ok(path.to_string_lossy().to_string())
    }

    #[cfg(windows)]
    {
        let output = std::process::Command::new("whoami")
            .args(["/user", "/fo", "csv", "/nh"])
            .output()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let parts: Vec<&str> = stdout.trim().split(',').collect();
        if parts.len() == 2 {
            let sid = parts[1].trim_matches('"');
            Ok(format!(r"\\.\pipe\primus-nexus-{}-{}", port, sid))
        } else {
            anyhow::bail!("Failed to retrieve user SID on Windows for secure IPC path")
        }
    }
}

pub fn verify_ipc_ownership(_path: &str) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use anyhow::Context;
        use std::os::unix::fs::MetadataExt;
        let meta = std::fs::metadata(path).context("Failed to get socket metadata")?;
        let socket_uid = meta.uid();
        
        let output = std::process::Command::new("id").arg("-u").output()?;
        let my_uid_str = String::from_utf8_lossy(&output.stdout);
        let my_uid: u32 = my_uid_str.trim().parse()?;
        
        if socket_uid != my_uid {
            anyhow::bail!("Security exception: IPC socket is not owned by the current user (uid {} != {}). Possible shadowing attack.", socket_uid, my_uid);
        }
    }
    
    #[cfg(windows)]
    {
        // On Windows, checking named pipe ACLs requires winapi/windows-sys.
        // We rely on the SID embedded in the path for isolation.
    }
    Ok(())
}
