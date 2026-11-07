//! Moving leftovers aside instead of deleting them, and bringing them back.
//!
//! Layout: `<root>/<id>/manifest.json`, the files under `files/<n>`, exported
//! registry entries under `registry/<n>.json`. The manifest is written before
//! the first move and after every item, so a crash never loses track of what
//! was moved.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::scan::{is_protected, is_protected_key, is_under};
use crate::{platform, Error, ItemKind, Leftover, QuarantineBatch, QuarantineItem, Result};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PutReport {
    pub batch: Option<QuarantineBatch>,
    pub failed: Vec<(String, String)>,
}

pub struct Quarantine {
    root: PathBuf,
    /// The elevated helper's own root in a system folder: ids start with
    /// `admin-`, and what was moved is readable by admins only.
    admin: bool,
}

/// Batches in the elevated helper's root, see [`Quarantine::admin`].
pub const ADMIN_PREFIX: &str = "admin-";

/// Batch and trace ids: `[a-z0-9-]`, so they are always one plain path
/// component.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// A short `[a-z0-9-]` form of a name, for ids.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.len() >= 24 {
            break;
        }
    }
    let out = out.trim_end_matches('-');
    if out.is_empty() {
        "item".into()
    } else {
        out.into()
    }
}

/// Writes JSON to a temporary file next to `path` and renames it over, so a
/// crash leaves the old or the new file, never half of one.
pub(crate) fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

fn is_file_kind(kind: ItemKind) -> bool {
    matches!(kind, ItemKind::File | ItemKind::Dir)
}

impl Quarantine {
    pub fn new(root: PathBuf) -> Self {
        Self { root, admin: false }
    }

    /// The quarantine the elevated helper uses. Never the user's own: a
    /// manifest or link there could be changed to send a restore or purge
    /// run with admin rights anywhere.
    pub fn admin() -> Result<Self> {
        let root = platform::admin_quarantine_root();
        if let Some(parent) = root.parent() {
            platform::secure_dir(parent, false)?;
        }
        platform::secure_dir(&root, false)?;
        Ok(Self { root, admin: true })
    }

    /// `files/` and `registry/` of a batch.
    fn content_dir(&self, path: &Path) -> Result<()> {
        if self.admin {
            platform::secure_dir(path, true)
        } else {
            Ok(fs::create_dir_all(path)?)
        }
    }

    fn batch_dir(&self, id: &str) -> Result<PathBuf> {
        if !valid_id(id) {
            return Err(Error::Failed(format!("invalid id {id:?}")));
        }
        Ok(self.root.join(id))
    }

    fn read_manifest(dir: &Path) -> Result<QuarantineBatch> {
        Ok(serde_json::from_slice(&fs::read(
            dir.join("manifest.json"),
        )?)?)
    }

    pub fn put(&self, app_name: &str, items: &[Leftover]) -> Result<PutReport> {
        fs::create_dir_all(&self.root)?;
        let created = crate::now();
        let prefix = if self.admin { ADMIN_PREFIX } else { "" };
        let base = format!("{prefix}{created}-{}", slug(app_name));
        let (id, dir) = (1..)
            .map(|n| {
                if n == 1 {
                    base.clone()
                } else {
                    format!("{base}-{n}")
                }
            })
            .find_map(|id| {
                let dir = self.root.join(&id);
                // create_dir fails on an existing one: two sweeps in one second.
                match fs::create_dir(&dir) {
                    Ok(()) => Some(Ok((id, dir))),
                    Err(e) if e.kind() == ErrorKind::AlreadyExists => None,
                    Err(e) => Some(Err(e)),
                }
            })
            .expect("endless ids")?;
        let mut batch = QuarantineBatch {
            id,
            app_name: app_name.into(),
            created,
            items: Vec::new(),
            needs_admin: self.admin,
        };
        let manifest = dir.join("manifest.json");
        write_json_atomic(&manifest, &batch)?;
        let mut failed = Vec::new();
        for (n, item) in items.iter().enumerate() {
            let stored = match self.put_one(&dir, n, item, &batch) {
                Ok(Some(stored)) => stored,
                Ok(None) => continue,
                Err(e) => {
                    failed.push((item.path.clone(), e.to_string()));
                    continue;
                }
            };
            batch.items.push(QuarantineItem {
                kind: item.kind,
                original: item.path.clone(),
                value: item.value.clone(),
                stored,
                size: item.size,
            });
            write_json_atomic(&manifest, &batch)?;
        }
        if batch.items.is_empty() {
            fs::remove_dir_all(&dir)?;
            return Ok(PutReport {
                batch: None,
                failed,
            });
        }
        Ok(PutReport {
            batch: Some(batch),
            failed,
        })
    }

    /// Moves one item; `None` when it is inside a folder already moved.
    fn put_one(
        &self,
        dir: &Path,
        n: usize,
        item: &Leftover,
        batch: &QuarantineBatch,
    ) -> Result<Option<String>> {
        if is_file_kind(item.kind) {
            let src = Path::new(&item.path);
            if protected(src) {
                return Err(Error::Failed("protected location".into()));
            }
            if batch
                .items
                .iter()
                .any(|i| i.kind == ItemKind::Dir && is_under(&item.path, &i.original))
            {
                return Ok(None);
            }
            // Never move the quarantine into itself.
            if is_under(&self.root.to_string_lossy(), &item.path) || self.root == src {
                return Err(Error::Failed("protected location".into()));
            }
            self.content_dir(&dir.join("files"))?;
            let stored = format!("files/{n}");
            move_path(src, &dir.join(&stored))?;
            Ok(Some(stored))
        } else {
            if item.kind == ItemKind::RegKey && is_protected_key(&item.path) {
                return Err(Error::Failed("protected location".into()));
            }
            self.content_dir(&dir.join("registry"))?;
            let stored = format!("registry/{n}.json");
            platform::quarantine_registry(item, &dir.join(&stored))?;
            Ok(Some(stored))
        }
    }

    pub fn list(&self) -> Result<Vec<QuarantineBatch>> {
        let entries = match fs::read_dir(&self.root) {
            Ok(e) => e,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let mut out: Vec<QuarantineBatch> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_str().is_some_and(valid_id))
            .filter_map(|e| Self::read_manifest(&e.path()).ok())
            .collect();
        out.sort_by(|a, b| (b.created, &b.id).cmp(&(a.created, &a.id)));
        Ok(out)
    }

    pub fn restore(&self, id: &str) -> Result<Vec<(String, String)>> {
        self.restore_where(id, |_| true)
    }

    /// Restores the items `allow` accepts; the others stay and are reported.
    pub fn restore_where(
        &self,
        id: &str,
        allow: impl Fn(&QuarantineItem) -> bool,
    ) -> Result<Vec<(String, String)>> {
        let dir = self.batch_dir(id)?;
        let manifest = dir.join("manifest.json");
        let mut batch = Self::read_manifest(&dir)?;
        let mut failed = Vec::new();
        let mut kept = Vec::new();
        for item in std::mem::take(&mut batch.items) {
            let result = if self.linked(id, &item) {
                Err(Error::Failed("the batch folder contains a link".into()))
            } else if allow(&item) {
                restore_one(&dir, &item)
            } else {
                Err(Error::Failed("not restored with admin rights".into()))
            };
            match result {
                Ok(()) => {}
                Err(e) => {
                    failed.push((item.original.clone(), e.to_string()));
                    kept.push(item);
                }
            }
            // Keep the manifest in step: restored items are gone from it.
            let mut snapshot = batch.clone();
            snapshot.items = kept.clone();
            write_json_atomic(&manifest, &snapshot)?;
        }
        batch.items = kept;
        if batch.items.is_empty() {
            fs::remove_dir_all(&dir)?;
        } else {
            write_json_atomic(&manifest, &batch)?;
        }
        Ok(failed)
    }

    /// Does a link inside the batch (its folder, `files/`, `registry/`) lead
    /// the item's source elsewhere?
    fn linked(&self, id: &str, item: &QuarantineItem) -> bool {
        let sub = if is_file_kind(item.kind) {
            "files"
        } else {
            "registry"
        };
        let real = |p: &Path| fs::canonicalize(p).ok();
        let inside = real(&self.root).map(|r| r.join(id).join(sub));
        inside.is_none() || real(&self.root.join(id).join(sub)) != inside
    }

    pub fn purge(&self, id: &str) -> Result<()> {
        let dir = self.batch_dir(id)?;
        match fs::remove_dir_all(&dir) {
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
            r => Ok(r?),
        }
    }

    pub fn purge_older_than(&self, days: u64) -> Result<usize> {
        let cutoff = crate::now().saturating_sub(days.saturating_mul(86_400));
        let mut n = 0;
        for batch in self.list()? {
            if batch.created < cutoff {
                self.purge(&batch.id)?;
                n += 1;
            }
        }
        Ok(n)
    }
}

/// `stored` comes from a manifest on disk: only our own two shapes.
fn valid_stored(stored: &str) -> bool {
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    stored.strip_prefix("files/").is_some_and(digits)
        || stored
            .strip_prefix("registry/")
            .and_then(|s| s.strip_suffix(".json"))
            .is_some_and(digits)
}

fn restore_one(dir: &Path, item: &QuarantineItem) -> Result<()> {
    if !valid_stored(&item.stored) {
        return Err(Error::Failed("invalid manifest entry".into()));
    }
    let src = dir.join(&item.stored);
    if !is_file_kind(item.kind) {
        if item.kind == ItemKind::RegKey && is_protected_key(&item.original) {
            return Err(Error::Failed("protected location".into()));
        }
        platform::restore_registry(item, &src)?;
        let _ = fs::remove_file(&src);
        return Ok(());
    }
    let dest = Path::new(&item.original);
    if protected(dest) {
        return Err(Error::Failed("protected location".into()));
    }
    if dest.symlink_metadata().is_ok() {
        return Err(Error::Failed("something else is there now".into()));
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    move_path(&src, dest)
}

/// [`is_protected`] for the path as it really is. Judging the real path
/// alone matters on Windows: `%TEMP%` and old install paths often carry
/// short names (`RUNNER~1`) that only resolve to the long spelling here.
pub fn protected(path: &Path) -> bool {
    !path.is_absolute() || real_path(path).is_none_or(|r| is_protected(&r))
}

/// `path` with links in its parent folders resolved, so a symlink or
/// junction on the way cannot lead around [`is_protected`]. The last
/// component stays: moving a link moves the link, not its target.
pub fn real_path(path: &Path) -> Option<PathBuf> {
    let mut rest = vec![path.file_name()?];
    let mut base = path.parent()?;
    loop {
        if let Ok(real) = fs::canonicalize(base) {
            let mut out = plain_path(real);
            out.extend(rest.iter().rev());
            return Some(out);
        }
        rest.push(base.file_name()?);
        base = base.parent()?;
    }
}

/// Back to the spelling the protected lists use: `C:\` instead of `\\?\C:\`
/// (UNC stays UNC, which is protected), `/var` instead of macOS's
/// `/private/var`.
fn plain_path(real: PathBuf) -> PathBuf {
    let s = real.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        if !rest.starts_with(r"UNC\") {
            return PathBuf::from(rest);
        }
    }
    if let Some(rest) = s.strip_prefix("/private/") {
        let top = rest.split('/').next().unwrap_or("");
        if matches!(top, "var" | "etc" | "tmp") {
            return PathBuf::from(format!("/{rest}"));
        }
    }
    real
}

/// Rename, or copy and delete when the target is on another volume.
fn move_path(src: &Path, dest: &Path) -> Result<()> {
    match fs::rename(src, dest) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::CrossesDevices => {
            if let Err(e) = copy_recursive(src, dest) {
                // The source is untouched; drop the partial copy.
                let _ = remove_any(dest);
                return Err(e.into());
            }
            // The copy is complete: on a failed delete keep it (no data loss)
            // and say so.
            remove_any(src).map_err(|e| {
                Error::Failed(format!(
                    "copied, but the original could not be removed fully: {e}"
                ))
            })
        }
        Err(e) => Err(e.into()),
    }
}

fn remove_any(path: &Path) -> std::io::Result<()> {
    if path.symlink_metadata()?.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

fn copy_recursive(src: &Path, dest: &Path) -> std::io::Result<()> {
    let meta = src.symlink_metadata()?;
    if meta.file_type().is_symlink() {
        #[cfg(unix)]
        return std::os::unix::fs::symlink(fs::read_link(src)?, dest);
        #[cfg(not(unix))]
        return Err(std::io::Error::other("cannot copy a link to another drive"));
    }
    if meta.is_dir() {
        fs::create_dir(dest)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            copy_recursive(&entry.path(), &dest.join(entry.file_name()))?;
        }
        fs::set_permissions(dest, meta.permissions())?;
        Ok(())
    } else {
        fs::copy(src, dest).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Confidence;

    fn lo(path: &Path, kind: ItemKind) -> Leftover {
        Leftover {
            kind,
            path: path.to_string_lossy().into_owned(),
            value: None,
            size: Some(1),
            confidence: Confidence::High,
            reason: "x".into(),
            needs_admin: false,
        }
    }

    #[test]
    fn put_list_restore() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        fs::create_dir_all(data.join("App/sub")).unwrap();
        fs::write(data.join("App/sub/f"), "hello").unwrap();
        fs::write(data.join("app.conf"), "c").unwrap();
        let q = Quarantine::new(tmp.path().join("quarantine"));
        let items = [
            lo(&data.join("App"), ItemKind::Dir),
            lo(&data.join("App/sub/f"), ItemKind::File), // inside the folder: skipped, not failed
            lo(&data.join("app.conf"), ItemKind::File),
            lo(&data.join("missing"), ItemKind::File),
            lo(Path::new("/usr"), ItemKind::Dir),
        ];
        let report = q.put("Some App!", &items).unwrap();
        let batch = report.batch.unwrap();
        assert_eq!(batch.items.len(), 2);
        assert_eq!(report.failed.len(), 2, "{:?}", report.failed);
        assert!(
            valid_id(&batch.id) && batch.id.ends_with("-some-app"),
            "{}",
            batch.id
        );
        assert!(!data.join("App").exists() && !data.join("app.conf").exists());
        assert_eq!(q.list().unwrap(), vec![batch.clone()]);

        // A new file in the old place blocks only that item.
        fs::write(data.join("app.conf"), "new").unwrap();
        let failed = q.restore(&batch.id).unwrap();
        assert_eq!(failed.len(), 1);
        assert_eq!(fs::read_to_string(data.join("App/sub/f")).unwrap(), "hello");
        assert_eq!(fs::read_to_string(data.join("app.conf")).unwrap(), "new");
        assert_eq!(q.list().unwrap()[0].items.len(), 1);

        fs::remove_file(data.join("app.conf")).unwrap();
        assert!(q.restore(&batch.id).unwrap().is_empty());
        assert_eq!(fs::read_to_string(data.join("app.conf")).unwrap(), "c");
        assert!(q.list().unwrap().is_empty(), "empty batch is removed");
    }

    #[test]
    fn nothing_moved_means_no_batch() {
        let tmp = tempfile::tempdir().unwrap();
        let q = Quarantine::new(tmp.path().join("quarantine"));
        let report = q
            .put("x", &[lo(&tmp.path().join("nope"), ItemKind::File)])
            .unwrap();
        assert!(report.batch.is_none());
        assert!(q.list().unwrap().is_empty());
    }

    #[test]
    fn purge_and_broken_manifests() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("quarantine");
        let q = Quarantine::new(root.clone());
        for (i, age) in [0u64, 40 * 86_400].iter().enumerate() {
            let f = tmp.path().join(format!("f{i}"));
            fs::write(&f, "x").unwrap();
            let b = q
                .put("app", &[lo(&f, ItemKind::File)])
                .unwrap()
                .batch
                .unwrap();
            let mut m = Quarantine::read_manifest(&root.join(&b.id)).unwrap();
            m.created -= age;
            write_json_atomic(&root.join(&b.id).join("manifest.json"), &m).unwrap();
        }
        fs::create_dir(root.join("broken")).unwrap();
        fs::write(root.join("broken/manifest.json"), "{").unwrap();
        let list = q.list().unwrap();
        assert_eq!(list.len(), 2);
        assert!(list[0].created > list[1].created, "newest first");
        assert_eq!(q.purge_older_than(30).unwrap(), 1);
        assert_eq!(q.list().unwrap().len(), 1);
        assert!(q.purge("../x").is_err());
        q.purge(&list[0].id).unwrap();
        assert!(q.list().unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn links_cannot_lead_around_protection() {
        let tmp = tempfile::tempdir().unwrap();
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink("/usr/share", &link).unwrap();
        // Spelled harmless, really /usr/share/applications.
        let sneaky = link.join("applications");
        assert!(!is_protected(&sneaky) && protected(&sneaky));
        let q = Quarantine::new(tmp.path().join("quarantine"));
        let report = q.put("x", &[lo(&sneaky, ItemKind::Dir)]).unwrap();
        assert!(report.batch.is_none() && report.failed.len() == 1);
        // A link itself is moved as a link.
        assert!(!protected(&link));
        assert_eq!(plain_path("/private/var/x".into()), Path::new("/var/x"));
        assert_eq!(plain_path(r"\\?\C:\x".into()), Path::new(r"C:\x"));
    }

    #[test]
    fn restore_where_keeps_refused_items() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("f");
        fs::write(&f, "x").unwrap();
        let q = Quarantine::new(tmp.path().join("quarantine"));
        let b = q
            .put("app", &[lo(&f, ItemKind::File)])
            .unwrap()
            .batch
            .unwrap();
        assert_eq!(q.restore_where(&b.id, |_| false).unwrap().len(), 1);
        assert!(!f.exists() && q.list().unwrap()[0].items.len() == 1);
        assert!(q.restore(&b.id).unwrap().is_empty() && f.exists());
    }

    #[cfg(unix)]
    #[test]
    fn restore_refuses_links_inside_the_batch() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("f");
        fs::write(&f, "x").unwrap();
        let q = Quarantine::new(tmp.path().join("quarantine"));
        let b = q
            .put("app", &[lo(&f, ItemKind::File)])
            .unwrap()
            .batch
            .unwrap();
        let files = tmp.path().join("quarantine").join(&b.id).join("files");
        let elsewhere = tmp.path().join("elsewhere");
        fs::rename(&files, &elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, &files).unwrap();
        assert_eq!(q.restore(&b.id).unwrap().len(), 1);
        assert!(!f.exists() && elsewhere.join("0").exists());
    }

    #[test]
    fn rejects_tampered_manifest() {
        assert!(valid_stored("files/3") && valid_stored("registry/2.json"));
        assert!(!valid_stored("files/../x") && !valid_stored("/etc/x") && !valid_stored("files/"));
        assert_eq!(slug("7-Zip 23.01 (x64)"), "7-zip-23-01-x64");
        assert_eq!(slug("…"), "item");
    }
}
