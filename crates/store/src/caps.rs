//! Size caps — the one place they are defined (spec: "caps as server constants with typed
//! rejection"). The server enforces them on upload and commit ([`crate::manifest`]); the CLI
//! checks the same constants before uploading so a doomed push fails fast and offline.
//!
//! "MB" here means MiB (1024²), matching the blob route's historical body limit.

/// Largest Markdown blob a Version may have (2 MB).
pub const MARKDOWN_MAX: u64 = 2 * 1024 * 1024;

/// Largest single Asset (or derivative) blob (100 MB).
pub const ASSET_MAX: u64 = 100 * 1024 * 1024;

/// Largest sum of Asset sizes one Version may reference (500 MB). Derivatives are derived and
/// do not count; the Markdown does not count (it has its own cap).
pub const VERSION_MAX: u64 = 500 * 1024 * 1024;

/// Human form for messages: `2 MB`, `100 MB`, `500 MB`, `1.5 MB`, `900 KB`, `12 B`.
pub fn human_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * 1024;
    if bytes >= MB {
        // Round up to a tenth so "just over the cap" never reads as equal to it.
        let tenths = (bytes * 10).div_ceil(MB);
        let (whole, tenth) = (tenths / 10, tenths % 10);
        if tenth == 0 { format!("{whole} MB") } else { format!("{whole}.{tenth} MB") }
    } else if bytes >= KB {
        format!("{} KB", bytes.div_ceil(KB))
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_and_human_sizes() {
        assert_eq!(MARKDOWN_MAX, 2_097_152);
        assert_eq!(ASSET_MAX, 104_857_600);
        assert_eq!(VERSION_MAX, 524_288_000);
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(12), "12 B");
        assert_eq!(human_size(1024), "1 KB");
        assert_eq!(human_size(1025), "2 KB");
        assert_eq!(human_size(57 * 1024), "57 KB");
        assert_eq!(human_size(MARKDOWN_MAX), "2 MB");
        assert_eq!(human_size(MARKDOWN_MAX + 1), "2.1 MB");
        assert_eq!(human_size(MARKDOWN_MAX + 512 * 1024), "2.5 MB");
        assert_eq!(human_size(ASSET_MAX + 1), "100.1 MB");
        assert_eq!(human_size(VERSION_MAX), "500 MB");
        assert_eq!(human_size(6 * ASSET_MAX), "600 MB");
    }
}
