use crate::error::{DuplffError, Result};
use crate::models::DuplicateGroup;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Plan produced by a dry run -- describes what would be deleted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionPlan {
    pub files_to_delete: Vec<PathBuf>,
    pub bytes_to_reclaim: u64,
}

/// Record of a single action taken.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionRecord {
    pub path: PathBuf,
    pub action: ActionType,
}

/// The type of action performed on a file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ActionType {
    Trashed,
}

/// Log of all actions taken during a trash operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionLog {
    pub actions: Vec<ActionRecord>,
    pub bytes_reclaimed: u64,
    pub timestamp: String,
}

/// Produce a dry-run plan: which files would be deleted and how much space reclaimed.
pub fn dry_run(groups: &[DuplicateGroup]) -> ActionPlan {
    let mut files_to_delete = Vec::new();
    let mut bytes_to_reclaim = 0u64;

    for group in groups {
        for dup in &group.duplicates {
            // A duplicate entry that aliases the keep path must never be
            // planned for deletion. Deleting it would remove the kept file.
            if dup.entry.path == group.keep.entry.path {
                continue;
            }
            files_to_delete.push(dup.entry.path.clone());
            bytes_to_reclaim += group.size;
        }
    }

    ActionPlan {
        files_to_delete,
        bytes_to_reclaim,
    }
}

/// Move duplicate files to the OS trash. The keep file is never touched.
pub fn trash_duplicates(groups: &[DuplicateGroup]) -> Result<ActionLog> {
    let mut actions = Vec::new();
    let mut bytes_reclaimed = 0u64;

    for group in groups {
        for dup in &group.duplicates {
            // Last line of defense before deletion. Scanning dedupes aliased
            // paths, but trashing an entry equal to the keep path would
            // delete the file we promised to keep.
            if dup.entry.path == group.keep.entry.path {
                continue;
            }
            trash::delete(&dup.entry.path).map_err(|e| {
                DuplffError::TrashError(format!("{}: {}", dup.entry.path.display(), e))
            })?;
            actions.push(ActionRecord {
                path: dup.entry.path.clone(),
                action: ActionType::Trashed,
            });
            bytes_reclaimed += group.size;
        }
    }

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .to_string();

    Ok(ActionLog {
        actions,
        bytes_reclaimed,
        timestamp,
    })
}

/// Result of an undo operation.
#[derive(Debug, Clone)]
pub struct UndoResult {
    pub restored: Vec<PathBuf>,
    pub not_found: Vec<PathBuf>,
}

/// Undo a previous trash operation by restoring files from the OS trash.
pub fn undo(log: &ActionLog) -> Result<UndoResult> {
    let trash_items = trash::os_limited::list()
        .map_err(|e| DuplffError::TrashError(format!("failed to list trash: {e}")))?;

    let mut restored = Vec::new();
    let mut not_found = Vec::new();

    for record in &log.actions {
        // Find matching trash item by original path
        let matching: Vec<_> = trash_items
            .iter()
            .filter(|item| {
                let original = item.original_parent.join(&item.name);
                original == record.path
            })
            .cloned()
            .collect();

        if matching.is_empty() {
            not_found.push(record.path.clone());
        } else {
            // Restore the most recently trashed matching item
            if let Err(e) = trash::os_limited::restore_all(matching.into_iter().take(1)) {
                not_found.push(record.path.clone());
                eprintln!("warning: could not restore {}: {e}", record.path.display());
            } else {
                restored.push(record.path.clone());
            }
        }
    }

    Ok(UndoResult {
        restored,
        not_found,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::*;
    use std::time::SystemTime;

    fn make_group(paths: &[&str]) -> DuplicateGroup {
        let keep = RankedFile {
            entry: FileEntry {
                path: paths[0].into(),
                size: 100,
                modified: SystemTime::UNIX_EPOCH,
            },
            reason: KeepReason::LexicographicFirst,
        };
        let duplicates: Vec<RankedFile> = paths[1..]
            .iter()
            .map(|p| RankedFile {
                entry: FileEntry {
                    path: (*p).into(),
                    size: 100,
                    modified: SystemTime::UNIX_EPOCH,
                },
                reason: KeepReason::LexicographicFirst,
            })
            .collect();
        DuplicateGroup {
            hash: [0u8; 32],
            size: 100,
            keep,
            duplicates,
        }
    }

    #[test]
    fn dry_run_lists_files_to_delete() {
        let groups = vec![make_group(&["/keep.txt", "/delete1.txt", "/delete2.txt"])];
        let plan = dry_run(&groups);
        assert_eq!(plan.files_to_delete.len(), 2);
        assert_eq!(plan.bytes_to_reclaim, 200);
        assert!(plan
            .files_to_delete
            .contains(&std::path::PathBuf::from("/delete1.txt")));
    }

    #[test]
    fn dry_run_never_includes_keep_file() {
        let groups = vec![make_group(&["/keep.txt", "/dup.txt"])];
        let plan = dry_run(&groups);
        assert!(!plan
            .files_to_delete
            .contains(&std::path::PathBuf::from("/keep.txt")));
    }

    #[test]
    fn dry_run_excludes_duplicates_that_alias_the_keep_path() {
        // Simulates an upstream failure where the keep path is also listed
        // as a duplicate. Only the genuinely distinct path may be planned.
        let groups = vec![make_group(&["/keep.txt", "/keep.txt", "/other.txt"])];
        let plan = dry_run(&groups);
        assert_eq!(plan.files_to_delete.len(), 1);
        assert_eq!(
            plan.files_to_delete[0],
            std::path::PathBuf::from("/other.txt")
        );
        assert_eq!(plan.bytes_to_reclaim, 100);
    }

    #[test]
    fn trash_duplicates_refuses_to_trash_the_keep_path() {
        use std::fs;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let only_copy = dir.path().join("only_copy.txt");
        fs::write(&only_copy, "irreplaceable").unwrap();

        // Group where the sole "duplicate" is the keep file itself
        let path_str = only_copy.to_str().unwrap();
        let groups = vec![make_group(&[path_str, path_str])];

        let log = trash_duplicates(&groups).unwrap();
        assert!(only_copy.exists(), "keep file must survive");
        assert!(log.actions.is_empty());
        assert_eq!(log.bytes_reclaimed, 0);
    }

    #[test]
    fn undo_with_no_matching_trash_items() {
        let log = ActionLog {
            actions: vec![ActionRecord {
                path: "/nonexistent/file.txt".into(),
                action: ActionType::Trashed,
            }],
            bytes_reclaimed: 100,
            timestamp: "0".to_string(),
        };
        let result = undo(&log).unwrap();
        assert!(result.restored.is_empty());
        assert_eq!(result.not_found.len(), 1);
    }

    #[test]
    fn trash_duplicates_moves_files() {
        use std::fs;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let keep = dir.path().join("keep.txt");
        let dup = dir.path().join("dup.txt");
        fs::write(&keep, "content").unwrap();
        fs::write(&dup, "content").unwrap();

        let groups = vec![DuplicateGroup {
            hash: [0u8; 32],
            size: 7,
            keep: RankedFile {
                entry: FileEntry {
                    path: keep.clone(),
                    size: 7,
                    modified: SystemTime::UNIX_EPOCH,
                },
                reason: KeepReason::LexicographicFirst,
            },
            duplicates: vec![RankedFile {
                entry: FileEntry {
                    path: dup.clone(),
                    size: 7,
                    modified: SystemTime::UNIX_EPOCH,
                },
                reason: KeepReason::LexicographicFirst,
            }],
        }];

        let log = trash_duplicates(&groups).unwrap();
        assert!(keep.exists(), "keep file should still exist");
        assert!(!dup.exists(), "duplicate should be trashed");
        assert_eq!(log.actions.len(), 1);
    }
}
