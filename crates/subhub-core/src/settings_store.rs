use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Context;
use chrono::Utc;

use crate::settings::{SCHEMA_VERSION, Settings};

/// Missing file -> defaults. Unreadable/corrupt/newer-schema file -> moved to `settings.json.bad`
/// (a timestamped name if one exists) and defaults returned, so the only copy is never overwritten.
pub(crate) fn load(path: &Path) -> anyhow::Result<Settings> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Settings::default()),
        Err(_) => anyhow::bail!("Settings file could not be read"),
    };
    let bytes = bytes
        .strip_prefix(&[0xEF, 0xBB, 0xBF][..])
        .unwrap_or(&bytes);
    match serde_json::from_slice::<Settings>(bytes) {
        Ok(s) if s.schema_version <= SCHEMA_VERSION => Ok(sanitize(s)),
        _ => {
            quarantine(path)?;
            Ok(Settings::default())
        }
    }
}

fn sanitize(mut s: Settings) -> Settings {
    s.refresh_minutes = s.refresh_minutes.clamp(1, 60);
    s.display.running_out_threshold = s.display.running_out_threshold.min(100);
    // Older files stored "no taskbar display" as the style Off.
    if s.display.taskbar_mode == crate::settings::TaskbarMode::Off {
        s.display.taskbar_enabled = false;
        s.display.taskbar_mode = crate::settings::TaskbarMode::Badge;
    }
    s
}

fn bad_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".bad");
    let first = path.with_file_name(&name);
    if !first.exists() {
        return first;
    }
    name.push(format!(".{}", Utc::now().format("%Y%m%d%H%M%S%3f")));
    path.with_file_name(name)
}

fn quarantine(path: &Path) -> anyhow::Result<()> {
    fs::rename(path, bad_path(path))
        .context("Settings file is unreadable and could not be backed up")
}

/// Atomic: write `settings.json.tmp`, flush to disk, rename over the target.
pub(crate) fn save(path: &Path, settings: &Settings) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).context("Could not create the settings folder")?;
    }
    let mut tmp_name = path.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(".tmp");
    let tmp = path.with_file_name(tmp_name);
    let json = serde_json::to_vec_pretty(settings).context("Could not serialize settings")?;
    let mut f = fs::File::create(&tmp).context("Could not write settings")?;
    f.write_all(&json).context("Could not write settings")?;
    f.sync_all().context("Could not write settings")?;
    drop(f);
    fs::rename(&tmp, path).context("Could not replace the settings file")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Account, ProviderKind, Reading};
    use crate::settings::TrayMetric;
    use uuid::Uuid;

    fn account() -> Account {
        Account {
            id: Uuid::new_v4(),
            provider: ProviderKind::Claude,
            label: "Work".into(),
            email: Some("a@example.test".into()),
            profile_dir: Some("C:\\fake\\profile".into()),
            hub: None,
            group: Some("Work".into()),
            hidden: false,
            hidden_windows: vec!["x".into()],
            pinned_windows: vec!["five_hour".into()],
        }
    }

    #[test]
    fn missing_file_gives_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let s = load(&dir.path().join("settings.json")).unwrap();
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn roundtrip_creates_dirs_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("settings.json");
        let mut s = Settings::default();
        let a = account();
        s.display.tray_metric = TrayMetric::Window {
            account: a.id,
            key: "five_hour".into(),
        };
        s.readings.insert(
            a.id,
            Reading {
                fetched_at: "2026-11-06T10:00:00Z".parse().unwrap(),
                windows: vec![],
            },
        );
        s.accounts.push(a);
        s.refresh_minutes = 15;
        save(&path, &s).unwrap();
        save(&path, &s).unwrap();
        assert_eq!(load(&path).unwrap(), s);
        let names: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 1, "{names:?}");
    }

    #[test]
    fn corrupt_file_is_kept_as_bad_and_defaults_returned() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, "{ not json").unwrap();
        assert_eq!(load(&path).unwrap(), Settings::default());
        assert!(!path.exists());
        assert_eq!(
            fs::read_to_string(dir.path().join("settings.json.bad")).unwrap(),
            "{ not json"
        );
        // A second corrupt file does not overwrite the first backup.
        fs::write(&path, "again").unwrap();
        load(&path).unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("settings.json.bad")).unwrap(),
            "{ not json"
        );
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn newer_schema_is_quarantined_and_values_are_clamped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, r#"{"schema_version": 99}"#).unwrap();
        assert_eq!(load(&path).unwrap(), Settings::default());
        assert!(dir.path().join("settings.json.bad").exists());
        fs::write(
            &path,
            r#"{"refresh_minutes": 0, "display": {"running_out_threshold": 250}}"#,
        )
        .unwrap();
        let s = load(&path).unwrap();
        assert_eq!(
            (s.refresh_minutes, s.display.running_out_threshold),
            (1, 100)
        );
    }

    #[test]
    fn taskbar_off_style_loads_as_switched_off() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, r#"{"display": {"taskbar_mode": "Off"}}"#).unwrap();
        let d = load(&path).unwrap().display;
        assert!(!d.taskbar_enabled);
        assert_eq!(d.taskbar_mode, crate::settings::TaskbarMode::Badge);
        // Files without the field show the display, in the Badge style.
        fs::write(&path, r#"{"display": {}}"#).unwrap();
        let d = load(&path).unwrap().display;
        assert!(d.taskbar_enabled);
        assert_eq!(d.taskbar_mode, crate::settings::TaskbarMode::Badge);
    }

    #[test]
    fn utf8_bom_is_tolerated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, b"\xEF\xBB\xBF{\"refresh_minutes\": 10}").unwrap();
        assert_eq!(load(&path).unwrap().refresh_minutes, 10);
    }
}
