use humansize::{format_size, BINARY};

/// Format a byte count as a human-readable string (e.g. "14.3 KiB").
pub fn human_bytes(bytes: u64) -> String {
    format_size(bytes, BINARY)
}

/// Truncate a path string to fit within max_width characters, keeping the
/// end visible.
///
/// If the path is longer than max_width, replaces the beginning with "...".
#[allow(dead_code)]
pub fn truncate_path(path: &str, max_width: usize) -> String {
    let char_count = path.chars().count();
    if char_count <= max_width {
        return path.to_string();
    }
    if max_width <= 3 {
        return "...".to_string();
    }
    // Split on a char boundary. Slicing by byte offset would panic on
    // multi-byte characters.
    let skip = char_count - (max_width - 3);
    let tail_start = path
        .char_indices()
        .nth(skip)
        .map(|(i, _)| i)
        .unwrap_or(path.len());
    format!("...{}", &path[tail_start..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_bytes_formats_correctly() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(1024), "1 KiB");
        assert_eq!(human_bytes(1048576), "1 MiB");
    }

    #[test]
    fn truncate_path_short_path_unchanged() {
        assert_eq!(truncate_path("/a/b.txt", 20), "/a/b.txt");
    }

    #[test]
    fn truncate_path_long_path_truncated() {
        let long = "/very/long/path/to/some/deeply/nested/file.txt";
        let result = truncate_path(long, 20);
        assert!(result.starts_with("..."));
        assert_eq!(result.len(), 20);
    }

    #[test]
    fn truncate_path_tiny_width() {
        assert_eq!(truncate_path("/a/b/c/d.txt", 3), "...");
    }

    #[test]
    fn truncate_path_multibyte_does_not_panic() {
        // Each of these characters is multiple bytes in UTF-8, so a byte
        // based slice offset would land mid-character and panic
        let path = "/tmp/写真フォルダ/レシピ集/夕食.txt";
        let result = truncate_path(path, 10);
        assert!(result.starts_with("..."));
        assert_eq!(result.chars().count(), 10);
    }
}
