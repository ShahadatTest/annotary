//! Data dir resolution (`--data-dir` > `ANNOTARY_HOME` > `~/.annotary`) and `~` display.

use std::path::{Path, PathBuf};

/// Name of the default data dir under `$HOME`.
pub const DEFAULT_DIR_NAME: &str = ".annotary";

/// `$HOME` (or the OS notion of it).
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from).or_else(std::env::home_dir)
}

/// The data dir: explicit flag, then `ANNOTARY_HOME`, then `~/.annotary`. Always absolute
/// (relative inputs are resolved against the current dir, symlinks left alone) so `server.json`
/// and messages name a path that means the same thing from anywhere.
pub fn data_dir(flag: Option<&Path>, env_home: Option<&str>) -> PathBuf {
    let raw = if let Some(p) = flag {
        p.to_path_buf()
    } else if let Some(h) = env_home.filter(|h| !h.is_empty()) {
        PathBuf::from(h)
    } else {
        home_dir().unwrap_or_else(|| PathBuf::from(".")).join(DEFAULT_DIR_NAME)
    };
    std::path::absolute(&raw).unwrap_or(raw)
}

/// Layout inside the data dir.
pub struct DataDir {
    pub root: PathBuf,
}

impl DataDir {
    pub fn new(root: PathBuf) -> DataDir {
        DataDir { root }
    }
    pub fn config_toml(&self) -> PathBuf {
        self.root.join("config.toml")
    }
    pub fn server_json(&self) -> PathBuf {
        self.root.join("server.json")
    }
    pub fn logs(&self) -> PathBuf {
        self.root.join("logs")
    }
    pub fn store(&self) -> PathBuf {
        self.root.join("store")
    }
    /// Create `root/`, `logs/` and `store/`.
    pub fn ensure(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.root)?;
        std::fs::create_dir_all(self.logs())?;
        std::fs::create_dir_all(self.store())
    }
}

/// Show a path with `$HOME` shortened to `~` (what humans expect in messages).
pub fn display(path: &Path) -> String {
    if let Some(home) = home_dir()
        && let Ok(rest) = path.strip_prefix(&home)
    {
        if rest.as_os_str().is_empty() {
            return "~".to_string();
        }
        return format!("~/{}", rest.display());
    }
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolution_order() {
        let flag = Path::new("/tmp/x");
        assert_eq!(data_dir(Some(flag), Some("/tmp/y")), PathBuf::from("/tmp/x"));
        assert_eq!(data_dir(None, Some("/tmp/y")), PathBuf::from("/tmp/y"));
        assert_eq!(data_dir(None, Some("")), home_dir().unwrap().join(".annotary"));
        assert_eq!(data_dir(None, None), home_dir().unwrap().join(".annotary"));
        assert!(data_dir(Some(Path::new("rel/dir")), None).is_absolute());
        assert!(data_dir(Some(Path::new("rel/dir")), None).ends_with("rel/dir"));
    }

    #[test]
    fn tilde_display() {
        let home = home_dir().unwrap();
        assert_eq!(display(&home.join(".annotary/store")), "~/.annotary/store");
        assert_eq!(display(Path::new("/opt/x")), "/opt/x");
    }
}
