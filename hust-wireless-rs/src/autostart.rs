use crate::config;
use crate::error::Result;

#[cfg(not(windows))]
use crate::error::AppError;

pub const RUN_NAME: &str = "HUSTWiredAuth";

#[cfg(windows)]
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

pub fn autostart_command() -> Result<String> {
    let exe = std::env::current_exe()?;
    Ok(format!("\"{}\" watch", exe.display()))
}

#[cfg(windows)]
pub fn install() -> Result<()> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    config::load_credentials()?;
    let command = autostart_command()?;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu.create_subkey(RUN_KEY)?;
    key.set_value(RUN_NAME, &command)?;
    Ok(())
}

#[cfg(not(windows))]
pub fn install() -> Result<()> {
    Err(AppError::msg("开机自启仅支持 Windows"))
}

#[cfg(windows)]
pub fn uninstall() -> Result<()> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_SET_VALUE};

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = match hkcu.open_subkey_with_flags(RUN_KEY, KEY_SET_VALUE) {
        Ok(key) => key,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    match key.delete_value(RUN_NAME) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(not(windows))]
pub fn uninstall() -> Result<()> {
    Ok(())
}
