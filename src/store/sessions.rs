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
    let local_id = format!(
        "{}-{}-{:08x}-{}",
        started.format("%Y-%m-%d-%H%M"),
        options.agent.name(),
        Local::now().timestamp_subsec_nanos(),
        std::process::id()
    );
    let folder = workspace.demand_dir(&id).join("sessions");
    fs::create_dir_all(&folder)?;
    let path = folder.join(format!("{local_id}.md"));
    ensure!(
        !path.exists(),
        "A record with this session ID already exists; try again"
    );
    store_fs::write_atomic(&path, &document.render())?;
    Ok(SessionHandle {
        id: local_id,
        demand: id,
        path,
    })
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
