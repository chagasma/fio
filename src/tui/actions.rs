use std::{
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail, ensure};
use clap::ValueEnum;

use super::{
    App, Modal,
    screen::{NewDemand, Prompt, Screen, Tab},
};
use crate::{
    context::{self, DEFAULT_DECISIONS, DEFAULT_SESSIONS},
    model::{Agent, DemandId, Priority, Status},
    store::{catalog::DemandKey, demands, sessions, state, workspace::Workspace},
};

pub(super) struct Picker {
    pub title: String,
    pub choices: Vec<(String, External)>,
    pub selected: usize,
}

/// A program to run outside the TUI, in a workspace.
pub(super) struct External {
    program: String,
    args: Vec<String>,
    workspace: PathBuf,
}

impl External {
    fn editor(path: &Path, workspace: &Path) -> Result<Self> {
        ensure!(
            path.exists(),
            "The editor target does not exist: {}",
            path.display()
        );
        Self::with_editor(
            std::env::var("VISUAL").ok().as_deref(),
            std::env::var("EDITOR").ok().as_deref(),
            path,
            workspace,
        )
    }

    /// `visual` wins over `editor` unless it is blank.
    fn with_editor(
        visual: Option<&str>,
        editor: Option<&str>,
        path: &Path,
        workspace: &Path,
    ) -> Result<Self> {
        let editor = visual
            .filter(|s| !s.trim().is_empty())
            .or(editor)
            .context("Set VISUAL or EDITOR to your preferred terminal editor")?;
        let mut words = shell_words::split(editor)
            .context("Invalid VISUAL / EDITOR command")?
            .into_iter();
        let program = words.next().context("The editor command is empty")?;
        let mut args: Vec<String> = words.collect();
        args.push(
            path.to_str()
                .context("Editor path must be UTF-8")?
                .to_owned(),
        );
        Ok(Self {
            program,
            args,
            workspace: workspace.to_owned(),
        })
    }

    pub fn run(self) -> Result<()> {
        let status = Command::new(&self.program)
            .args(&self.args)
            .current_dir(&self.workspace)
            .status()
            .with_context(|| format!("Could not start {}", self.program))?;
        ensure!(status.success(), "{} exited with {status}", self.program);
        Ok(())
    }
}

impl DemandKey {
    fn workspace(&self) -> Result<Workspace> {
        Workspace::open(self.workspace.clone())
    }
}

impl App {
    /// The demand shown on the detail screen or selected in the current list.
    pub(super) fn target(&self) -> Result<DemandKey> {
        let screen = if self.is_detail() {
            Some(&self.screen)
        } else {
            self.selected_row().map(|row| &row.target)
        };
        match screen {
            Some(Screen::Detail(key)) => Ok(key.clone()),
            _ => bail!("Select a demand first"),
        }
    }

    pub(super) fn begin_prompt(&mut self, prompt: Prompt) {
        self.message.clear();
        self.modal = Modal::Input {
            prompt,
            text: String::new(),
        };
    }

    pub(super) fn begin_new(&mut self) -> Result<()> {
        let workspace = match &self.screen {
            Screen::Projects => self
                .catalog
                .projects
                .get(self.selection.selected().unwrap_or(0))
                .context("Select a project first")?
                .workspace
                .clone(),
            Screen::Demands(Some(workspace)) => workspace.clone(),
            _ => self.target()?.workspace,
        };
        self.begin_prompt(Prompt::NewId(NewDemand {
            workspace,
            parent: None,
        }));
        Ok(())
    }

    /// Starts a sub-demand inside the selected demand.
    pub(super) fn begin_new_child(&mut self) -> Result<()> {
        let parent = self.target()?;
        let entry = self
            .catalog
            .find(&parent)
            .context("Select a demand first")?;
        ensure!(
            entry.loaded.document.metadata.parent.is_none(),
            "'{}' is already a sub-demand; only one level is supported",
            parent.id
        );
        self.begin_prompt(Prompt::NewId(NewDemand {
            workspace: parent.workspace,
            parent: Some(parent.id),
        }));
        Ok(())
    }

    pub(super) fn begin_status(&mut self) -> Result<()> {
        let target = self.target()?;
        self.begin_prompt(Prompt::Status(target));
        Ok(())
    }

    pub(super) fn submit(&mut self, prompt: &Prompt, text: &str) -> Result<()> {
        match prompt {
            Prompt::NewId(target) => {
                let id: DemandId = text.trim().parse().map_err(anyhow::Error::msg)?;
                self.modal = Modal::Input {
                    prompt: Prompt::NewTitle(target.clone(), id),
                    text: String::new(),
                };
            }
            Prompt::NewTitle(target, id) => {
                let workspace = Workspace::open(target.workspace.clone())?;
                demands::create(
                    &workspace,
                    id,
                    text,
                    Priority::default(),
                    target.parent.as_ref(),
                )?;
                self.refresh()?;
                self.go(Screen::Detail(DemandKey {
                    workspace: target.workspace.clone(),
                    id: id.clone(),
                }));
                self.tab = Tab::Summary;
                self.message = format!("Demand '{id}' created");
            }
            Prompt::Status(key) => {
                let status = Status::from_str(text.trim(), true).map_err(anyhow::Error::msg)?;
                demands::load_for_update(&key.workspace()?, &key.id)?.set_status(status)?;
                self.refresh()?;
                self.message = format!("Status updated for '{}'", key.id);
            }
            Prompt::Search => {
                text.clone_into(&mut self.query);
                self.go(Screen::Search);
            }
            Prompt::Command => self.command(text)?,
        }
        Ok(())
    }

    pub(super) fn use_selected(&mut self) -> Result<()> {
        let target = self.target()?;
        state::select(&target.workspace()?, &target.id)?;
        self.refresh()?;
        self.message = format!("Active demand: {}", target.id);
        Ok(())
    }

    pub(super) fn copy_context(&mut self) -> Result<()> {
        let target = self.target()?;
        let workspace = target.workspace()?;
        let demand = demands::load(&workspace, &target.id)?;
        let text =
            context::build(&workspace, &demand, DEFAULT_SESSIONS, DEFAULT_DECISIONS)?.markdown();
        let clipboard = match &mut self.clipboard {
            Some(clipboard) => clipboard,
            empty => empty.insert(arboard::Clipboard::new().context("Clipboard unavailable")?),
        };
        clipboard.set_text(text).context("Could not copy context")?;
        self.message = format!("Context copied for '{}'", target.id);
        Ok(())
    }

    pub(super) fn edit_selected(&mut self) -> Result<()> {
        let target = self.target()?;
        let workspace = target.workspace()?;
        let loaded = demands::load(&workspace, &target.id)?;
        let paths: Vec<PathBuf> = match (self.is_detail(), self.tab) {
            (true, Tab::Sessions) => sessions::read(&loaded)?
                .into_iter()
                .rev()
                .map(|record| record.path)
                .collect(),
            (true, Tab::Docs) => loaded
                .document
                .metadata
                .docs
                .iter()
                .map(|doc| workspace.root().join(doc))
                .collect(),
            _ => vec![loaded.path],
        };
        ensure!(!paths.is_empty(), "No files recorded in this tab");
        let choices = paths
            .iter()
            .map(|path| {
                Ok((
                    path.display().to_string(),
                    External::editor(path, workspace.root())?,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        self.choose("Open in editor", choices);
        Ok(())
    }

    pub(super) fn resume_selected(&mut self) -> Result<()> {
        let target = self.target()?;
        let workspace = target.workspace()?;
        let loaded = demands::load(&workspace, &target.id)?;
        let mut choices = Vec::new();
        for record in sessions::read(&loaded)?.into_iter().rev() {
            let session = record.metadata();
            let Some(id) = session.session_id.as_deref() else {
                continue;
            };
            if id.is_empty() || id.starts_with('-') || id.chars().any(char::is_control) {
                continue;
            }
            let args = match session.agent {
                Agent::Codex => vec!["resume".to_owned(), id.to_owned()],
                Agent::Claude => vec!["--resume".to_owned(), id.to_owned()],
            };
            choices.push((
                format!(
                    "{} · {} · {id}",
                    session.agent.name(),
                    session.started.format("%Y-%m-%dT%H:%M:%S%:z")
                ),
                External {
                    program: session.agent.name().to_owned(),
                    args,
                    workspace: workspace.root().to_owned(),
                },
            ));
        }
        ensure!(
            !choices.is_empty(),
            "No resumable sessions. Record a native session ID with --session-id first."
        );
        self.choose("Resume agent session", choices);
        Ok(())
    }

    fn choose(&mut self, title: &str, choices: Vec<(String, External)>) {
        // Always show the target before launching an external application.
        self.modal = Modal::Picker(Picker {
            title: title.into(),
            choices,
            selected: 0,
        });
    }
}
