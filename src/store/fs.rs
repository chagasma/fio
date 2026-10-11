use std::{
    fs::{self, File},
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

pub fn write_atomic(path: &Path, contents: &str) -> Result<()> {
    let parent = path.parent().context("File has no parent directory")?;
    fs::create_dir_all(parent)?;
    let mut name = path.file_name().context("File has no name")?.to_os_string();
    name.push(format!(".{}.tmp", std::process::id()));
    let temporary = parent.join(name);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .with_context(|| format!("Could not create {}", temporary.display()))?;
    let result = (|| -> Result<()> {
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.with_context(|| format!("Could not write {}", path.display()))
}

/// Opens (creating when needed) and exclusively locks a lock file.
pub fn lock(path: &Path) -> Result<File> {
    let file = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .with_context(|| format!("Could not open {}", path.display()))?;
    file.lock()
        .with_context(|| format!("Could not lock {}", path.display()))?;
    Ok(file)
}

/// Sorted directory entries; a missing directory is empty.
pub fn read_dir_sorted(folder: &Path) -> Result<Vec<PathBuf>> {
    let entries = match fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error).with_context(|| format!("Could not list {}", folder.display()));
        }
    };
    let mut paths = entries
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    paths.sort();
    Ok(paths)
}
