use anyhow::Result;

use crate::store::catalog::Catalog;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    collections::BTreeSet,
    path::PathBuf,
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
        let mut paths: BTreeSet<_> = catalog
            .projects
            .iter()
            .flat_map(|project| [project.workspace.clone(), project.workspace.join(".fio")])
            .collect();
        if let Some(dirs) = directories::ProjectDirs::from("", "", "fio") {
            // Watch an existing ancestor so first-time registration is also detected.
            if let Some(parent) = dirs.config_dir().ancestors().find(|path| path.is_dir()) {
                paths.insert(parent.to_owned());
            }
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
            if matches!(
                event.kind,
                notify::EventKind::Remove(_)
                    | notify::EventKind::Modify(notify::event::ModifyKind::Name(_))
            ) {
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
            if matches!(event.kind, notify::EventKind::Access(_)) {
                continue;
            }
            if event.paths.iter().any(|path| {
                path.file_name().is_some_and(|name| {
                    name == "config.toml" || name == "project.toml" || name == "state.toml"
                }) || path.extension().is_some_and(|extension| extension == "md")
                    || path.is_dir()
                    || matches!(
                        event.kind,
                        notify::EventKind::Remove(_)
                            | notify::EventKind::Modify(notify::event::ModifyKind::Name(_))
                    )
            }) {
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
