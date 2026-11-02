//! `store/meta.json`: a human-readable mirror of the store schema version and Tenant id.
//! SQLite is the truth; this file exists so `ls store/` tells you what you are looking at.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::Result;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    pub schema_version: u32,
    pub tenant_id: Option<String>,
    pub created_at: String,
}

impl Meta {
    pub fn path(store_dir: &Path) -> std::path::PathBuf {
        store_dir.join(crate::META_FILE)
    }

    pub fn read(store_dir: &Path) -> Result<Option<Meta>> {
        let p = Self::path(store_dir);
        if !p.exists() {
            return Ok(None);
        }
        let s = std::fs::read_to_string(p)?;
        Ok(Some(serde_json::from_str(&s)?))
    }

    pub fn write(&self, store_dir: &Path) -> Result<()> {
        let mut s = serde_json::to_string_pretty(self)?;
        s.push('\n');
        std::fs::write(Self::path(store_dir), s)?;
        Ok(())
    }
}
