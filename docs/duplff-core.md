# duplff-core

Core library for duplicate file detection. Used by both the CLI and GUI.

## Pipeline

1. **Scan** — Walk directories using the `ignore` crate (respects `.gitignore`). Filters by extension, min/max size, and exclude patterns.
2. **Group by size** — Files with unique sizes cannot be duplicates and are discarded.
3. **Partial hash** — BLAKE3 hash of the first 4KB. Files with unique partial hashes are discarded.
4. **Full hash** — BLAKE3 hash of the entire file (128KB buffer). Groups files by matching full hash.
5. **Rank** — Within each group, pick the "keep" file using: priority path > deepest path > newest mtime > lexicographic first.
6. **Report** — Return a `DuplicateReport` with groups, stats, and ranked files.

## Key Types

- `ScanConfig` — Roots, extensions, size limits, priority paths, exclude patterns, flags (paranoid, no_cache, follow_symlinks)
- `DuplicateReport` — Groups + aggregate stats (total files, bytes, duplicates, wasted)
- `DuplicateGroup` — Hash, size, one `keep` file, and a list of `duplicates`
- `RankedFile` — A `FileEntry` plus the `KeepReason` explaining why it was ranked

## Features

- **Hash cache**: SQLite-based cache at `~/.cache/duplff/hashes.db` (the OS cache directory, so `%LOCALAPPDATA%` on Windows). Keyed by path + size + mtime. Disabled with `no_cache`.
- **Paranoid mode**: After hash matching, performs byte-by-byte comparison to confirm files are identical.
- **Safe deletion**: Uses the `trash` crate to move files to the OS trash (never permanently deletes).
- **Action logs**: Every trash operation is saved as JSON to `~/.local/share/duplff/logs/`. Supports undo by restoring from trash.
- **Dry run**: Produces an `ActionPlan` listing files that would be deleted and bytes that would be reclaimed.
- **Progress reporting**: `ProgressHandler` trait with callbacks for scan progress, hash progress, and completion.

## Dependencies

blake3, ignore, rayon, trash, thiserror, serde, rusqlite
