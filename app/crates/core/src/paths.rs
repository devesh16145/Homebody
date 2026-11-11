//! Where things live on disk. One directory per user, private to that user.

use std::path::PathBuf;

/// `$SAI_DATA_DIR` if set (tests, development), else the platform data directory + `SandboxAI`.
/// macOS: `~/Library/Application Support/SandboxAI`.
pub fn data_dir() -> PathBuf {
    if let Ok(p) = std::env::var("SAI_DATA_DIR") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("SandboxAI")
}

pub fn ensure_data_dir() -> std::io::Result<PathBuf> {
    let d = data_dir();
    std::fs::create_dir_all(&d)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700));
    }
    Ok(d)
}

pub fn config_file() -> PathBuf { data_dir().join("settings.json") }
pub fn ledger_file() -> PathBuf { data_dir().join("ledger.jsonl") }
pub fn ledger_key() -> PathBuf { data_dir().join("ledger.key") }
pub fn control_socket() -> PathBuf { data_dir().join("control.sock") }
/// The local certificate authority used for looking inside (see `inspect`). The key never leaves here.
pub fn inspect_key() -> PathBuf { data_dir().join("inspect-ca.key") }
pub fn inspect_cert() -> PathBuf { data_dir().join("inspect-ca.pem") }
pub fn logs_dir() -> PathBuf { data_dir().join("logs") }

pub fn downloads_dir() -> PathBuf {
    dirs::download_dir().or_else(dirs::home_dir).unwrap_or_else(|| PathBuf::from("."))
}
