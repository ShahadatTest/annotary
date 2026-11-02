//! `config.toml` in the data dir: named profiles (`server`, `token`, optional `vault`). The
//! server never writes this file; only `login`/`profile` verbs (and humans) do. There is no
//! default profile: every call names who it is (`-p NAME`, `ANNOTARY_TOKEN` or
//! `ANNOTARY_PROFILE`). A `default = "…"` line from an older CLI is read only to point at the
//! fix in the "no credential" message, and dropped the next time the CLI writes the file.
//!
//! ```toml
//! device = "01K…"           # CMD-23: this install's Device id (see `Config::device`)
//! update_check = false      # no notice of a newer release, and no asking (`crate::update`)
//!
//! [profiles.rowan]
//! server = "http://127.0.0.1:7377"
//! token = "ant_…"
//! vault = "notes"           # optional
//! vault_id = "01K…"         # CMD-5: written by the CLI (the Vault `vault` names, across renames)
//! ```

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::exit::{CliError, CliResult};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// `default = "…"` written by an older CLI: no longer selects a profile; never written.
    #[serde(default, rename = "default", skip_serializing)]
    pub legacy_default: Option<String>,
    /// CMD-23: this CLI install's Device id (26 characters of Crockford base32), one for every
    /// profile and server, as iOS keeps one id across servers. Read and kept, never written yet:
    /// the CLI makes its Device only when a device setting applies to it (none in the starter
    /// set); the first command that reads one will write this line, read-only verbs never do.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    /// `false` turns the update notice off (`crate::update`). Set by hand, kept when the CLI writes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_check: Option<bool>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub profiles: BTreeMap<String, Profile>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault: Option<String>,
    /// CMD-5: the id of the Vault `vault` named when the CLI last heard from the server (written
    /// by the CLI, never by hand): lets the profile follow that Vault through renames. A `vault`
    /// edited by hand to a name this id never carried wins over it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault_id: Option<String>,
}

impl Config {
    /// Load `path`; a missing file is an empty config; a malformed one is a usage error.
    pub fn load(path: &Path) -> CliResult<Config> {
        if !path.exists() {
            return Ok(Config::default());
        }
        let text = std::fs::read_to_string(path)
            .map_err(|e| CliError::usage(format!("cannot read {}: {e}", crate::paths::display(path))))?;
        Config::parse(&text).map_err(|e| CliError::usage(format!("malformed {}: {e}", crate::paths::display(path))))
    }

    pub fn parse(text: &str) -> Result<Config, toml::de::Error> {
        toml::from_str(text)
    }

    pub fn to_toml(&self) -> String {
        toml::to_string(self).expect("config serialises")
    }

    /// Write `path` (creating parent dirs), owner-readable only. Atomic: written to a sibling
    /// temp file and renamed into place, so a concurrent reader never sees a half-written file.
    pub fn save(&self, path: &Path) -> CliResult<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension(format!("toml.{}.tmp", std::process::id()));
        std::fs::write(&tmp, self.to_toml())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    // ---- ticket 06 ----
    /// `a, b, c` or `none` — for messages.
    pub fn profile_names_display(&self) -> String {
        let names = self.names();
        if names.is_empty() { "none".to_string() } else { names.join(", ") }
    }
    // ---- end ticket 06 ----

    /// Profile names in file order (BTreeMap: sorted).
    pub fn names(&self) -> Vec<&str> {
        self.profiles.keys().map(|s| s.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_profiles_and_reads_an_old_default_without_writing_it_back() {
        let c = Config::parse(
            r#"
default = "rowan"
[profiles.rowan]
server = "http://127.0.0.1:7377"
token = "ant_a"
vault = "notes"
[profiles.linter]
token = "ant_b"
"#,
        )
        .unwrap();
        assert_eq!(c.legacy_default.as_deref(), Some("rowan"));
        assert_eq!(c.names(), vec!["linter", "rowan"]);
        assert_eq!(c.profiles["rowan"].vault.as_deref(), Some("notes"));
        assert_eq!(c.profiles["linter"].server, None);
        assert!(!c.to_toml().contains("default"), "{}", c.to_toml());
        let back = Config::parse(&c.to_toml()).unwrap();
        assert_eq!(back, Config { legacy_default: None, ..c });
    }

    // spec: S29-30
    #[test]
    fn keeps_the_device_line_through_a_rewrite() {
        let text = "device = \"01K9ZZ0000000000000000000A\"\n\n[profiles.rowan]\nserver = \"http://127.0.0.1:7377\"\ntoken = \"ant_a\"\n";
        let c = Config::parse(text).unwrap();
        assert_eq!(c.device.as_deref(), Some("01K9ZZ0000000000000000000A"));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        c.save(&path).unwrap();
        let back = Config::load(&path).unwrap();
        assert_eq!(back, c);
        assert!(back.to_toml().starts_with("device = "), "{}", back.to_toml());
        assert_eq!(Config::parse("[profiles.rowan]\n").unwrap().device, None);
        assert!(!Config::default().to_toml().contains("device"));
    }

    #[test]
    fn missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let c = Config::load(&dir.path().join("config.toml")).unwrap();
        assert_eq!(c, Config::default());
    }
}
