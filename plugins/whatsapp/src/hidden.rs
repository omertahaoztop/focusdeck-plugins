use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HiddenError {
    Unreadable,
    Unwritable,
}

#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    hidden: Vec<String>,
}

pub struct Hidden {
    path: PathBuf,
    ids: BTreeSet<String>,
}

impl Hidden {
    // a missing file is an empty list, but an unreadable or foreign one must never be overwritten
    pub fn load(path: &Path) -> Result<Self, HiddenError> {
        let ids = match fs::read_to_string(path) {
            Ok(text) => {
                let file: File =
                    serde_json::from_str(&text).map_err(|_| HiddenError::Unreadable)?;
                if file.version != VERSION {
                    return Err(HiddenError::Unreadable);
                }
                file.hidden.into_iter().collect()
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => BTreeSet::new(),
            Err(_) => return Err(HiddenError::Unreadable),
        };
        Ok(Self {
            path: path.to_path_buf(),
            ids,
        })
    }

    pub fn contains(&self, id: &str) -> bool {
        self.ids.contains(id)
    }

    pub fn insert(&mut self, id: &str) -> bool {
        self.ids.insert(id.to_string())
    }

    pub fn retain<'a>(&mut self, ids: impl IntoIterator<Item = &'a str>) -> bool {
        let keep: BTreeSet<&str> = ids.into_iter().collect();
        let before = self.ids.len();
        self.ids.retain(|id| keep.contains(id.as_str()));
        self.ids.len() != before
    }

    pub fn save(&self) -> Result<(), HiddenError> {
        self.write().map_err(|_| HiddenError::Unwritable)
    }

    fn write(&self) -> io::Result<()> {
        let dir = self.path.parent().unwrap_or(Path::new("."));
        fs::create_dir_all(dir)?;
        let body = serde_json::to_string(&File {
            version: VERSION,
            hidden: self.ids.iter().cloned().collect(),
        })
        .map_err(io::Error::other)?;
        let mut name = self.path.file_name().unwrap_or_default().to_os_string();
        name.push(".tmp");
        let temp = dir.join(name);
        let written = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&temp)?;
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
            file.write_all(body.as_bytes())?;
            file.sync_all()?;
            fs::rename(&temp, &self.path)
        })();
        if written.is_err() {
            let _ = fs::remove_file(&temp);
        }
        written
    }
}
