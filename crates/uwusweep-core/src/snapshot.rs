//! Install traces: what is there before an install, what is there after.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::quarantine::{valid_id, write_json_atomic};
use crate::scan::{home_dir, is_under, skip_in_root};
use crate::{platform, Confidence, Error, ItemKind, Leftover, Result, ScanLevel, Trace};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub files: BTreeSet<String>,
    pub registry: BTreeSet<String>,
    pub apps: BTreeMap<String, String>,
    pub created: u64,
}

pub fn take() -> Result<Snapshot> {
    let home = home_dir();
    let mut files = BTreeSet::new();
    for root in platform::leftover_roots(ScanLevel::Deep) {
        let mut it = WalkDir::new(&root.path)
            .min_depth(1)
            .max_depth(root.depth)
            .follow_links(false)
            .into_iter();
        while let Some(entry) = it.next() {
            let Ok(entry) = entry else { continue };
            if entry.depth() == 1
                && skip_in_root(
                    &root.path,
                    &entry.file_name().to_string_lossy(),
                    home.as_deref(),
                )
            {
                if entry.file_type().is_dir() {
                    it.skip_current_dir();
                }
                continue;
            }
            files.insert(entry.path().to_string_lossy().into_owned());
        }
    }
    let apps = platform::list_apps()?
        .into_iter()
        .map(|a| (a.id, a.name))
        .collect();
    Ok(Snapshot {
        files,
        registry: platform::registry_snapshot(),
        apps,
        created: crate::now(),
    })
}

pub fn diff(before: &Snapshot, after: &Snapshot) -> (Vec<Leftover>, Vec<String>) {
    diff_with(before, after, |p| match Path::new(p).symlink_metadata() {
        Ok(m) if m.is_dir() => ItemKind::Dir,
        _ => ItemKind::File,
    })
}

/// [`diff`] with the file-or-folder check passed in, for tests.
pub fn diff_with(
    before: &Snapshot,
    after: &Snapshot,
    kind_of: impl Fn(&str) -> ItemKind,
) -> (Vec<Leftover>, Vec<String>) {
    let mut items = Vec::new();
    for (set_before, set_after, registry) in [
        (&before.files, &after.files, false),
        (&before.registry, &after.registry, true),
    ] {
        let new: BTreeSet<&str> = set_after
            .difference(set_before)
            .map(String::as_str)
            .collect();
        for path in &new {
            // Only the topmost new level: a new folder X, not X/a and X/b.
            if has_ancestor_in(path, &new) {
                continue;
            }
            items.push(Leftover {
                kind: if registry {
                    ItemKind::RegKey
                } else {
                    kind_of(path)
                },
                path: path.to_string(),
                value: None,
                size: None,
                confidence: Confidence::High,
                reason: "trace".into(),
                needs_admin: guess_needs_admin(path),
            });
        }
    }
    let apps = after
        .apps
        .keys()
        .filter(|id| !before.apps.contains_key(*id))
        .cloned()
        .collect();
    (items, apps)
}

fn has_ancestor_in(path: &str, set: &BTreeSet<&str>) -> bool {
    let mut p = path;
    while let Some(i) = p.rfind(['/', '\\']) {
        p = &p[..i];
        if set.contains(p) {
            return true;
        }
    }
    false
}

/// The user's own places (home, `HKCU`) need no admin; everything else does.
fn guess_needs_admin(path: &str) -> bool {
    if path.len() > 4
        && path
            .get(..4)
            .is_some_and(|p| p.eq_ignore_ascii_case("HKCU"))
    {
        return false;
    }
    match home_dir() {
        Some(home) => !is_under(path, &home.to_string_lossy()),
        None => true,
    }
}

pub struct Traces {
    root: PathBuf,
}

impl Traces {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn path(&self, id: &str) -> Result<PathBuf> {
        if !valid_id(id) {
            return Err(Error::Failed(format!("invalid id {id:?}")));
        }
        Ok(self.root.join(format!("{id}.json")))
    }

    pub fn save(&self, trace: &Trace) -> Result<()> {
        let path = self.path(&trace.id)?;
        fs::create_dir_all(&self.root)?;
        write_json_atomic(&path, trace)
    }

    /// Newest first; unreadable files are skipped.
    pub fn list(&self) -> Result<Vec<Trace>> {
        let entries = match fs::read_dir(&self.root) {
            Ok(e) => e,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let mut out: Vec<Trace> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .filter_map(|e| serde_json::from_slice::<Trace>(&fs::read(e.path()).ok()?).ok())
            .collect();
        out.sort_by(|a, b| (b.created, &b.id).cmp(&(a.created, &a.id)));
        Ok(out)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        match fs::remove_file(self.path(id)?) {
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
            r => Ok(r?),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(files: &[&str], registry: &[&str], apps: &[&str]) -> Snapshot {
        Snapshot {
            files: files.iter().map(|s| s.to_string()).collect(),
            registry: registry.iter().map(|s| s.to_string()).collect(),
            apps: apps
                .iter()
                .map(|s| (s.to_string(), format!("{s} name")))
                .collect(),
            created: 0,
        }
    }

    #[test]
    fn diff_collapses_to_top_level() {
        let before = snap(&["/r/a", "/r/b"], &[r"HKCU\Software\Old"], &["x"]);
        let after = snap(
            &[
                "/r/a",
                "/r/a/new.txt",
                "/r/b",
                "/r/c",
                "/r/c/1",
                "/r/c/2/3",
                "/r/c-other",
            ],
            &[
                r"HKCU\Software\Old",
                r"HKCU\Software\New",
                r"HKCU\Software\New\Sub",
                r"HKLM\Software\M",
            ],
            &["x", "y"],
        );
        let (items, apps) = diff_with(&before, &after, |p| {
            if p.ends_with(".txt") {
                ItemKind::File
            } else {
                ItemKind::Dir
            }
        });
        let got: Vec<(&str, ItemKind)> = items.iter().map(|l| (l.path.as_str(), l.kind)).collect();
        assert_eq!(
            got,
            [
                ("/r/a/new.txt", ItemKind::File),
                ("/r/c", ItemKind::Dir),
                ("/r/c-other", ItemKind::Dir),
                (r"HKCU\Software\New", ItemKind::RegKey),
                (r"HKLM\Software\M", ItemKind::RegKey),
            ]
        );
        assert!(items
            .iter()
            .all(|l| l.confidence == Confidence::High && l.reason == "trace"));
        assert!(!items[3].needs_admin && items[4].needs_admin);
        assert_eq!(apps, ["y"]);
    }

    #[test]
    fn diff_detects_kind_on_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path().join("d");
        fs::create_dir(&d).unwrap();
        let gone = tmp.path().join("gone").to_string_lossy().into_owned();
        let d = d.to_string_lossy().into_owned();
        let (items, _) = diff(&snap(&[], &[], &[]), &snap(&[&d, &gone], &[], &[]));
        assert_eq!(
            items.iter().map(|l| l.kind).collect::<Vec<_>>(),
            [ItemKind::Dir, ItemKind::File]
        );
    }

    #[test]
    fn traces_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let t = Traces::new(tmp.path().join("traces"));
        assert!(t.list().unwrap().is_empty());
        for (id, created) in [("a-1", 1), ("b-2", 2)] {
            t.save(&Trace {
                id: id.into(),
                name: "n".into(),
                created,
                app_ids: vec![],
                items: vec![],
            })
            .unwrap();
        }
        fs::write(tmp.path().join("traces/bad.json"), "nope").unwrap();
        assert!(t
            .save(&Trace {
                id: "../x".into(),
                name: "n".into(),
                created: 0,
                app_ids: vec![],
                items: vec![]
            })
            .is_err());
        let ids: Vec<String> = t.list().unwrap().into_iter().map(|t| t.id).collect();
        assert_eq!(ids, ["b-2", "a-1"]);
        t.delete("a-1").unwrap();
        t.delete("a-1").unwrap();
        assert_eq!(t.list().unwrap().len(), 1);
    }

    #[test]
    fn needs_admin_guess_takes_any_text() {
        assert!(guess_needs_admin("aÄÖ\\x"));
    }
}
