use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, ensure};
use chrono::{Local, SecondsFormat};
use serde::Serialize;

use super::{
    Scan, demands, fs as store_fs,
    markdown::{self, Document},
    state::{self, WorkspaceLock},
    workspace::Workspace,
};
use crate::model::{Agent, DemandId, Session, now, section};

/// A session record with its local ID (the file name without extension).
pub struct SessionRecord {
    pub id: String,
    pub path: PathBuf,
    pub document: Document<Session>,
}

impl SessionRecord {
    pub fn metadata(&self) -> &Session {
        &self.document.metadata
    }

    pub fn is_open(&self) -> bool {
        self.metadata().ended.is_none()
    }

    pub fn text(&self, title: &str) -> Result<String> {
        markdown::section_text(&self.document.body, title)
    }

    pub fn save(&self) -> Result<()> {
        store_fs::write_atomic(&self.path, &self.document.render())
    }

    pub fn read(path: PathBuf, demand: &DemandId) -> Result<Self> {
        let source = fs::read_to_string(&path)
            .with_context(|| format!("Could not read {}", path.display()))?;
        let document = Document::<Session>::parse(&source)
            .with_context(|| format!("Invalid session: {}", path.display()))?;
        ensure!(
            &document.metadata.demand == demand,
            "Session belongs to another demand: {}",
            path.display()
        );
        let id = path
            .file_stem()
            .and_then(|name| name.to_str())
            .context("Session filename must be UTF-8")?
            .to_owned();
        Ok(Self { id, path, document })
    }
}

#[derive(Serialize)]
pub struct SessionHandle {
    pub id: String,
    pub demand: DemandId,
    pub path: PathBuf,
}

pub struct StartOptions {
    pub agent: Agent,
    pub demand: Option<DemandId>,
    pub objective: Option<String>,
    pub session_id: Option<String>,
    pub transcript: Option<String>,
}

/// Text edits shared by `update` and `end`.
#[derive(Default)]
pub struct Notes {
    pub summary: Option<String>,
    pub next: Option<String>,
    pub state: Option<String>,
}

fn session_files(demand: &demands::LoadedDemand) -> Result<Vec<PathBuf>> {
    Ok(
        store_fs::read_dir_sorted(&demand.folder().join("sessions"))?
            .into_iter()
            .filter(|path| path.extension().is_some_and(|extension| extension == "md"))
            .collect(),
    )
}

/// Sessions of one demand; any unreadable record is an error.
pub fn read(demand: &demands::LoadedDemand) -> Result<Vec<SessionRecord>> {
    session_files(demand)?
        .into_iter()
        .map(|path| SessionRecord::read(path, demand.id()))
        .collect()
}

/// Sessions of every demand in the workspace; unreadable records become warnings.
pub fn scan(workspace: &Workspace) -> Result<Scan<SessionRecord>> {
    let demands = demands::scan(workspace)?;
    let mut result = Scan {
        items: Vec::new(),
        warnings: demands.warnings,
    };
    for demand in &demands.items {
        for path in session_files(demand)? {
            match SessionRecord::read(path.clone(), demand.id()) {
                Ok(record) => result.items.push(record),
                Err(error) => result
                    .warnings
                    .push(format!("{}: {error:#}", path.display())),
            }
        }
    }
    Ok(result)
}

pub fn start(workspace: &Workspace, options: StartOptions) -> Result<SessionHandle> {
    let _lock = WorkspaceLock::acquire(workspace)?;
    start_unlocked(workspace, options)
}

/// Callers must hold the [`WorkspaceLock`].
pub fn start_unlocked(workspace: &Workspace, options: StartOptions) -> Result<SessionHandle> {
    let id = match options.demand {
        Some(id) => id,
        None => state::active(workspace)?,
    };
    demands::load(workspace, &id)?;
    let started = now();
    let session = Session {
        agent: options.agent,
        session_id: options.session_id,
        transcript: options.transcript,
        demand: id.clone(),
        started,
        ended: None,
        branch: git_branch(workspace.root()),
        fio_summary_revision: None,
        fio_summary_seen: None,
        extra: BTreeMap::new(),
    };
    let body = format!(
        "\n## {}\n\n## {}\n\n## {}\n",
        section::SESSION_OBJECTIVE,
        section::WORK_COMPLETED,
        section::NEXT_STEPS
    );
    let mut document = Document::new(session, body)?;
    if let Some(objective) = options.objective {
        markdown::set_section(&mut document.body, section::SESSION_OBJECTIVE, &objective)?;
    }
    let base = format!(
        "{}-{}-{:08x}-{}",
        started.format("%Y-%m-%d-%H%M"),
        options.agent.name(),
        Local::now().timestamp_subsec_nanos(),
        std::process::id()
    );
    let folder = workspace.demand_dir(&id).join("sessions");
    fs::create_dir_all(&folder)?;
    let (local_id, path) = unused_record(&folder, &base);
    store_fs::write_atomic(&path, &document.render())?;
    Ok(SessionHandle {
        id: local_id,
        demand: id,
        path,
    })
}

/// A record name under `folder` that is not taken yet: `base`, else `base-2`, `base-3`...
/// Callers must hold the [`WorkspaceLock`].
fn unused_record(folder: &Path, base: &str) -> (String, PathBuf) {
    let mut id = base.to_owned();
    let mut attempt = 1;
    while folder.join(format!("{id}.md")).exists() {
        attempt += 1;
        id = format!("{base}-{attempt}");
    }
    let path = folder.join(format!("{id}.md"));
    (id, path)
}

/// Open sessions, optionally restricted to one demand, plus warnings for unreadable records.
pub fn open(workspace: &Workspace, demand: Option<&DemandId>) -> Result<Scan<SessionRecord>> {
    if let Some(id) = demand {
        demands::load(workspace, id)?;
    }
    let mut scanned = scan(workspace)?;
    scanned.items.retain(|record| {
        record.is_open() && demand.is_none_or(|id| &record.metadata().demand == id)
    });
    scanned.items.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(scanned)
}

fn apply_notes(record: &mut SessionRecord, notes: &Notes) -> Result<()> {
    let body = &mut record.document.body;
    if let Some(summary) = &notes.summary {
        markdown::set_section(body, section::WORK_COMPLETED, summary)?;
    }
    if let Some(next) = &notes.next {
        markdown::set_section(body, section::NEXT_STEPS, next)?;
    }
    Ok(())
}

fn update_state(workspace: &Workspace, demand: &DemandId, notes: &Notes) -> Result<()> {
    if let Some(text) = &notes.state {
        demands::load_for_update(workspace, demand)?.set_current_state(text)?;
    }
    Ok(())
}

pub fn update(workspace: &Workspace, id: &str, notes: &Notes) -> Result<PathBuf> {
    let _lock = WorkspaceLock::acquire(workspace)?;
    let mut record = open(workspace, None)?
        .items
        .into_iter()
        .find(|record| record.id == id)
        .context("No matching open session; see fio session list")?;
    apply_notes(&mut record, notes)?;
    if notes.summary.is_some() {
        let revision = format!(
            "{}-{}",
            Local::now().to_rfc3339_opts(SecondsFormat::Nanos, false),
            std::process::id()
        );
        record
            .document
            .set_field("fio_summary_revision", revision)?;
    }
    update_state(workspace, &record.metadata().demand.clone(), notes)?;
    record.save()?;
    Ok(record.path)
}

pub fn end(workspace: &Workspace, id: Option<&str>, notes: &Notes) -> Result<PathBuf> {
    let _lock = WorkspaceLock::acquire(workspace)?;
    let mut candidates = open(workspace, None)?.items;
    if let Some(id) = id {
        candidates.retain(|record| record.id == id);
    }
    ensure!(
        !candidates.is_empty(),
        "No matching open session; see fio session list"
    );
    ensure!(
        candidates.len() == 1,
        "Multiple sessions are open; use fio session end <session-id> (see fio session list)"
    );
    let mut record = candidates.remove(0);
    apply_notes(&mut record, notes)?;
    ensure!(
        !record.text(section::WORK_COMPLETED)?.is_empty(),
        "Provide --summary or fill in 'Work completed' in the Markdown before ending the session"
    );
    record.document.set_field("ended", now())?;
    update_state(workspace, &record.metadata().demand.clone(), notes)?;
    record.save()?;
    Ok(record.path)
}

fn git_branch(workspace: &Path) -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    let directory = if cwd.starts_with(workspace) {
        cwd.as_path()
    } else {
        workspace
    };
    let result = Command::new("git")
        .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
        .current_dir(directory)
        .output()
        .ok()?;
    result
        .status
        .success()
        .then(|| String::from_utf8_lossy(&result.stdout).trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::testing::{TempWorkspace, assert_err, id};

    fn try_start(
        workspace: &Workspace,
        demand: Option<&str>,
        objective: Option<&str>,
    ) -> Result<SessionHandle> {
        start(
            workspace,
            StartOptions {
                agent: Agent::Claude,
                demand: demand.map(id),
                objective: objective.map(str::to_owned),
                session_id: None,
                transcript: None,
            },
        )
    }

    fn start_for(workspace: &Workspace, demand: &str) -> SessionHandle {
        try_start(workspace, Some(demand), None).unwrap()
    }

    fn summary(text: &str) -> Notes {
        Notes {
            summary: Some(text.to_owned()),
            ..Notes::default()
        }
    }

    #[test]
    fn start_creates_an_open_record() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let handle = start_for(&temp.workspace, "a");
        let open = open(&temp.workspace, None).unwrap().items;
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].id, handle.id);
        assert!(open[0].is_open());
    }

    #[test]
    fn start_records_the_objective() {
        let temp = TempWorkspace::with_demands(&["a"]);
        try_start(&temp.workspace, Some("a"), Some("Write tests")).unwrap();
        let record = open(&temp.workspace, None).unwrap().items.remove(0);
        assert_eq!(
            record.text(section::SESSION_OBJECTIVE).unwrap(),
            "Write tests"
        );
    }

    #[test]
    fn start_fails_for_an_unknown_demand() {
        let temp = TempWorkspace::new();
        assert_err(
            try_start(&temp.workspace, Some("ghost"), None),
            "Could not read demand 'ghost'",
        );
    }

    #[test]
    fn start_without_a_demand_needs_an_active_one() {
        let temp = TempWorkspace::with_demands(&["a"]);
        assert_err(try_start(&temp.workspace, None, None), "No active demand");
        state::select(&temp.workspace, &id("a")).unwrap();
        let handle = try_start(&temp.workspace, None, None).unwrap();
        assert_eq!(handle.demand, id("a"));
    }

    #[test]
    fn sub_demand_sessions_live_in_the_sub_demand_folder() {
        let temp = TempWorkspace::with_demands(&["p"]);
        temp.create_child("c", "p");
        let handle = start_for(&temp.workspace, "c");
        assert!(
            handle
                .path
                .starts_with(temp.workspace.demands_dir().join("p/c/sessions"))
        );
        let scanned = scan(&temp.workspace).unwrap();
        assert_eq!(scanned.items.len(), 1);
        assert_eq!(scanned.warnings, Vec::<String>::new());
    }

    #[test]
    fn parent_sessions_do_not_leak_into_scan_warnings() {
        let temp = TempWorkspace::with_demands(&["p"]);
        temp.create_child("c", "p");
        start_for(&temp.workspace, "p");
        assert_eq!(
            scan(&temp.workspace).unwrap().warnings,
            Vec::<String>::new()
        );
    }

    #[test]
    fn open_can_filter_by_demand() {
        let temp = TempWorkspace::with_demands(&["a", "b"]);
        start_for(&temp.workspace, "a");
        start_for(&temp.workspace, "b");
        assert_eq!(open(&temp.workspace, None).unwrap().items.len(), 2);
        let only_b = open(&temp.workspace, Some(&id("b"))).unwrap().items;
        assert_eq!(only_b.len(), 1);
        assert_eq!(only_b[0].metadata().demand, id("b"));
    }

    #[test]
    fn update_writes_summary_and_next_steps() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let handle = start_for(&temp.workspace, "a");
        let notes = Notes {
            summary: Some("Did things".into()),
            next: Some("More things".into()),
            state: None,
        };
        update(&temp.workspace, &handle.id, &notes).unwrap();
        let record = open(&temp.workspace, None).unwrap().items.remove(0);
        assert_eq!(record.text(section::WORK_COMPLETED).unwrap(), "Did things");
        assert_eq!(record.text(section::NEXT_STEPS).unwrap(), "More things");
    }

    #[test]
    fn update_sets_the_summary_revision_only_with_a_summary() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let handle = start_for(&temp.workspace, "a");
        let next_only = Notes {
            next: Some("later".into()),
            ..Notes::default()
        };
        update(&temp.workspace, &handle.id, &next_only).unwrap();
        let record = open(&temp.workspace, None).unwrap().items.remove(0);
        assert!(record.metadata().fio_summary_revision.is_none());
        update(&temp.workspace, &handle.id, &summary("done")).unwrap();
        let record = open(&temp.workspace, None).unwrap().items.remove(0);
        assert!(record.metadata().fio_summary_revision.is_some());
    }

    #[test]
    fn update_and_end_write_the_demand_current_state() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let handle = start_for(&temp.workspace, "a");
        let state = |text: &str| Notes {
            state: Some(text.to_owned()),
            ..Notes::default()
        };
        update(&temp.workspace, &handle.id, &state("In progress")).unwrap();
        let current = || {
            let demand = demands::load(&temp.workspace, &id("a")).unwrap();
            markdown::section_text(&demand.document.body, section::CURRENT_STATE).unwrap()
        };
        assert_eq!(current(), "In progress");
        let finish = Notes {
            summary: Some("Done".into()),
            state: Some("Finished".into()),
            ..Notes::default()
        };
        end(&temp.workspace, Some(&handle.id), &finish).unwrap();
        assert_eq!(current(), "Finished");
    }

    #[test]
    fn update_requires_an_open_session() {
        let temp = TempWorkspace::with_demands(&["a"]);
        assert_err(
            update(&temp.workspace, "missing", &summary("x")),
            "No matching open session",
        );
    }

    #[test]
    fn end_requires_a_summary() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let handle = start_for(&temp.workspace, "a");
        assert_err(
            end(&temp.workspace, Some(&handle.id), &Notes::default()),
            "Provide --summary",
        );
        assert_eq!(open(&temp.workspace, None).unwrap().items.len(), 1);
    }

    #[test]
    fn end_accepts_a_summary_recorded_earlier() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let handle = start_for(&temp.workspace, "a");
        update(&temp.workspace, &handle.id, &summary("Done")).unwrap();
        end(&temp.workspace, Some(&handle.id), &Notes::default()).unwrap();
    }

    #[test]
    fn end_closes_the_session() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let handle = start_for(&temp.workspace, "a");
        end(&temp.workspace, Some(&handle.id), &summary("Done")).unwrap();
        assert!(open(&temp.workspace, None).unwrap().items.is_empty());
        let all = scan(&temp.workspace).unwrap().items;
        assert!(all[0].metadata().ended.is_some());
    }

    #[test]
    fn closed_sessions_cannot_be_updated_or_ended_again() {
        let temp = TempWorkspace::with_demands(&["a"]);
        let handle = start_for(&temp.workspace, "a");
        end(&temp.workspace, Some(&handle.id), &summary("Done")).unwrap();
        assert_err(
            update(&temp.workspace, &handle.id, &summary("x")),
            "No matching open session",
        );
        assert_err(
            end(&temp.workspace, Some(&handle.id), &summary("x")),
            "No matching open session",
        );
    }

    #[test]
    fn end_without_an_id_is_ambiguous_with_several_open_sessions() {
        let temp = TempWorkspace::with_demands(&["a"]);
        start_for(&temp.workspace, "a");
        start_for(&temp.workspace, "a");
        assert_err(
            end(&temp.workspace, None, &summary("Done")),
            "Multiple sessions are open",
        );
    }

    #[test]
    fn end_without_an_id_closes_the_only_open_session() {
        let temp = TempWorkspace::with_demands(&["a"]);
        start_for(&temp.workspace, "a");
        end(&temp.workspace, None, &summary("Done")).unwrap();
        assert!(open(&temp.workspace, None).unwrap().items.is_empty());
    }

    #[test]
    fn unused_record_skips_names_that_are_taken() {
        let temp = TempWorkspace::new();
        let folder = temp.workspace.demands_dir();
        assert_eq!(unused_record(&folder, "s").0, "s");
        fs::write(folder.join("s.md"), "").unwrap();
        assert_eq!(unused_record(&folder, "s").0, "s-2");
        fs::write(folder.join("s-2.md"), "").unwrap();
        let (id, path) = unused_record(&folder, "s");
        assert_eq!(id, "s-3");
        assert_eq!(path, folder.join("s-3.md"));
    }

    fn copy_session_to_another_demand(temp: &TempWorkspace, handle: &SessionHandle, to: &str) {
        let target = temp.workspace.demand_dir(&id(to)).join("sessions");
        fs::create_dir_all(&target).unwrap();
        fs::copy(&handle.path, target.join("copied.md")).unwrap();
    }

    #[test]
    fn scan_skips_a_record_that_belongs_to_another_demand_with_a_warning() {
        let temp = TempWorkspace::with_demands(&["a", "b"]);
        let handle = start_for(&temp.workspace, "a");
        copy_session_to_another_demand(&temp, &handle, "b");
        let scanned = scan(&temp.workspace).unwrap();
        assert_eq!(scanned.items.len(), 1);
        assert_eq!(scanned.warnings.len(), 1);
    }

    #[test]
    fn read_is_strict_about_a_record_that_belongs_to_another_demand() {
        let temp = TempWorkspace::with_demands(&["a", "b"]);
        let handle = start_for(&temp.workspace, "a");
        copy_session_to_another_demand(&temp, &handle, "b");
        let demand = demands::load(&temp.workspace, &id("b")).unwrap();
        assert_err(read(&demand), "belongs to another demand");
    }
}
