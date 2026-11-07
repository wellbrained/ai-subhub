//! Start with Windows: one per-user Run-key value (no administrator rights). The registry is the
//! truth; nothing about it is kept in the settings file.
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::core::{PCWSTR, w};

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const VALUE: PCWSTR = w!("AI SubHub");

/// The command currently registered, if any.
pub fn registered() -> Option<String> {
    let mut size = 0u32;
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            VALUE,
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut size),
        )
        .ok()
        .ok()?;
        let mut buf = vec![0u16; (size as usize).div_ceil(2)];
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            VALUE,
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
        .ok()
        .ok()?;
        let len = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..len]))
    }
}

pub fn is_enabled() -> bool {
    registered().is_some()
}

fn current_command() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    Ok(subhub_core::launch::startup_command(&exe))
}

pub fn set_enabled(on: bool) -> Result<(), String> {
    unsafe {
        if on {
            let command = current_command()?;
            let wide: Vec<u16> = command.encode_utf16().chain(std::iter::once(0)).collect();
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                VALUE,
                REG_SZ.0,
                Some(wide.as_ptr().cast()),
                (wide.len() * 2) as u32,
            )
            .ok()
            .map_err(|e| e.to_string())
        } else {
            let r = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE);
            // Already absent is fine.
            if r.is_ok() || r.0 == 2 {
                Ok(())
            } else {
                Err(format!("Windows refused the change (error {}).", r.0))
            }
        }
    }
}

/// An enabled entry follows the exe when it runs from a new path (a moved or updated build).
/// Development builds (cargo's `target\debug|release`) never take it over: the installed app
/// stays on startup while a dev build runs for testing (owner, 2026-11-07).
pub fn refresh_if_enabled() {
    // A development build must not take over the installed app's entry.
    if std::env::current_exe().map_or(true, |p| subhub_core::update::is_dev_build(&p)) {
        return;
    }
    if let (Some(current), Ok(want)) = (registered(), current_command())
        && current != want
    {
        let _ = set_enabled(true);
    }
}
