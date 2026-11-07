//! "Where usage goes": the scan's buckets of a range split by task kind, project, thread and
//! model, each with its main-thread / subagent parts. Shares are weighted by API value (the
//! price book's estimate), so models with different prices compare; when nothing in the range is
//! priced the work tokens weigh instead.
use std::collections::HashMap;
use std::hash::Hash;
use std::path::{Component, Path};

use uuid::Uuid;

use crate::model::ProviderKind;
use crate::projects::Project;
use crate::stats::{ModelPrice, PriceBook, Scan, TaskKind};

/// Where a step ran: a saved project folder, else the working folder's name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ProjectKey {
    Saved(Uuid),
    Folder(String),
    /// The log had no working folder.
    Unknown,
}

/// Normalized for comparing Windows paths: `\\?\` dropped, `/` as `\`, lowercase, no trailing
/// separator.
fn norm(p: &str) -> String {
    let p = p.strip_prefix(r"\\?\").unwrap_or(p);
    let mut s = p.replace('/', "\\").to_lowercase();
    while s.len() > 1 && s.ends_with('\\') {
        s.pop();
    }
    s
}

/// The project of a working folder: the saved project folder that contains it (the deepest when
/// nested), else the folder's own name. Hidden projects count too (they are only left out of
/// Launch menus).
pub fn project_of(cwd: Option<&str>, projects: &[Project]) -> ProjectKey {
    let Some(cwd) = cwd.filter(|c| !c.trim().is_empty()) else {
        return ProjectKey::Unknown;
    };
    let c = norm(cwd);
    let best = projects
        .iter()
        .filter_map(|p| {
            let d = norm(&p.directory.to_string_lossy());
            let inside =
                !d.is_empty() && (c == d || (c.starts_with(&d) && c[d.len()..].starts_with('\\')));
            inside.then_some((d.len(), p.id))
        })
        .max();
    if let Some((_, id)) = best {
        return ProjectKey::Saved(id);
    }
    let name = Path::new(&cwd.replace('\\', "/"))
        .components()
        .rev()
        .find_map(|c| match c {
            Component::Normal(n) => Some(n.to_string_lossy().into_owned()),
            _ => None,
        });
    name.map_or(ProjectKey::Unknown, ProjectKey::Folder)
}

/// One row of a breakdown.
#[derive(Debug, Clone, PartialEq)]
pub struct Share<K> {
    pub key: K,
    pub label: String,
    /// API estimate (USD), all of it and the subagent part.
    pub usd: f64,
    pub sub_usd: f64,
    /// Work tokens, all and the subagent part.
    pub work: u64,
    pub sub_work: u64,
    /// Of the range's total (value, or work tokens when nothing is priced), 0..=1.
    pub share: f64,
}

/// A thread (session) row.
#[derive(Debug, Clone, PartialEq)]
pub struct ThreadShare {
    pub thread: String,
    pub provider: ProviderKind,
    /// From a Claude `custom-title` line; None when the thread has none.
    pub title: Option<String>,
    /// The project most of its value went to.
    pub project: ProjectKey,
    pub usd: f64,
    pub sub_usd: f64,
    pub work: u64,
    pub share: f64,
    /// Last local hour index with use.
    pub last_hour: i64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct UsageReport {
    pub usd: f64,
    pub work: u64,
    pub sub_usd: f64,
    pub sub_work: u64,
    /// Shares weigh by work tokens: nothing in the range is priced.
    pub by_tokens: bool,
    /// Largest first; kinds without use are left out.
    pub kinds: Vec<Share<TaskKind>>,
    pub projects: Vec<Share<ProjectKey>>,
    pub models: Vec<Share<String>>,
    /// The biggest [`MAX_THREADS`] threads.
    pub threads: Vec<ThreadShare>,
}

impl UsageReport {
    /// Share of a task kind in the range (0 when absent).
    pub fn kind_share(&self, kind: TaskKind) -> f64 {
        self.kinds
            .iter()
            .find(|k| k.key == kind)
            .map_or(0.0, |k| k.share)
    }

    /// Subagents' part of the whole (value, or tokens).
    pub fn sub_share(&self) -> f64 {
        if self.by_tokens {
            ratio(self.sub_work as f64, self.work as f64)
        } else {
            ratio(self.sub_usd, self.usd)
        }
    }
}

pub const MAX_THREADS: usize = 20;

/// Per thread: totals, value per project, last hour.
type ThreadAcc = (Acc, HashMap<ProjectKey, f64>, i64);

fn ratio(a: f64, b: f64) -> f64 {
    if b > 0.0 { a / b } else { 0.0 }
}

#[derive(Default, Clone, Copy)]
struct Acc {
    usd: f64,
    sub_usd: f64,
    work: u64,
    sub_work: u64,
}

impl Acc {
    fn add(&mut self, usd: f64, work: u64, sub: bool) {
        self.usd += usd;
        self.work += work;
        if sub {
            self.sub_usd += usd;
            self.sub_work += work;
        }
    }
}

fn rows<K: Clone + Ord + Hash>(
    map: HashMap<K, Acc>,
    total: &Acc,
    by_tokens: bool,
    label: impl Fn(&K) -> String,
) -> Vec<Share<K>> {
    let mut v: Vec<Share<K>> = map
        .into_iter()
        .map(|(key, a)| Share {
            label: label(&key),
            share: if by_tokens {
                ratio(a.work as f64, total.work as f64)
            } else {
                ratio(a.usd, total.usd)
            },
            key,
            usd: a.usd,
            sub_usd: a.sub_usd,
            work: a.work,
            sub_work: a.sub_work,
        })
        .collect();
    v.sort_by(|a, b| {
        b.share
            .total_cmp(&a.share)
            .then(b.work.cmp(&a.work))
            .then(a.key.cmp(&b.key))
    });
    v
}

/// The breakdown of buckets with `lo <= hour <= hi` (local hour indexes).
pub fn usage(
    scan: &Scan,
    lo: i64,
    hi: i64,
    projects: &[Project],
    prices: &PriceBook,
) -> UsageReport {
    let mut price_cache: HashMap<&str, Option<ModelPrice>> = HashMap::new();
    let mut project_cache: HashMap<Option<&str>, ProjectKey> = HashMap::new();
    let mut total = Acc::default();
    let mut kinds: HashMap<TaskKind, Acc> = HashMap::new();
    let mut projs: HashMap<ProjectKey, Acc> = HashMap::new();
    let mut models: HashMap<String, Acc> = HashMap::new();
    let mut threads: HashMap<(ProviderKind, &str), ThreadAcc> = HashMap::new();
    for b in scan.buckets.iter().filter(|b| b.hour >= lo && b.hour <= hi) {
        let price = *price_cache
            .entry(b.model.as_str())
            .or_insert_with(|| prices.price(&b.model));
        let usd = price.map_or(0.0, |p| crate::stats::cost(&b.tokens, &p));
        let work = b.tokens.work();
        let cwd = b.cwd.as_deref();
        let project = project_cache
            .entry(cwd)
            .or_insert_with(|| project_of(cwd, projects))
            .clone();
        total.add(usd, work, b.sidechain);
        kinds.entry(b.kind).or_default().add(usd, work, b.sidechain);
        projs
            .entry(project.clone())
            .or_default()
            .add(usd, work, b.sidechain);
        models
            .entry(b.model.clone())
            .or_default()
            .add(usd, work, b.sidechain);
        let t = threads
            .entry((b.provider, &b.thread))
            .or_insert_with(|| (Acc::default(), HashMap::new(), b.hour));
        t.0.add(usd, work, b.sidechain);
        *t.1.entry(project).or_default() += usd + work as f64 * 1e-12;
        t.2 = t.2.max(b.hour);
    }
    let by_tokens = total.usd <= 0.0;
    let project_label = |k: &ProjectKey| match k {
        ProjectKey::Saved(id) => projects
            .iter()
            .find(|p| p.id == *id)
            .map_or_else(String::new, |p| p.label.clone()),
        ProjectKey::Folder(f) => f.clone(),
        ProjectKey::Unknown => String::new(),
    };
    let mut thread_rows: Vec<ThreadShare> = threads
        .into_iter()
        .map(|((provider, id), (a, projects, last_hour))| ThreadShare {
            thread: id.to_string(),
            provider,
            title: scan.titles.get(id).cloned(),
            project: projects
                .into_iter()
                .max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)))
                .map_or(ProjectKey::Unknown, |(k, _)| k),
            usd: a.usd,
            sub_usd: a.sub_usd,
            work: a.work,
            share: if by_tokens {
                ratio(a.work as f64, total.work as f64)
            } else {
                ratio(a.usd, total.usd)
            },
            last_hour,
        })
        .collect();
    thread_rows.sort_by(|a, b| {
        b.share
            .total_cmp(&a.share)
            .then(b.work.cmp(&a.work))
            .then(a.thread.cmp(&b.thread))
    });
    thread_rows.truncate(MAX_THREADS);
    UsageReport {
        usd: total.usd,
        work: total.work,
        sub_usd: total.sub_usd,
        sub_work: total.sub_work,
        by_tokens,
        kinds: rows(kinds, &total, by_tokens, |k| k.label().to_string()),
        projects: rows(projs, &total, by_tokens, project_label),
        models: rows(models, &total, by_tokens, |m| m.clone()),
        threads: thread_rows,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::{Bucket, Tokens};
    use std::path::PathBuf;
    use std::sync::Arc;

    fn project(id: u128, dir: &str) -> Project {
        Project {
            id: Uuid::from_u128(id),
            label: format!("P{id}"),
            directory: PathBuf::from(dir),
            hidden: id == 3,
        }
    }

    #[test]
    fn working_folders_resolve_to_saved_projects_or_their_name() {
        let ps = [
            project(1, r"D:\Github\app"),
            project(2, r"D:\Github\app\web"),
            project(3, r"C:\Users\x\notes\"),
        ];
        let p = |c: &str| project_of(Some(c), &ps);
        assert_eq!(p(r"D:\Github\app"), ProjectKey::Saved(Uuid::from_u128(1)));
        assert_eq!(
            p(r"d:\github\APP\src"),
            ProjectKey::Saved(Uuid::from_u128(1))
        );
        assert_eq!(
            p("D:/Github/app/web/src"),
            ProjectKey::Saved(Uuid::from_u128(2))
        );
        assert_eq!(
            p(r"\\?\C:\Users\x\notes"),
            ProjectKey::Saved(Uuid::from_u128(3))
        );
        // A sibling with the same prefix is not inside.
        assert_eq!(
            p(r"D:\Github\application"),
            ProjectKey::Folder("application".into())
        );
        assert_eq!(p(r"C:\scratch\thing\"), ProjectKey::Folder("thing".into()));
        assert_eq!(p("/home/me/code/tool"), ProjectKey::Folder("tool".into()));
        assert_eq!(project_of(None, &ps), ProjectKey::Unknown);
        assert_eq!(project_of(Some(" "), &ps), ProjectKey::Unknown);
        assert_eq!(p(r"C:\"), ProjectKey::Unknown);
    }

    fn bucket(
        kind: TaskKind,
        sub: bool,
        cwd: Option<&str>,
        thread: &str,
        model: &str,
        out: u64,
    ) -> Bucket {
        Bucket {
            hour: 100,
            provider: ProviderKind::Claude,
            model: model.into(),
            tokens: Tokens {
                output: out,
                ..Default::default()
            },
            kind,
            sidechain: sub,
            cwd: cwd.map(Arc::from),
            thread: Arc::from(thread),
        }
    }

    #[test]
    fn shares_weigh_by_api_value_with_subagent_parts() {
        let ps = [project(1, r"D:\Github\app")];
        let mut scan = Scan {
            buckets: vec![
                // Opus output: $20 per 1M; Haiku: $0.50 per 1M.
                bucket(
                    TaskKind::WritingCode,
                    false,
                    Some(r"D:\Github\app\src"),
                    "t1",
                    "claude-opus-5-5",
                    1_000_000,
                ),
                bucket(
                    TaskKind::BuildsTests,
                    true,
                    Some(r"D:\Github\app"),
                    "t1",
                    "claude-opus-5-5",
                    1_000_000,
                ),
                bucket(
                    TaskKind::ReadingSearching,
                    false,
                    None,
                    "t2",
                    "claude-haiku-5-5",
                    10_000_000,
                ),
                // Outside the range.
                Bucket {
                    hour: 50,
                    ..bucket(TaskKind::Talking, false, None, "t3", "claude-opus-5-5", 1)
                },
            ],
            ..Default::default()
        };
        scan.titles.insert("t1".into(), "Fix login".into());
        let r = usage(&scan, 90, 110, &ps, &PriceBook::default());
        assert!(!r.by_tokens);
        assert!((r.usd - 45.0).abs() < 1e-9);
        // Haiku has most tokens but the least value.
        assert_eq!(r.kinds.last().unwrap().key, TaskKind::ReadingSearching);
        assert!((r.kind_share(TaskKind::WritingCode) - 20.0 / 45.0).abs() < 1e-9);
        assert_eq!(r.kind_share(TaskKind::Talking), 0.0);
        assert!((r.sub_share() - 20.0 / 45.0).abs() < 1e-9);
        let app = &r.projects[0];
        assert_eq!(
            (app.key.clone(), app.label.as_str()),
            (ProjectKey::Saved(Uuid::from_u128(1)), "P1")
        );
        assert!((app.sub_usd - 20.0).abs() < 1e-9);
        assert_eq!(r.projects[1].key, ProjectKey::Unknown);
        assert_eq!(r.threads[0].title.as_deref(), Some("Fix login"));
        assert_eq!(r.threads[0].project, ProjectKey::Saved(Uuid::from_u128(1)));
        assert_eq!(r.threads[1].title, None);
        assert_eq!(r.models[0].key, "claude-opus-5-5");

        // Nothing priced: shares by work tokens.
        let unpriced = Scan {
            buckets: vec![
                bucket(TaskKind::WritingCode, false, None, "a", "mystery", 30),
                bucket(TaskKind::Talking, false, None, "a", "mystery", 10),
            ],
            ..Default::default()
        };
        let r = usage(&unpriced, 0, 200, &[], &PriceBook::default());
        assert!(r.by_tokens);
        assert!((r.kind_share(TaskKind::WritingCode) - 0.75).abs() < 1e-9);
    }
}
