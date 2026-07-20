use duplff_core::models::DuplicateReport;
use std::collections::HashSet;
use std::path::PathBuf;

/// Which pane has focus in Results view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusPane {
    Groups,
    Detail,
}

/// Sort mode for the groups pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortMode {
    WastedDesc,
    SizeDesc,
    FileCountDesc,
    PathAsc,
}

impl SortMode {
    pub fn next(self) -> Self {
        match self {
            SortMode::WastedDesc => SortMode::SizeDesc,
            SortMode::SizeDesc => SortMode::FileCountDesc,
            SortMode::FileCountDesc => SortMode::PathAsc,
            SortMode::PathAsc => SortMode::WastedDesc,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SortMode::WastedDesc => "wasted",
            SortMode::SizeDesc => "size",
            SortMode::FileCountDesc => "files",
            SortMode::PathAsc => "path",
        }
    }
}

/// Compute which groups are visible and in what order for the given filter
/// and sort mode. Returns indices into `report.groups` in display order.
///
/// Rendering and input handling must both resolve the group cursor through
/// this function. If either computed display order on its own, the cursor
/// could point at one group on screen while keys acted on another.
pub fn visible_group_indices(
    report: &DuplicateReport,
    filter: &Option<String>,
    sort_mode: SortMode,
) -> Vec<usize> {
    let mut indices: Vec<usize> = (0..report.groups.len()).collect();

    if let Some(f) = filter {
        if !f.is_empty() {
            let f_lower = f.to_lowercase();
            let path_matches = |p: &std::path::Path| {
                p.to_str()
                    .is_some_and(|s| s.to_lowercase().contains(&f_lower))
            };
            indices.retain(|&i| {
                let g = &report.groups[i];
                path_matches(&g.keep.entry.path)
                    || g.duplicates.iter().any(|d| path_matches(&d.entry.path))
            });
        }
    }

    match sort_mode {
        SortMode::WastedDesc => {
            indices.sort_by_key(|&i| std::cmp::Reverse(report.groups[i].wasted_bytes()));
        }
        SortMode::SizeDesc => {
            indices.sort_by_key(|&i| std::cmp::Reverse(report.groups[i].size));
        }
        SortMode::FileCountDesc => {
            indices.sort_by_key(|&i| std::cmp::Reverse(report.groups[i].duplicates.len()));
        }
        SortMode::PathAsc => {
            indices.sort_by(|&a, &b| {
                report.groups[a]
                    .keep
                    .entry
                    .path
                    .cmp(&report.groups[b].keep.entry.path)
            });
        }
    }

    indices
}

/// The application state machine.
pub enum AppState {
    /// Scanning in progress.
    Scanning {
        files_found: usize,
        files_hashed: usize,
        total_to_hash: usize,
        phase: ScanPhase,
    },
    /// Scan complete, showing results.
    Results {
        report: DuplicateReport,
        group_cursor: usize,
        detail_cursor: usize,
        focus: FocusPane,
        marked: HashSet<PathBuf>,
        filter: Option<String>,
        /// True while keystrokes edit the filter text. An applied filter
        /// stays active after editing ends so navigation keys work again.
        filter_editing: bool,
        sort_mode: SortMode,
    },
    /// Confirmation dialog before deletion.
    Confirm {
        report: DuplicateReport,
        group_cursor: usize,
        detail_cursor: usize,
        focus: FocusPane,
        marked: HashSet<PathBuf>,
        filter: Option<String>,
        sort_mode: SortMode,
        message: String,
    },
    /// Help overlay.
    Help {
        report: DuplicateReport,
        group_cursor: usize,
        detail_cursor: usize,
        focus: FocusPane,
        marked: HashSet<PathBuf>,
        filter: Option<String>,
        sort_mode: SortMode,
    },
    /// Fatal error.
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanPhase {
    Scanning,
    Hashing,
}

#[cfg(test)]
mod tests {
    use super::*;
    use duplff_core::models::{DuplicateGroup, FileEntry, KeepReason, RankedFile};
    use std::time::SystemTime;

    fn group(keep: &str, dup: &str, size: u64) -> DuplicateGroup {
        let ranked = |path: &str| RankedFile {
            entry: FileEntry {
                path: path.into(),
                size,
                modified: SystemTime::UNIX_EPOCH,
            },
            reason: KeepReason::LexicographicFirst,
        };
        DuplicateGroup {
            hash: [0u8; 32],
            size,
            keep: ranked(keep),
            duplicates: vec![ranked(dup)],
        }
    }

    fn report(groups: Vec<DuplicateGroup>) -> DuplicateReport {
        DuplicateReport {
            groups,
            total_files_scanned: 0,
            total_bytes_scanned: 0,
            total_duplicates: 0,
            total_wasted_bytes: 0,
        }
    }

    #[test]
    fn visible_indices_sort_by_wasted_bytes() {
        let r = report(vec![
            group("/small/keep", "/small/dup", 10),
            group("/big/keep", "/big/dup", 1000),
        ]);
        let vis = visible_group_indices(&r, &None, SortMode::WastedDesc);
        assert_eq!(vis, vec![1, 0]);
    }

    #[test]
    fn visible_indices_apply_filter() {
        let r = report(vec![
            group("/small/keep", "/small/dup", 10),
            group("/big/keep", "/big/dup", 1000),
        ]);
        let vis = visible_group_indices(&r, &Some("small".into()), SortMode::WastedDesc);
        assert_eq!(vis, vec![0]);
    }

    #[test]
    fn visible_indices_filter_matches_duplicate_paths_too() {
        let r = report(vec![
            group("/x/keep", "/backup/dup", 10),
            group("/y/keep", "/y/dup", 1000),
        ]);
        let vis = visible_group_indices(&r, &Some("backup".into()), SortMode::PathAsc);
        assert_eq!(vis, vec![0]);
    }
}

impl AppState {
    /// Create the initial scanning state.
    pub fn scanning() -> Self {
        AppState::Scanning {
            files_found: 0,
            files_hashed: 0,
            total_to_hash: 0,
            phase: ScanPhase::Scanning,
        }
    }

    /// Transition from scan complete to results view.
    pub fn into_results(report: DuplicateReport) -> Self {
        AppState::Results {
            report,
            group_cursor: 0,
            detail_cursor: 0,
            focus: FocusPane::Groups,
            marked: HashSet::new(),
            filter: None,
            filter_editing: false,
            sort_mode: SortMode::WastedDesc,
        }
    }
}
