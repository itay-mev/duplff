use duplff_core::models::ScanConfig;
use duplff_core::progress::NoopProgress;
use duplff_core::{actions, find_duplicates};
use std::fs;
use tempfile::TempDir;

#[test]
fn full_pipeline_finds_exact_duplicates() {
    let dir = TempDir::new().unwrap();

    // Group 1: two identical files
    fs::write(dir.path().join("original.py"), "def hello(): pass").unwrap();
    fs::write(dir.path().join("copy.py"), "def hello(): pass").unwrap();

    // Group 2: three identical files
    let big_content = "x".repeat(10_000);
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/main.rs"), &big_content).unwrap();
    fs::write(dir.path().join("backup.rs"), &big_content).unwrap();
    fs::write(dir.path().join("old.rs"), &big_content).unwrap();

    // Not a duplicate: different content, different size
    fs::write(dir.path().join("unique.txt"), "only one of me here!!!!").unwrap();

    let config = ScanConfig {
        roots: vec![dir.path().to_path_buf()],
        min_size: 1,
        ..ScanConfig::default()
    };

    let report = find_duplicates(&config, &NoopProgress).unwrap();

    assert_eq!(report.groups.len(), 2);
    assert_eq!(report.total_duplicates, 3); // 1 from group1 + 2 from group2
    assert!(report.total_wasted_bytes > 0);
}

#[test]
fn full_pipeline_with_extension_filter() {
    let dir = TempDir::new().unwrap();

    fs::write(dir.path().join("a.py"), "same content").unwrap();
    fs::write(dir.path().join("b.py"), "same content").unwrap();
    fs::write(dir.path().join("c.txt"), "same content").unwrap(); // excluded by filter

    let config = ScanConfig {
        roots: vec![dir.path().to_path_buf()],
        extensions: Some(vec!["py".into()]),
        min_size: 1,
        ..ScanConfig::default()
    };

    let report = find_duplicates(&config, &NoopProgress).unwrap();
    assert_eq!(report.groups.len(), 1);
    assert_eq!(report.groups[0].duplicates.len(), 1);
}

#[test]
fn full_pipeline_with_priority_paths() {
    let dir = TempDir::new().unwrap();

    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();
    fs::write(dir.path().join("copy.rs"), "fn main() {}").unwrap();

    let config = ScanConfig {
        roots: vec![dir.path().to_path_buf()],
        priority_paths: vec![dir.path().join("src")],
        min_size: 1,
        ..ScanConfig::default()
    };

    let report = find_duplicates(&config, &NoopProgress).unwrap();
    assert_eq!(report.groups.len(), 1);
    assert!(report.groups[0]
        .keep
        .entry
        .path
        .starts_with(dir.path().join("src")));
}

#[test]
fn dry_run_does_not_delete_files() {
    let dir = TempDir::new().unwrap();

    fs::write(dir.path().join("a.txt"), "duplicate").unwrap();
    fs::write(dir.path().join("b.txt"), "duplicate").unwrap();

    let config = ScanConfig {
        roots: vec![dir.path().to_path_buf()],
        min_size: 1,
        ..ScanConfig::default()
    };

    let report = find_duplicates(&config, &NoopProgress).unwrap();
    let plan = actions::dry_run(&report.groups);

    assert_eq!(plan.files_to_delete.len(), 1);
    // Both files should still exist
    assert!(dir.path().join("a.txt").exists());
    assert!(dir.path().join("b.txt").exists());
}

#[test]
fn same_size_different_content_not_grouped() {
    let dir = TempDir::new().unwrap();

    // 10 bytes each, different content
    fs::write(dir.path().join("a.txt"), "aaaaaaaaaa").unwrap();
    fs::write(dir.path().join("b.txt"), "bbbbbbbbbb").unwrap();

    let config = ScanConfig {
        roots: vec![dir.path().to_path_buf()],
        min_size: 1,
        ..ScanConfig::default()
    };

    let report = find_duplicates(&config, &NoopProgress).unwrap();
    assert_eq!(report.groups.len(), 0);
}

#[test]
fn overlapping_roots_do_not_make_a_file_its_own_duplicate() {
    let dir = TempDir::new().unwrap();
    fs::create_dir(dir.path().join("photos")).unwrap();
    fs::write(
        dir.path().join("photos/only_copy.jpg"),
        "irreplaceable bytes",
    )
    .unwrap();

    // The nested root makes only_copy.jpg reachable through both walks
    let config = ScanConfig {
        roots: vec![dir.path().to_path_buf(), dir.path().join("photos")],
        min_size: 1,
        ..ScanConfig::default()
    };

    let report = find_duplicates(&config, &NoopProgress).unwrap();
    assert_eq!(report.total_files_scanned, 1);
    assert_eq!(report.groups.len(), 0);
}

#[test]
fn report_serializes_to_json() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.txt"), "dup").unwrap();
    fs::write(dir.path().join("b.txt"), "dup").unwrap();

    let config = ScanConfig {
        roots: vec![dir.path().to_path_buf()],
        min_size: 1,
        ..ScanConfig::default()
    };

    let report = find_duplicates(&config, &NoopProgress).unwrap();
    let json = serde_json::to_string_pretty(&report).unwrap();
    assert!(json.contains("total_files_scanned"));
    assert!(json.contains("total_duplicates"));
}
