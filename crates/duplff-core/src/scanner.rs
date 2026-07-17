// File scanning for duplff-core

use crate::error::{DuplffError, Result};
use crate::models::{FileEntry, ScanConfig};
use crate::progress::ProgressHandler;
use crossbeam_channel as channel;
use ignore::overrides::OverrideBuilder;
use ignore::WalkBuilder;
use ignore::WalkState;
use rayon::prelude::*;
use std::collections::HashSet;
use std::path::PathBuf;

/// Scan directories according to config, returning matching file entries.
pub fn scan(config: &ScanConfig, progress: &dyn ProgressHandler) -> Result<Vec<FileEntry>> {
    if config.roots.is_empty() {
        return Err(DuplffError::ScanError(
            "no root directories specified".into(),
        ));
    }

    let mut builder = WalkBuilder::new(&config.roots[0]);
    for root in &config.roots[1..] {
        builder.add(root);
    }
    builder
        .follow_links(config.follow_symlinks)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true);

    if !config.exclude_patterns.is_empty() {
        // OverrideBuilder needs a base path for relative patterns; roots[0] is fine
        // because overrides apply to all paths once attached to WalkBuilder.
        let mut overrides = OverrideBuilder::new(&config.roots[0]);
        for pattern in &config.exclude_patterns {
            // Negate the pattern so it's excluded
            overrides
                .add(&format!("!{pattern}"))
                .map_err(|e| DuplffError::ScanError(e.to_string()))?;
        }
        let overrides = overrides
            .build()
            .map_err(|e| DuplffError::ScanError(e.to_string()))?;
        builder.overrides(overrides);
    }

    let min_size = config.min_size;
    let max_size = config.max_size;
    let extensions = config.extensions.clone();

    let (tx, rx) = channel::unbounded();

    builder.build_parallel().run(|| {
        let tx = tx.clone();
        let extensions = extensions.clone();
        Box::new(move |result| {
            let entry = match result {
                Ok(e) => e,
                Err(_) => return WalkState::Continue,
            };

            // Skip non-files
            match entry.file_type() {
                Some(ft) if ft.is_file() => {}
                _ => return WalkState::Continue,
            }

            let metadata = match entry.metadata() {
                Ok(m) => m,
                Err(_) => return WalkState::Continue,
            };
            let size = metadata.len();

            // Size filter
            if size < min_size {
                return WalkState::Continue;
            }
            if let Some(max) = max_size {
                if size > max {
                    return WalkState::Continue;
                }
            }

            // Extension filter
            if let Some(ref exts) = extensions {
                let file_ext = entry.path().extension().and_then(|e| e.to_str());
                match file_ext {
                    Some(ext) if exts.iter().any(|e| e.eq_ignore_ascii_case(ext)) => {}
                    _ => return WalkState::Continue,
                }
            }

            let modified = metadata.modified().unwrap_or(std::time::UNIX_EPOCH);
            let _ = tx.send(FileEntry {
                path: entry.into_path(),
                size,
                modified,
            });

            WalkState::Continue
        })
    });

    // Drop the original sender so rx terminates
    drop(tx);

    let files: Vec<FileEntry> = rx.iter().collect();
    let files = dedupe_aliases(files);
    progress.on_scan_progress(files.len());
    Ok(files)
}

/// Drop entries that alias a file already collected under another entry.
///
/// Overlapping roots (e.g. scanning both `/data` and `/data/photos`) emit the
/// same file once per root, and followed symlinks can reach one file through
/// several paths. Downstream grouping is purely by (size, hash), so an aliased
/// file would be reported as a duplicate of itself and the deletion step could
/// remove the user's only copy. Entries are keyed by canonicalized path,
/// falling back to the path as scanned when canonicalization fails (e.g. the
/// file vanished mid-scan).
fn dedupe_aliases(files: Vec<FileEntry>) -> Vec<FileEntry> {
    let keyed: Vec<(PathBuf, FileEntry)> = files
        .into_par_iter()
        .map(|f| {
            let key = std::fs::canonicalize(&f.path).unwrap_or_else(|_| f.path.clone());
            (key, f)
        })
        .collect();

    let mut seen = HashSet::with_capacity(keyed.len());
    let mut unique = Vec::with_capacity(keyed.len());
    for (key, file) in keyed {
        if seen.insert(key) {
            unique.push(file);
        }
    }
    unique
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::NoopProgress;
    use std::fs;
    use tempfile::TempDir;

    fn make_test_tree() -> TempDir {
        let dir = TempDir::new().unwrap();
        // Create files of varying sizes
        fs::write(dir.path().join("a.txt"), "hello").unwrap(); // 5 bytes
        fs::write(dir.path().join("b.py"), "world!").unwrap(); // 6 bytes
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub/c.rs"), "fn main() {}").unwrap(); // 13 bytes
        fs::write(dir.path().join("sub/d.txt"), "hi").unwrap(); // 2 bytes
                                                                // Empty file — should be skipped with min_size=1
        fs::write(dir.path().join("empty.txt"), "").unwrap();
        dir
    }

    #[test]
    fn scans_all_files_with_no_filters() {
        let dir = make_test_tree();
        let config = ScanConfig {
            roots: vec![dir.path().to_path_buf()],
            min_size: 1,
            ..ScanConfig::default()
        };
        let files = scan(&config, &NoopProgress).unwrap();
        assert_eq!(files.len(), 4); // excludes empty.txt
    }

    #[test]
    fn filters_by_extension() {
        let dir = make_test_tree();
        let config = ScanConfig {
            roots: vec![dir.path().to_path_buf()],
            extensions: Some(vec!["txt".into()]),
            min_size: 1,
            ..ScanConfig::default()
        };
        let files = scan(&config, &NoopProgress).unwrap();
        // a.txt (5b) and sub/d.txt (2b) — both >=1 byte with .txt extension
        assert_eq!(files.len(), 2);
        assert!(files.iter().all(|f| f.path.extension().unwrap() == "txt"));
    }

    #[test]
    fn filters_by_min_size() {
        let dir = make_test_tree();
        let config = ScanConfig {
            roots: vec![dir.path().to_path_buf()],
            min_size: 5,
            ..ScanConfig::default()
        };
        let files = scan(&config, &NoopProgress).unwrap();
        assert!(files.iter().all(|f| f.size >= 5));
    }

    #[test]
    fn excludes_matching_patterns() {
        let dir = make_test_tree();
        let config = ScanConfig {
            roots: vec![dir.path().to_path_buf()],
            min_size: 1,
            exclude_patterns: vec!["sub".to_string()],
            ..ScanConfig::default()
        };
        let files = scan(&config, &NoopProgress).unwrap();
        // sub/c.rs and sub/d.txt should be excluded
        assert!(files.iter().all(|f| !f.path.to_str().unwrap().contains("sub")));
        assert_eq!(files.len(), 2); // a.txt and b.py only
    }

    #[test]
    fn duplicate_roots_do_not_duplicate_files() {
        let dir = make_test_tree();
        let config = ScanConfig {
            roots: vec![dir.path().to_path_buf(), dir.path().to_path_buf()],
            min_size: 1,
            ..ScanConfig::default()
        };
        let files = scan(&config, &NoopProgress).unwrap();
        assert_eq!(files.len(), 4);
    }

    #[test]
    fn overlapping_nested_roots_do_not_duplicate_files() {
        let dir = make_test_tree();
        let config = ScanConfig {
            roots: vec![dir.path().to_path_buf(), dir.path().join("sub")],
            min_size: 1,
            ..ScanConfig::default()
        };
        let files = scan(&config, &NoopProgress).unwrap();
        // sub/c.rs and sub/d.txt are reachable through both roots but must
        // appear only once each
        assert_eq!(files.len(), 4);
    }

    #[test]
    fn returns_correct_metadata() {
        let dir = make_test_tree();
        let config = ScanConfig {
            roots: vec![dir.path().to_path_buf()],
            min_size: 1,
            ..ScanConfig::default()
        };
        let files = scan(&config, &NoopProgress).unwrap();
        let a = files
            .iter()
            .find(|f| f.path.file_name().unwrap() == "a.txt")
            .unwrap();
        assert_eq!(a.size, 5);
    }
}
