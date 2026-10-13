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

#[cfg(test)]
mod tests {
    use std::{fs, thread};

    use notify::{
        Event, EventKind,
        event::{AccessKind, CreateKind, DataChange, ModifyKind, RemoveKind, RenameMode},
    };

    use super::*;
    use crate::{model::RegisteredProject, store::testing::TempWorkspace};

    fn event(kind: EventKind, path: &Path) -> Event {
        Event::new(kind).add_path(path.to_owned())
    }

    fn watching(temp: &TempWorkspace) -> FileWatch {
        let mut watch = FileWatch::new().unwrap();
        let errors = watch.sync_with(&temp.catalog(), None);
        assert!(errors.is_empty(), "{errors:?}");
        watch
    }

    /// Polls until the watcher reports a change, or `timeout` passes.
    fn changed_within(watch: &mut FileWatch, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if watch.ready().unwrap() {
                return true;
            }
            thread::sleep(Duration::from_millis(25));
        }
        false
    }

    #[test]
    fn markdown_and_configuration_changes_are_relevant() {
        let modify = EventKind::Modify(ModifyKind::Data(DataChange::Any));
        for name in ["demand.md", "state.toml", "project.toml", "config.toml"] {
            assert!(is_relevant(&event(modify, Path::new(name))), "{name}");
        }
    }

    #[test]
    fn other_files_and_plain_reads_are_not_relevant() {
        let modify = EventKind::Modify(ModifyKind::Data(DataChange::Any));
        assert!(!is_relevant(&event(modify, Path::new("/nope/notes.txt"))));
        assert!(!is_relevant(&event(modify, Path::new("/nope/write.lock"))));
        let read = EventKind::Access(AccessKind::Read);
        assert!(!is_relevant(&event(read, Path::new("demand.md"))));
    }

    #[test]
    fn removals_renames_and_directories_are_always_relevant() {
        let removed = EventKind::Remove(RemoveKind::File);
        assert!(is_relevant(&event(removed, Path::new("/nope/notes.txt"))));
        let renamed = EventKind::Modify(ModifyKind::Name(RenameMode::Any));
        assert!(is_relevant(&event(renamed, Path::new("/nope/notes.txt"))));
        let created = EventKind::Create(CreateKind::Folder);
        assert!(is_relevant(&event(created, &std::env::temp_dir())));
    }

    #[test]
    fn markdown_edits_in_a_project_trigger_a_single_reload() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let mut watch = watching(&temp);
        let path = temp
            .workspace
            .demand_dir(&"a".parse().unwrap())
            .join("demand.md");
        let source = fs::read_to_string(&path).unwrap();
        fs::write(&path, format!("{source}\nedited\n")).unwrap();
        assert!(changed_within(&mut watch, Duration::from_secs(5)));
        assert!(!watch.ready().unwrap());
    }

    #[test]
    fn an_untouched_project_reports_nothing() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let mut watch = watching(&temp);
        assert!(!changed_within(&mut watch, Duration::from_millis(500)));
    }

    #[test]
    fn sync_watches_the_project_its_dot_fio_and_the_config_ancestor() {
        let temp = TempWorkspace::new();
        let config_root = temp.workspace.root().join("cfg");
        fs::create_dir_all(&config_root).unwrap();
        let mut watch = FileWatch::new().unwrap();
        let config_dir = config_root.join("missing/fio");
        let errors = watch.sync_with(&temp.catalog(), Some(&config_dir));
        assert!(errors.is_empty(), "{errors:?}");
        let root = temp.workspace.root();
        for expected in [root.to_path_buf(), root.join(".fio"), config_root.clone()] {
            assert!(watch.paths.contains(&expected), "{expected:?}");
        }
    }

    #[test]
    fn sync_drops_projects_that_are_no_longer_registered() {
        let temp = TempWorkspace::new();
        let mut watch = watching(&temp);
        assert_eq!(watch.paths.len(), 2);
        let empty = Catalog::from_projects(Vec::new());
        assert_eq!(watch.sync_with(&empty, None), Vec::<String>::new());
        assert!(watch.paths.is_empty());
    }

    #[test]
    fn sync_reports_paths_it_cannot_watch() {
        let mut watch = FileWatch::new().unwrap();
        let missing = Catalog::from_projects(vec![RegisteredProject {
            name: "gone".into(),
            workspace: PathBuf::from("/definitely/not/here"),
        }]);
        let errors = watch.sync_with(&missing, None);
        assert_eq!(errors.len(), 2);
        assert!(errors[0].contains("Live reload unavailable"));
        assert!(watch.paths.is_empty());
    }

    #[test]
    fn a_deleted_dot_fio_is_unwatched_and_reported_as_a_change() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let mut watch = watching(&temp);
        let dot_fio = temp.workspace.root().join(".fio");
        fs::remove_dir_all(&dot_fio).unwrap();
        assert!(changed_within(&mut watch, Duration::from_secs(5)));
        assert!(!watch.paths.contains(&dot_fio));
    }
}
