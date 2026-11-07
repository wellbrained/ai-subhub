//! Projects: saved local folders (label + directory, optional hidden flag), shared by all
//! accounts, in the user's manual order (port of the C# `ProjectFolder` / `ProjectCatalog`).
//! Removing a project only removes the shortcut, never the folder. Pure list operations; the
//! Launch menu and the Projects settings page use them.
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Longest label kept.
pub const MAX_LABEL: usize = 60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: Uuid,
    pub label: String,
    pub directory: PathBuf,
    /// Hidden projects stay listed (and editable) in Settings but are left out of Launch menus.
    #[serde(default)]
    pub hidden: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectError {
    /// The folder is already saved as a project.
    Duplicate,
    /// No folder given.
    NoFolder,
}

impl ProjectError {
    pub fn message(self) -> &'static str {
        match self {
            ProjectError::Duplicate => "That folder is already saved.",
            ProjectError::NoFolder => "Choose a folder.",
        }
    }
}

/// Comparison key for a folder: canonical when the folder exists (resolves `..`, links and the
/// verbatim `\\?\` prefix), else the text with unified separators; no trailing separator;
/// case-insensitive on Windows.
pub fn folder_key(path: &Path) -> String {
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut s = resolved.to_string_lossy().replace('/', "\\");
    if let Some(rest) = s.strip_prefix("\\\\?\\") {
        s = rest.to_string();
    }
    let trimmed = s.trim_end_matches('\\');
    // Keep a drive root such as "C:\" distinct from "C:".
    let s = if trimmed.ends_with(':') {
        format!("{trimmed}\\")
    } else {
        trimmed.to_string()
    };
    if cfg!(windows) { s.to_lowercase() } else { s }
}

pub fn same_folder(a: &Path, b: &Path) -> bool {
    folder_key(a) == folder_key(b)
}

pub fn contains(list: &[Project], dir: &Path) -> bool {
    let key = folder_key(dir);
    list.iter().any(|p| folder_key(&p.directory) == key)
}

/// A label from the folder name (or the whole path for a drive root).
pub fn default_label(dir: &Path) -> String {
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| dir.to_string_lossy().into_owned());
    clean_label(&name).unwrap_or_else(|| "Project".into())
}

fn clean_label(label: &str) -> Option<String> {
    let l: String = label.trim().chars().take(MAX_LABEL).collect();
    let l = l.trim().to_string();
    (!l.is_empty()).then_some(l)
}

/// Adds a visible project at the end. An empty label becomes the folder's name. The same folder
/// (compared canonically) is never added twice.
pub fn add(list: &mut Vec<Project>, label: &str, dir: &Path) -> Result<Uuid, ProjectError> {
    if dir.as_os_str().is_empty() {
        return Err(ProjectError::NoFolder);
    }
    if contains(list, dir) {
        return Err(ProjectError::Duplicate);
    }
    let id = Uuid::new_v4();
    list.push(Project {
        id,
        label: clean_label(label).unwrap_or_else(|| default_label(dir)),
        directory: dir.to_path_buf(),
        hidden: false,
    });
    Ok(id)
}

/// The projects shown in Launch menus, in the user's order.
pub fn visible(list: &[Project]) -> Vec<&Project> {
    list.iter().filter(|p| !p.hidden).collect()
}

/// Moves one place up or down in the full list (hidden ones included, so the filtered Launch
/// order follows). False at the ends or for an unknown id.
pub fn move_project(list: &mut [Project], id: Uuid, up: bool) -> bool {
    let Some(i) = list.iter().position(|p| p.id == id) else {
        return false;
    };
    let j = if up {
        match i.checked_sub(1) {
            Some(j) => j,
            None => return false,
        }
    } else {
        i + 1
    };
    if j >= list.len() {
        return false;
    }
    list.swap(i, j);
    true
}

pub fn set_hidden(list: &mut [Project], id: Uuid, hidden: bool) -> bool {
    match list.iter_mut().find(|p| p.id == id) {
        Some(p) => {
            p.hidden = hidden;
            true
        }
        None => false,
    }
}

/// Removes the shortcut only; the folder on disk is not touched.
pub fn remove(list: &mut Vec<Project>, id: Uuid) -> bool {
    let before = list.len();
    list.retain(|p| p.id != id);
    list.len() != before
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    fn names(list: &[Project]) -> Vec<&str> {
        list.iter().map(|p| p.label.as_str()).collect()
    }

    fn three() -> Vec<Project> {
        let mut l = vec![];
        for (n, d) in [("A", "C:\\a"), ("B", "C:\\b"), ("C", "C:\\c")] {
            add(&mut l, n, &p(d)).unwrap();
        }
        l
    }

    #[test]
    fn add_uses_folder_name_when_label_is_empty() {
        let mut l = vec![];
        add(&mut l, "  ", &p("C:\\work\\my app")).unwrap();
        add(&mut l, " Mine ", &p("C:\\other")).unwrap();
        assert_eq!(names(&l), ["my app", "Mine"]);
        assert!(l.iter().all(|x| !x.hidden));
        assert_eq!(default_label(&p("C:\\")), "C:\\");
        assert_eq!(add(&mut l, "x", &p("")), Err(ProjectError::NoFolder));
    }

    #[test]
    fn label_is_capped() {
        let mut l = vec![];
        add(&mut l, &"x".repeat(100), &p("C:\\x")).unwrap();
        assert_eq!(l[0].label.chars().count(), MAX_LABEL);
    }

    #[cfg(windows)]
    #[test]
    fn same_folder_is_not_added_twice_ignoring_case_slashes_and_trailing_separator() {
        let mut l = vec![];
        add(&mut l, "A", &p("C:\\Work\\App")).unwrap();
        for dup in [
            "c:\\work\\app",
            "C:/Work/App",
            "C:\\Work\\App\\",
            "C:\\WORK\\APP\\\\",
        ] {
            assert_eq!(
                add(&mut l, "B", &p(dup)),
                Err(ProjectError::Duplicate),
                "{dup}"
            );
        }
        assert_eq!(l.len(), 1);
        assert!(add(&mut l, "B", &p("C:\\Work\\App2")).is_ok());
    }

    #[test]
    fn existing_folders_compare_canonically() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("proj");
        std::fs::create_dir(&sub).unwrap();
        let mut l = vec![];
        add(&mut l, "P", &sub).unwrap();
        // The same folder reached through `..`.
        let roundabout = dir.path().join("proj").join("..").join("proj");
        assert_eq!(add(&mut l, "Q", &roundabout), Err(ProjectError::Duplicate));
        assert!(contains(&l, &sub));
    }

    #[test]
    fn drive_root_key_keeps_its_separator() {
        assert_ne!(folder_key(&p("C:\\")), folder_key(&p("C:\\x")));
        assert_eq!(folder_key(&p("C:\\")), folder_key(&p("C:\\")));
    }

    #[test]
    fn move_reorders_the_full_list_and_stops_at_the_ends() {
        let mut l = three();
        let (a, b, c) = (l[0].id, l[1].id, l[2].id);
        assert!(!move_project(&mut l, a, true));
        assert!(!move_project(&mut l, c, false));
        assert!(!move_project(&mut l, Uuid::new_v4(), true));
        assert!(move_project(&mut l, b, true));
        assert_eq!(names(&l), ["B", "A", "C"]);
        assert!(move_project(&mut l, a, false));
        assert_eq!(names(&l), ["B", "C", "A"]);
    }

    #[test]
    fn hidden_projects_keep_their_place_and_leave_the_launch_list() {
        let mut l = three();
        let b = l[1].id;
        assert!(set_hidden(&mut l, b, true));
        assert_eq!(names(&l), ["A", "B", "C"]);
        let shown: Vec<&str> = visible(&l).iter().map(|p| p.label.as_str()).collect();
        assert_eq!(shown, ["A", "C"]);
        // Reordering the full list changes the filtered one; showing restores the place.
        move_project(&mut l, b, true);
        move_project(&mut l, b, true);
        assert_eq!(names(&l), ["B", "A", "C"]);
        set_hidden(&mut l, b, false);
        let shown: Vec<&str> = visible(&l).iter().map(|p| p.label.as_str()).collect();
        assert_eq!(shown, ["B", "A", "C"]);
        assert!(!set_hidden(&mut l, Uuid::new_v4(), true));
    }

    #[test]
    fn remove_only_drops_the_shortcut() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("keep");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("file.txt"), b"x").unwrap();
        let mut l = vec![];
        let id = add(&mut l, "K", &sub).unwrap();
        assert!(remove(&mut l, id));
        assert!(!remove(&mut l, id));
        assert!(l.is_empty());
        assert!(sub.join("file.txt").is_file());
    }

    #[test]
    fn json_roundtrip_and_missing_hidden_defaults_to_false() {
        let old =
            r#"{"id":"0b8c3b4e-2a3b-4f3e-9a31-111111111111","label":"P","directory":"C:\\proj"}"#;
        let pr: Project = serde_json::from_str(old).unwrap();
        assert!(!pr.hidden);
        let back: Project = serde_json::from_str(&serde_json::to_string(&pr).unwrap()).unwrap();
        assert_eq!(back, pr);
    }
}
