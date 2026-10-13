pub mod catalog;
pub mod demands;
pub mod fs;
pub mod markdown;
pub mod registry;
pub mod sessions;
pub mod state;
pub mod workspace;

/// Valid records read from a directory, plus warnings for the ones that could not be read.
pub struct Scan<T> {
    pub items: Vec<T>,
    pub warnings: Vec<String>,
}

impl<T> Default for Scan<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            warnings: Vec::new(),
        }
    }
}

#[cfg(test)]
pub mod testing {
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use anyhow::Result;

    use super::{catalog::Catalog, demands, sessions, workspace::Workspace};
    use crate::model::{Agent, DemandId, Priority, RegisteredProject};

    pub fn id(s: &str) -> DemandId {
        s.parse().unwrap()
    }

    /// Asserts that `result` failed with a message containing `expected`.
    pub fn assert_err<T>(result: Result<T>, expected: &str) {
        let error = format!("{:#}", result.err().expect("expected an error"));
        assert!(
            error.contains(expected),
            "expected {expected:?} in {error:?}"
        );
    }

    /// A throwaway workspace, removed on drop. It skips `Workspace::init` so the
    /// user's project registry is never touched.
    pub struct TempWorkspace {
        pub workspace: Workspace,
        root: PathBuf,
    }

    impl TempWorkspace {
        pub fn new() -> Self {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let root = std::env::temp_dir().join(format!(
                "fio-test-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&root); // leftover from a killed run with the same PID
            fs::create_dir_all(root.join(".fio/demands")).unwrap();
            fs::write(root.join(".fio/project.toml"), "name = \"test\"\n").unwrap();
            let workspace = Workspace::open(root.clone()).unwrap();
            Self { workspace, root }
        }

        pub fn with_demands(names: &[&str]) -> Self {
            let temp = Self::new();
            for name in names {
                temp.create_demand(name);
            }
            temp
        }

        pub fn create_demand(&self, name: &str) {
            self.try_create(name, None).unwrap();
        }

        pub fn create_child(&self, name: &str, parent: &str) {
            self.try_create(name, Some(parent)).unwrap();
        }

        /// Like `create_demand`/`create_child`, for tests that expect a failure.
        pub fn try_create(&self, name: &str, parent: Option<&str>) -> Result<()> {
            let parent = parent.map(id);
            let priority = Priority::default();
            demands::create(
                &self.workspace,
                &id(name),
                "Title",
                priority,
                parent.as_ref(),
            )
        }

        pub fn project(&self) -> RegisteredProject {
            RegisteredProject {
                name: "test".into(),
                workspace: self.workspace.root().to_path_buf(),
            }
        }

        pub fn catalog(&self) -> Catalog {
            Catalog::from_projects(vec![self.project()])
        }

        pub fn start_session(&self, demand: &str, native: Option<&str>, objective: Option<&str>) {
            let options = sessions::StartOptions {
                agent: Agent::Claude,
                demand: Some(id(demand)),
                objective: objective.map(str::to_owned),
                session_id: native.map(str::to_owned),
                transcript: None,
            };
            sessions::start(&self.workspace, options).unwrap();
        }
    }

    impl Drop for TempWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}
