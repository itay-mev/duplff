// Deduplication logic for duplff-core

use crate::cache::HashCache;
use crate::error::Result;
use crate::hasher;
use crate::models::{FileEntry, HashedFile};
use crate::progress::ProgressHandler;
use rayon::prelude::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Group files by size, discarding groups with only one file.
pub fn group_by_size(files: Vec<FileEntry>) -> Vec<Vec<FileEntry>> {
    let mut size_map: HashMap<u64, Vec<FileEntry>> = HashMap::new();
    for file in files {
        size_map.entry(file.size).or_default().push(file);
    }
    size_map.into_values().filter(|g| g.len() > 1).collect()
}

/// Run the full deduplication pipeline: size grouping -> partial hash -> full hash.
///
/// Returns groups of files that are byte-identical (same size + same full BLAKE3 hash).
/// Each inner Vec has at least 2 files.
pub fn find_duplicate_groups(
    files: Vec<FileEntry>,
    progress: &dyn ProgressHandler,
    cache: Option<&HashCache>,
    paranoid: bool,
) -> Result<Vec<Vec<HashedFile>>> {
    // Stage 1: Group by size
    let size_groups = group_by_size(files);

    // Flatten for hashing
    let candidates: Vec<FileEntry> = size_groups.into_iter().flatten().collect();
    let total = candidates.len();

    // Stage 2: Partial hash
    let hashed_count = AtomicUsize::new(0);
    let partial_results: Vec<Result<(FileEntry, [u8; 32])>> = candidates
        .into_par_iter()
        .map(|entry| {
            let mtime = entry
                .modified
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let hash = if let Some(cached) =
                cache.and_then(|c| c.get_partial(&entry.path, entry.size, mtime))
            {
                cached
            } else {
                let h = hasher::partial_hash(&entry.path)?;
                if let Some(c) = cache {
                    c.put_partial(&entry.path, entry.size, mtime, &h);
                }
                h
            };
            let count = hashed_count.fetch_add(1, Ordering::Relaxed) + 1;
            if count.is_multiple_of(100) || count == total {
                progress.on_hash_progress(count, total);
            }
            Ok((entry, hash))
        })
        .collect();

    // Collect results, warn on errors (file may have been deleted mid-scan)
    let mut partial_map: HashMap<(u64, [u8; 32]), Vec<FileEntry>> = HashMap::new();
    for result in partial_results {
        match result {
            Ok((entry, hash)) => {
                partial_map
                    .entry((entry.size, hash))
                    .or_default()
                    .push(entry);
            }
            Err(e) => {
                eprintln!("warning: skipping file during partial hash: {e}");
            }
        }
    }

    // Drop unique partial hashes
    let partial_groups: Vec<Vec<FileEntry>> =
        partial_map.into_values().filter(|g| g.len() > 1).collect();

    let candidates: Vec<FileEntry> = partial_groups.into_iter().flatten().collect();
    let total = candidates.len();

    // Stage 3: Full hash
    let hashed_count = AtomicUsize::new(0);
    let full_results: Vec<Result<HashedFile>> = candidates
        .into_par_iter()
        .map(|entry| {
            let mtime = entry
                .modified
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let hash = if let Some(cached) =
                cache.and_then(|c| c.get_full(&entry.path, entry.size, mtime))
            {
                cached
            } else {
                let h = hasher::full_hash(&entry.path)?;
                if let Some(c) = cache {
                    c.put_full(&entry.path, entry.size, mtime, &h);
                }
                h
            };
            let count = hashed_count.fetch_add(1, Ordering::Relaxed) + 1;
            if count.is_multiple_of(100) || count == total {
                progress.on_hash_progress(count, total);
            }
            Ok(HashedFile { entry, hash })
        })
        .collect();

    // Group by full hash, warn on errors
    let mut full_map: HashMap<(u64, [u8; 32]), Vec<HashedFile>> = HashMap::new();
    for result in full_results {
        match result {
            Ok(hf) => {
                full_map
                    .entry((hf.entry.size, hf.hash))
                    .or_default()
                    .push(hf);
            }
            Err(e) => {
                eprintln!("warning: skipping file during full hash: {e}");
            }
        }
    }

    // Keep only groups with 2+ files
    let mut groups: Vec<Vec<HashedFile>> = full_map.into_values().filter(|g| g.len() > 1).collect();

    // Paranoid verification: byte-by-byte compare all files in each group
    if paranoid {
        use crate::verify;
        groups.retain(|group| {
            let paths: Vec<&std::path::Path> =
                group.iter().map(|hf| hf.entry.path.as_path()).collect();
            verify::verify_group(&paths).unwrap_or(false)
        });
    }

    Ok(groups)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::FileEntry;
    use crate::progress::NoopProgress;
    use std::fs;
    use std::time::SystemTime;
    use tempfile::TempDir;

    #[test]
    fn group_by_size_filters_unique_sizes() {
        let files = vec![
            FileEntry {
                path: "/a".into(),
                size: 100,
                modified: SystemTime::UNIX_EPOCH,
            },
            FileEntry {
                path: "/b".into(),
                size: 100,
                modified: SystemTime::UNIX_EPOCH,
            },
            FileEntry {
                path: "/c".into(),
                size: 200,
                modified: SystemTime::UNIX_EPOCH,
            }, // unique
        ];
        let groups = group_by_size(files);
        assert_eq!(groups.len(), 1); // only the size=100 group
        assert_eq!(groups[0].len(), 2);
    }

    #[test]
    fn group_by_size_empty_input() {
        let groups = group_by_size(vec![]);
        assert!(groups.is_empty());
    }

    #[test]
    fn group_by_size_all_unique() {
        let files = vec![
            FileEntry {
                path: "/a".into(),
                size: 100,
                modified: SystemTime::UNIX_EPOCH,
            },
            FileEntry {
                path: "/b".into(),
                size: 200,
                modified: SystemTime::UNIX_EPOCH,
            },
        ];
        let groups = group_by_size(files);
        assert!(groups.is_empty());
    }

    #[test]
    fn hash_progress_reports_final_count() {
        use crate::progress::ProgressHandler;
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct Recorder {
            last_done: AtomicUsize,
            last_total: AtomicUsize,
        }
        impl ProgressHandler for Recorder {
            fn on_scan_progress(&self, _: usize) {}
            fn on_hash_progress(&self, done: usize, total: usize) {
                self.last_done.store(done, Ordering::Relaxed);
                self.last_total.store(total, Ordering::Relaxed);
            }
            fn on_complete(&self, _: usize) {}
        }

        let dir = TempDir::new().unwrap();
        // Two duplicates: far fewer than the 100-file reporting interval,
        // so only the completion update can fire.
        fs::write(dir.path().join("a.txt"), "same").unwrap();
        fs::write(dir.path().join("b.txt"), "same").unwrap();
        let files: Vec<FileEntry> = ["a.txt", "b.txt"]
            .iter()
            .map(|name| {
                let path = dir.path().join(name);
                let meta = fs::metadata(&path).unwrap();
                FileEntry {
                    path,
                    size: meta.len(),
                    modified: meta.modified().unwrap(),
                }
            })
            .collect();

        let recorder = Recorder {
            last_done: AtomicUsize::new(0),
            last_total: AtomicUsize::new(0),
        };
        let groups = find_duplicate_groups(files, &recorder, None, false).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(recorder.last_done.load(Ordering::Relaxed), 2);
        assert_eq!(recorder.last_total.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn find_duplicates_end_to_end() {
        let dir = TempDir::new().unwrap();
        // Two identical files
        fs::write(dir.path().join("a.txt"), "duplicate content").unwrap();
        fs::write(dir.path().join("b.txt"), "duplicate content").unwrap();
        // Same size but different content (MUST be exact same byte length!)
        fs::write(dir.path().join("c.txt"), "differing_content").unwrap();
        // Different size — will be filtered by size grouping if unique
        fs::write(dir.path().join("d.txt"), "unique").unwrap();

        let files: Vec<FileEntry> = ["a.txt", "b.txt", "c.txt", "d.txt"]
            .iter()
            .map(|name| {
                let path = dir.path().join(name);
                let meta = fs::metadata(&path).unwrap();
                FileEntry {
                    path,
                    size: meta.len(),
                    modified: meta.modified().unwrap(),
                }
            })
            .collect();

        let groups = find_duplicate_groups(files, &NoopProgress, None, false).unwrap();
        // a.txt and b.txt are duplicates (same content = same hash)
        // c.txt has same size as a.txt and b.txt but different content
        // So there should be exactly 1 group with 2 files
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].len(), 2);
    }
}
