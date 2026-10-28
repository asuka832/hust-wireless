use std::fs;
use std::path::{Path, PathBuf};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};

use crate::dpapi;
use crate::error::{AppError, Result};

const CONFIG_DIR: &str = "hust-wireless-cli";

pub fn config_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
        .join(CONFIG_DIR)
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

pub fn login_url_path() -> PathBuf {
    config_dir().join("wired-login-url.txt")
}

pub fn log_path() -> PathBuf {
    config_dir().join("hust-auth.log")
}

/// Candidate locations written by older versions of this tool. These files
/// are read and copied into the user config directory, never modified.
pub fn legacy_login_url_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        paths.push(parent.join("wired-login-url.txt"));
    }
    if let Ok(cwd) = std::env::current_dir() {
        paths.push(cwd.join("wired-login-url.txt"));
    }
    paths.retain(|path| path != &login_url_path());
    paths.dedup();
    paths
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredConfig {
    username: String,
    password_dpapi: String,
    #[serde(default)]
    service: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credentials {
    pub username: String,
    pub password: String,
    pub service: String,
}

pub fn save_credentials(username: &str, password: &str, service: &str) -> Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let protected = dpapi::protect(password.as_bytes())?;
    let stored = StoredConfig {
        username: username.to_string(),
        password_dpapi: STANDARD.encode(protected),
        service: service.to_string(),
    };
    fs::write(path, serde_json::to_vec_pretty(&stored)?)?;
    Ok(())
}

pub fn load_credentials() -> Result<Credentials> {
    let path = config_path();
    let raw = match fs::read(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(AppError::msg("尚未设置账号，请先选择菜单 4 设置账号"));
        }
        Err(error) => return Err(error.into()),
    };
    let stored: StoredConfig = serde_json::from_slice(&raw)
        .map_err(|_| AppError::msg("账号配置已损坏，请重新运行 setup"))?;
    let protected = STANDARD
        .decode(&stored.password_dpapi)
        .map_err(|_| AppError::msg("账号配置已损坏，请重新运行 setup"))?;
    let password = dpapi::unprotect(&protected)?;
    let password = String::from_utf8(password)
        .map_err(|_| AppError::msg("账号配置已损坏，请重新运行 setup"))?;
    Ok(Credentials {
        username: stored.username,
        password,
        service: stored.service,
    })
}

pub fn remove_config() -> Result<()> {
    remove_if_exists(&config_path())
}

pub fn remove_log_with_retry() -> Result<()> {
    let path = log_path();
    for attempt in 0..30 {
        match fs::remove_file(&path) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) if attempt == 29 => {
                return Err(AppError::msg(format!(
                    "后台监测仍占用日志文件；请关闭监测窗口后重试卸载（{error}）"
                )));
            }
            Err(_) => {
                if attempt == 0 {
                    println!("正在等待后台监测退出…");
                }
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        }
    }
    Ok(())
}

pub fn remove_empty_config_dir() {
    // Leave the directory in place if it contains unrelated files.
    let _ = fs::remove_dir(config_dir());
}

fn remove_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvRestore(Option<OsString>);

    impl Drop for EnvRestore {
        fn drop(&mut self) {
            unsafe {
                match &self.0 {
                    Some(value) => std::env::set_var("APPDATA", value),
                    None => std::env::remove_var("APPDATA"),
                }
            }
        }
    }

    #[test]
    fn credentials_roundtrip_hides_password() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _restore = EnvRestore(std::env::var_os("APPDATA"));
        let temp =
            std::env::temp_dir().join(format!("hust-wireless-config-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).unwrap();
        unsafe {
            std::env::set_var("APPDATA", &temp);
        }

        save_credentials("student", "private-password", "").unwrap();
        let saved = std::fs::read_to_string(config_path()).unwrap();
        assert!(!saved.contains("private-password"));

        let credentials = load_credentials().unwrap();
        assert_eq!(credentials.username, "student");
        assert_eq!(credentials.password, "private-password");
        assert_eq!(credentials.service, "");

        let _ = std::fs::remove_dir_all(&temp);
    }
}
