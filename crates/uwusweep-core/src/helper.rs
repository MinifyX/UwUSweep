//! The elevated helper: the same binary, started with admin rights for one
//! job file, whose hash is on its command line.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::quarantine::{valid_id, PutReport, Quarantine};
use crate::{platform, App, AutostartEntry, Error, Leftover, Result};

pub const HELPER_FLAG: &str = "--uwusweep-helper";

// The job file is read once per helper run; boxing App would only change the
// Rust side, not the JSON, and make every caller wrap it.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum Op {
    #[serde(rename_all = "camelCase")]
    Uninstall { app: App, quiet: bool },
    /// Quarantine, Restore and Purge always use [`Quarantine::admin`]: the
    /// job names no folder, so it cannot point the helper at one the user
    /// can change.
    #[serde(rename_all = "camelCase")]
    Quarantine {
        app_name: String,
        items: Vec<Leftover>,
    },
    #[serde(rename_all = "camelCase")]
    Restore { id: String },
    #[serde(rename_all = "camelCase")]
    Purge { id: String },
    #[serde(rename_all = "camelCase")]
    Autostart {
        entry: AutostartEntry,
        enabled: bool,
    },
    #[serde(rename_all = "camelCase")]
    RestorePoint { description: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum Outcome {
    #[serde(rename_all = "camelCase")]
    Ok {
        report: Option<PutReport>,
        failed: Vec<(String, String)>,
    },
    #[serde(rename_all = "camelCase")]
    Err { message: String },
}

fn check_id(id: &str) -> Result<()> {
    if valid_id(id) {
        Ok(())
    } else {
        Err(Error::Failed(format!("invalid id {id:?}")))
    }
}

fn ok(report: Option<PutReport>, failed: Vec<(String, String)>) -> Outcome {
    Outcome::Ok { report, failed }
}

fn try_op(op: &Op, quarantine: impl Fn() -> Result<Quarantine>) -> Result<Outcome> {
    Ok(match op {
        Op::Uninstall { app, quiet } => {
            platform::uninstall(app, *quiet)?;
            ok(None, vec![])
        }
        // Quarantine::put itself refuses protected paths and keys.
        Op::Quarantine { app_name, items } => {
            let report = quarantine()?.put(app_name, items)?;
            let failed = report.failed.clone();
            ok(Some(report), failed)
        }
        Op::Restore { id } => {
            check_id(id)?;
            ok(None, quarantine()?.restore(id)?)
        }
        Op::Purge { id } => {
            check_id(id)?;
            quarantine()?.purge(id)?;
            ok(None, vec![])
        }
        // Only the id is taken from the job: the place and file come from the
        // system's own list, so a job cannot name any file to rewrite.
        Op::Autostart { entry, enabled } => {
            let entry = platform::autostart_list()?
                .into_iter()
                .find(|e| e.id == entry.id)
                .ok_or_else(|| Error::Failed("autostart entry not found".into()))?;
            platform::autostart_set(&entry, *enabled)?;
            ok(None, vec![])
        }
        Op::RestorePoint { description } => {
            platform::create_restore_point(description)?;
            ok(None, vec![])
        }
    })
}

/// In-process, without elevation.
pub fn run_op(op: &Op) -> Outcome {
    try_op(op, Quarantine::admin).unwrap_or_else(|e| Outcome::Err {
        message: e.to_string(),
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn result_path(job: &Path) -> PathBuf {
    let mut p = job.as_os_str().to_owned();
    p.push(".result.json");
    PathBuf::from(p)
}

/// Writes the job, runs the helper elevated, reads its outcomes and removes
/// both files again.
pub fn run_as_admin(exe: &Path, ops: &[Op], dir: &Path) -> Result<Vec<Outcome>> {
    fs::create_dir_all(dir)?;
    let bytes = serde_json::to_vec(ops)?;
    let sha = sha256_hex(&bytes);
    // Not a secret, only unique: the hash on the command line protects it.
    let nonce = sha256_hex(
        format!(
            "{:?}{}{:p}",
            std::time::SystemTime::now(),
            std::process::id(),
            &bytes
        )
        .as_bytes(),
    );
    let job = dir.join(format!("job-{}.json", &nonce[..16]));
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&job)?;
    f.write_all(&bytes)?;
    drop(f);
    let result = result_path(&job);
    let args = [
        HELPER_FLAG.to_string(),
        job.to_string_lossy().into_owned(),
        sha,
    ];
    let code = platform::run_elevated(exe, &args);
    let outcomes = code.and_then(|code| {
        let data = fs::read(&result)
            .map_err(|_| Error::Failed(format!("the helper exited with code {code}")))?;
        Ok(serde_json::from_slice::<Vec<Outcome>>(&data)?)
    });
    let _ = fs::remove_file(&job);
    let _ = fs::remove_file(&result);
    outcomes
}

/// Entry point of the elevated helper: `args` is the full argv,
/// `[exe, HELPER_FLAG, job, sha256]`. 0 when every op succeeded, 1 when one
/// failed, 2 when the job was refused and nothing ran.
pub fn helper_main(args: &[String]) -> i32 {
    let [_, flag, job, sha] = args else { return 2 };
    if flag != HELPER_FLAG {
        return 2;
    }
    let job = Path::new(job);
    let Ok(bytes) = fs::read(job) else { return 2 };
    // The file may have been swapped after it was written; the command line
    // could not.
    if !sha256_hex(&bytes).eq_ignore_ascii_case(sha) {
        return 2;
    }
    let Ok(ops) = serde_json::from_slice::<Vec<Op>>(&bytes) else {
        return 2;
    };
    let outcomes: Vec<Outcome> = ops.iter().map(run_op).collect();
    let all_ok = outcomes.iter().all(|o| matches!(o, Outcome::Ok { .. }));
    let result = result_path(job);
    // create_new refuses an existing file or link planted at that name.
    let _ = fs::remove_file(&result);
    let written = serde_json::to_vec(&outcomes).ok().and_then(|data| {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&result)
            .ok()?;
        f.write_all(&data).ok()
    });
    if written.is_some() && all_ok {
        0
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Confidence, ItemKind};

    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("quarantine");
        let file = tmp.path().join("leftover.conf");
        fs::write(&file, "x").unwrap();
        (tmp, root, file)
    }

    fn run_in(root: &Path, op: &Op) -> Outcome {
        try_op(op, || Ok(Quarantine::new(root.to_path_buf()))).unwrap_or_else(|e| Outcome::Err {
            message: e.to_string(),
        })
    }

    fn quarantine_op(file: &Path) -> Op {
        Op::Quarantine {
            app_name: "App".into(),
            items: vec![Leftover {
                kind: ItemKind::File,
                path: file.to_string_lossy().into_owned(),
                value: None,
                size: None,
                confidence: Confidence::High,
                reason: "x".into(),
                needs_admin: false,
            }],
        }
    }

    #[test]
    fn quarantine_and_restore_in_process() {
        let (_tmp, root, file) = setup();
        let Outcome::Ok {
            report: Some(report),
            failed,
        } = run_in(&root, &quarantine_op(&file))
        else {
            panic!()
        };
        assert!(failed.is_empty());
        assert!(!file.exists());
        let id = report.batch.unwrap().id;
        assert_eq!(
            run_in(&root, &Op::Restore { id }),
            Outcome::Ok {
                report: None,
                failed: vec![]
            }
        );
        assert!(file.exists());
    }

    #[test]
    fn job_names_no_folder_and_ids_are_checked() {
        let (tmp, root, file) = setup();
        // An old-style job with its own root is not a valid job any more.
        let old = format!(
            r#"[{{"op":"purge","root":{:?},"id":"x"}}]"#,
            tmp.path().join("quarantine")
        );
        assert!(serde_json::from_str::<Vec<Op>>(&old)
            .is_ok_and(|ops| ops == [Op::Purge { id: "x".into() }]));
        assert!(matches!(
            run_in(&root, &Op::Purge { id: "../..".into() }),
            Outcome::Err { .. }
        ));
        assert!(file.exists());
    }

    #[test]
    fn helper_checks_the_hash() {
        let tmp = tempfile::tempdir().unwrap();
        // Refused before any quarantine is opened, so no system folder is
        // touched; enough to see the job ran and its outcome came back.
        let bytes = serde_json::to_vec(&[Op::Purge { id: "../..".into() }]).unwrap();
        let job = tmp.path().join("job.json");
        fs::write(&job, &bytes).unwrap();
        let args = |sha: &str| {
            vec![
                "exe".to_string(),
                HELPER_FLAG.into(),
                job.to_string_lossy().into_owned(),
                sha.into(),
            ]
        };
        assert_eq!(helper_main(&args(&sha256_hex(b"something else"))), 2);
        assert!(!result_path(&job).exists(), "nothing ran");
        assert_eq!(helper_main(&args(&sha256_hex(&bytes))), 1);
        let outcomes: Vec<Outcome> =
            serde_json::from_slice(&fs::read(result_path(&job)).unwrap()).unwrap();
        assert!(matches!(&outcomes[..], [Outcome::Err { .. }]));
    }
}
