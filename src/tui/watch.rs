use anyhow::Result;

use crate::store::catalog::Catalog;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};

pub(super) struct FileWatch {
    watcher: RecommendedWatcher,
    events: Receiver<notify::Result<notify::Event>>,
    paths: BTreeSet<PathBuf>,
    changed: Option<Instant>,
}
impl FileWatch {
    pub fn new() -> Result<Self> {
        let (send, events) = mpsc::channel();
        Ok(Self {
            watcher: notify::recommended_watcher(move |event| {
                let _ = send.send(event);
            })?,
            events,
            paths: BTreeSet::new(),
            changed: None,
        })
    }
    pub fn sync(&mut self, catalog: &Catalog) -> Vec<String> {
        let config_dir =
            directories::ProjectDirs::from("", "", "fio").map(|dirs| dirs.config_dir().to_owned());
        self.sync_with(catalog, config_dir.as_deref())
    }

    /// Watches every project and, so first-time registration is detected too,
    /// the nearest existing ancestor of `config_dir`.
    pub fn sync_with(&mut self, catalog: &Catalog, config_dir: Option<&Path>) -> Vec<String> {
        let mut paths: BTreeSet<_> = catalog
            .projects
            .iter()
            .flat_map(|project| [project.workspace.clone(), project.workspace.join(".fio")])
            .collect();
        if let Some(parent) = config_dir.and_then(|dir| dir.ancestors().find(|path| path.is_dir()))
        {
            paths.insert(parent.to_owned());
        }
        let mut errors = Vec::new();
        for path in self.paths.difference(&paths) {
            let _ = self.watcher.unwatch(path);
        }
        self.paths.retain(|path| paths.contains(path));
        for path in paths.difference(&self.paths).cloned().collect::<Vec<_>>() {
            match self.watcher.watch(
                &path,
                if path.file_name().is_some_and(|name| name == ".fio") {
                    RecursiveMode::Recursive
                } else {
                    RecursiveMode::NonRecursive
                },
            ) {
                Ok(()) => {
                    self.paths.insert(path);
                }
                Err(error) => errors.push(format!(
                    "Live reload unavailable for {}: {error}",
                    path.display()
                )),
            }
        }
        errors
    }
    pub fn ready(&mut self) -> Result<bool> {
        while let Ok(event) = self.events.try_recv() {
            let event = event?;
            if is_removal(event.kind) {
                let removed: Vec<_> = self
                    .paths
                    .iter()
                    .filter(|watched| {
                        event.paths.iter().any(|path| watched.starts_with(path))
                            && !watched.exists()
                    })
                    .cloned()
                    .collect();
                for path in removed {
                    let _ = self.watcher.unwatch(&path);
                    self.paths.remove(&path);
                }
            }
            if is_relevant(&event) {
                self.changed = Some(Instant::now());
            }
        }
        if self
            .changed
            .is_some_and(|time| time.elapsed() >= Duration::from_millis(200))
        {
            self.changed = None;
            return Ok(true);
        }
        Ok(false)
    }
}

fn is_removal(kind: notify::EventKind) -> bool {
    matches!(
        kind,
        notify::EventKind::Remove(_)
            | notify::EventKind::Modify(notify::event::ModifyKind::Name(_))
    )
}

/// Whether an event can change what fio shows: Markdown or configuration files,
/// directories, and removals or renames. Plain reads and other files are ignored.
fn is_relevant(event: &notify::Event) -> bool {
    !matches!(event.kind, notify::EventKind::Access(_))
        && (is_removal(event.kind)
            || event.paths.iter().any(|path| {
                path.file_name().is_some_and(|name| {
                    name == "config.toml" || name == "project.toml" || name == "state.toml"
                }) || path.extension().is_some_and(|extension| extension == "md")
                    || path.is_dir()
            }))
}
