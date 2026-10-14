use std::{
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail, ensure};
use ratatui::widgets::ListState;

use super::{
    App, Modal,
    screen::{NewDemand, Prompt, RowId, Screen, Tab},
};
use crate::{
    context::{self, DEFAULT_DECISIONS, DEFAULT_SESSIONS},
    model::{Agent, DemandId, ItemKind, Priority, Status, section},
    store::{catalog::DemandKey, demands, markdown, sessions, state, workspace::Workspace},
};

pub(super) struct Picker {
    pub title: String,
    pub choices: Vec<(String, Action)>,
    pub state: ListState,
    pub selected: usize,
}

pub(super) enum Action {
    External(External),
    Status(DemandKey, Status),
    Priority(DemandKey, Priority),
    AddItem(DemandKey, ItemKind),
    CompleteItem(DemandKey, ItemKind, String),
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
        self.choose(
            "Set status",
            Status::ALL
                .into_iter()
                .map(|status| (status.to_string(), Action::Status(target.clone(), status)))
                .collect(),
        );
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
            Prompt::AddItem(key, kind) => {
                let id =
                    demands::load_for_update(&key.workspace()?, &key.id)?.add_item(*kind, text)?;
                self.refresh()?;
                self.message = format!("Recorded {id} in '{}'", key.id);
            }
            Prompt::Answer(key, id) => {
                demands::load_for_update(&key.workspace()?, &key.id)?.complete_item(
                    ItemKind::Question,
                    id,
                    Some(text),
                )?;
                self.refresh()?;
                self.message = format!("Answered {id} in '{}'", key.id);
            }
            Prompt::Search => {
                self.go(Screen::Search);
                text.clone_into(&mut self.query);
                self.invalidate();
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
                    Action::External(External::editor(path, workspace.root())?),
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
        let mut records = sessions::read(&loaded)?;
        records
            .sort_by(|a, b| (a.metadata().started, &a.path).cmp(&(b.metadata().started, &b.path)));
        for record in records.into_iter().rev() {
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
                    "{} · {} · {id}\n  Objective: {}\n  Summary: {}\n  Next: {}",
                    session.agent.name(),
                    session.started.format("%Y-%m-%dT%H:%M:%S%:z"),
                    record.text(section::SESSION_OBJECTIVE)?.replace('\n', " "),
                    record.text(section::WORK_COMPLETED)?.replace('\n', " "),
                    record.text(section::NEXT_STEPS)?.replace('\n', " ")
                ),
                Action::External(External {
                    program: session.agent.name().to_owned(),
                    args,
                    workspace: workspace.root().to_owned(),
                }),
            ));
        }
        ensure!(
            !choices.is_empty(),
            "No resumable sessions. Record a native session ID with --session-id first."
        );
        self.choose("Resume agent session", choices);
        Ok(())
    }

    pub(super) fn begin_priority(&mut self) -> Result<()> {
        let target = self.target()?;
        self.choose(
            "Set priority",
            [Priority::High, Priority::Medium, Priority::Low]
                .into_iter()
                .map(|priority| {
                    (
                        priority.to_string(),
                        Action::Priority(target.clone(), priority),
                    )
                })
                .collect(),
        );
        Ok(())
    }

    pub(super) fn begin_action(&mut self) -> Result<()> {
        let target = self.target()?;
        let kind = if self.is_detail() {
            self.tab.item_kind()
        } else {
            self.selected_row().and_then(|row| row.tab.item_kind())
        };
        if let Some(kind @ (ItemKind::Question | ItemKind::Debt)) = kind {
            let demand = demands::load(&target.workspace()?, &target.id)?;
            let selected = if self.is_detail() {
                None
            } else {
                self.selected_row().and_then(|row| match &row.id {
                    RowId::Item(_, id) => Some(id.as_str()),
                    _ => None,
                })
            };
            let choices = markdown::open_items(&demand.document.body, kind)?
                .into_iter()
                .filter_map(|item| {
                    let id = item.id?;
                    if selected.is_some_and(|selected| selected != id) {
                        return None;
                    }
                    let verb = if kind == ItemKind::Question {
                        "Answer"
                    } else {
                        "Resolve"
                    };
                    Some((
                        format!("{verb} {id}: {}", item.text),
                        Action::CompleteItem(target.clone(), kind, id),
                    ))
                })
                .collect::<Vec<_>>();
            ensure!(!choices.is_empty(), "No numbered open items in this view");
            self.choose("Choose an item", choices);
        } else {
            self.choose(
                "Record in demand",
                ItemKind::ALL
                    .into_iter()
                    .map(|kind| {
                        let title = match kind {
                            ItemKind::Decision => "Record decision",
                            ItemKind::Question => "Ask question",
                            ItemKind::Debt => "Record debt",
                        };
                        (title.to_owned(), Action::AddItem(target.clone(), kind))
                    })
                    .collect(),
            );
        }
        Ok(())
    }

    pub(super) fn apply_action(&mut self, action: Action) -> Result<()> {
        match action {
            Action::External(external) => self.external = Some(external),
            Action::AddItem(key, kind) => self.begin_prompt(Prompt::AddItem(key, kind)),
            Action::CompleteItem(key, ItemKind::Question, id) => {
                self.begin_prompt(Prompt::Answer(key, id));
            }
            Action::CompleteItem(key, kind, id) => {
                demands::load_for_update(&key.workspace()?, &key.id)?
                    .complete_item(kind, &id, None)?;
                self.refresh()?;
                self.message = format!("Resolved {id} in '{}'", key.id);
            }
            Action::Status(key, status) => {
                demands::load_for_update(&key.workspace()?, &key.id)?.set_status(status)?;
                self.refresh()?;
                self.message = format!("Status updated for '{}'", key.id);
            }
            Action::Priority(key, priority) => {
                demands::load_for_update(&key.workspace()?, &key.id)?.set_priority(priority)?;
                self.refresh()?;
                self.message = format!("Priority updated for '{}'", key.id);
            }
        }
        Ok(())
    }

    pub(super) fn preview_text(&self) -> Result<String> {
        let key = self.target()?;
        let entry = self.catalog.find(&key).context("Demand unavailable")?;
        let metadata = &entry.loaded.document.metadata;
        let body = &entry.loaded.document.body;
        let mut records = sessions::read(&entry.loaded)?;
        records.sort_by_key(|record| record.metadata().started);
        let next = records
            .last()
            .map(|record| record.text(section::NEXT_STEPS))
            .transpose()?
            .unwrap_or_default();
        Ok(format!(
            "{}\n{} · {}\n\nObjective\n{}\n\nCurrent state\n{}\n\nNext steps\n{}",
            metadata.title,
            metadata.status,
            metadata.priority,
            markdown::section_text(body, section::OBJECTIVE)?,
            markdown::section_text(body, section::CURRENT_STATE)?,
            if next.is_empty() {
                "Not recorded."
            } else {
                &next
            }
        ))
    }

    pub(super) fn preview_context(&mut self) -> Result<()> {
        let target = self.target()?;
        let workspace = target.workspace()?;
        let demand = demands::load(&workspace, &target.id)?;
        let text =
            context::build(&workspace, &demand, DEFAULT_SESSIONS, DEFAULT_DECISIONS)?.markdown();
        self.modal = Modal::Preview(super::screen::Preview::new(
            format!("Context · {}", target.id),
            text,
        ));
        Ok(())
    }

    fn choose(&mut self, title: &str, choices: Vec<(String, Action)>) {
        // Always show the target before launching an external application.
        self.modal = Modal::Picker(Picker {
            title: title.into(),
            choices,
            selected: 0,
            state: ListState::default(),
        });
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::store::testing::{TempWorkspace, assert_err};

    fn editor(visual: Option<&str>, editor: Option<&str>, path: &Path) -> Result<External> {
        External::with_editor(visual, editor, path, Path::new("/w"))
    }

    fn existing_file() -> (TempWorkspace, PathBuf) {
        let temp = TempWorkspace::new();
        let path = temp.workspace.root().join("notes.md");
        fs::write(&path, "x").unwrap();
        (temp, path)
    }

    #[test]
    fn visual_wins_over_editor() {
        let (_temp, path) = existing_file();
        let external = editor(Some("vim"), Some("nano"), &path).unwrap();
        assert_eq!(external.program, "vim");
        assert_eq!(external.args, [path.to_str().unwrap()]);
    }

    #[test]
    fn a_blank_visual_falls_back_to_editor() {
        let (_temp, path) = existing_file();
        let external = editor(Some("  "), Some("nano"), &path).unwrap();
        assert_eq!(external.program, "nano");
    }

    #[test]
    fn editor_arguments_are_split_like_a_shell_would() {
        let (_temp, path) = existing_file();
        let external = editor(Some("code --wait --folder 'a b'"), None, &path).unwrap();
        assert_eq!(external.program, "code");
        assert_eq!(
            external.args,
            ["--wait", "--folder", "a b", path.to_str().unwrap()]
        );
    }

    #[test]
    fn missing_or_broken_editor_settings_are_explained() {
        let (_temp, path) = existing_file();
        assert_err(editor(None, None, &path), "Set VISUAL or EDITOR");
        assert_err(
            editor(Some("'unterminated"), None, &path),
            "Invalid VISUAL / EDITOR",
        );
        assert_err(editor(None, Some(""), &path), "editor command is empty");
    }

    #[test]
    fn the_editor_target_must_exist() {
        let result = External::editor(Path::new("/definitely/not/here.md"), Path::new("/w"));
        assert_err(result, "does not exist");
    }

    fn external(program: &str, args: &[&str], workspace: &Path) -> External {
        External {
            program: program.into(),
            args: args.iter().map(ToString::to_string).collect(),
            workspace: workspace.to_path_buf(),
        }
    }

    #[test]
    fn run_succeeds_in_the_workspace_directory() {
        let temp = TempWorkspace::new();
        let root = temp.workspace.root();
        external("sh", &["-c", "pwd > marker"], root).run().unwrap();
        let printed = fs::read_to_string(root.join("marker")).unwrap();
        assert_eq!(
            Path::new(printed.trim()).canonicalize().unwrap(),
            root.canonicalize().unwrap()
        );
    }

    #[test]
    fn run_reports_a_failing_exit_status() {
        let temp = TempWorkspace::new();
        let result = external("sh", &["-c", "exit 3"], temp.workspace.root()).run();
        assert_err(result, "exited with");
    }

    #[test]
    fn run_reports_a_program_that_cannot_start() {
        let temp = TempWorkspace::new();
        let result = external("fio-no-such-program", &[], temp.workspace.root()).run();
        assert_err(result, "Could not start fio-no-such-program");
    }
}
