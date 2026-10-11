use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use serde::Serialize;

use crate::{
    context::{self, DEFAULT_DECISIONS, DEFAULT_SESSIONS},
    hooks,
    index::Index,
    model::{Agent, DemandId, ItemKind, Priority, Status},
    store::{
        catalog::Catalog,
        demands, registry,
        sessions::{self, Notes, StartOptions},
        state,
        workspace::Workspace,
    },
    tui,
    view::{DemandView, OpenSession},
};

#[derive(Parser)]
#[command(version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Handle an agent lifecycle event received as JSON on stdin
    Hook {
        #[arg(value_enum)]
        agent: Agent,
        #[arg(long)]
        demand: Option<DemandId>,
    },
    /// Configure project-local lifecycle hooks
    Hooks {
        #[command(subcommand)]
        command: HooksCommand,
    },
    /// Search demands and sessions across registered projects
    Search {
        term: String,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// Initialize the current workspace and register the project
    Init {
        /// Project name (defaults to the current directory name)
        #[arg(long)]
        name: Option<String>,
    },
    /// List registered projects
    Projects {
        #[arg(long)]
        json: bool,
    },
    /// Create a demand (backlog status, medium priority by default)
    New {
        id: DemandId,
        title: String,
        #[arg(long, value_enum, default_value = "medium")]
        priority: Priority,
    },
    /// List demands in the current workspace or across all projects
    List {
        #[arg(long, value_enum)]
        status: Option<Status>,
        #[arg(long)]
        all_projects: bool,
        #[arg(long)]
        json: bool,
    },
    /// Show a demand; JSON includes items and sessions
    Show {
        id: DemandId,
        #[arg(long)]
        json: bool,
    },
    /// Change a demand status
    Status {
        id: DemandId,
        #[arg(value_enum)]
        status: Status,
    },
    /// Change a demand priority
    Priority {
        id: DemandId,
        #[arg(value_enum)]
        priority: Priority,
    },
    /// Link existing documents or directories (paths relative to the current directory)
    Link {
        id: DemandId,
        #[arg(required = true, num_args = 1..)]
        paths: Vec<PathBuf>,
    },
    /// Record a decision with ID DEC<n>
    Decision { id: DemandId, text: String },
    /// Record a question with ID Q<n>
    Ask { id: DemandId, text: String },
    /// Answer question Q<n>
    Answer {
        id: DemandId,
        item: String,
        text: String,
    },
    /// Record a technical debt with ID D<n>
    Debt { id: DemandId, text: String },
    /// Resolve debt D<n>
    Resolve { id: DemandId, item: String },
    /// Set the active demand in the workspace
    Use { id: DemandId },
    /// Build context for the specified or active demand
    Context {
        id: Option<DemandId>,
        #[arg(long, default_value_t = DEFAULT_SESSIONS)]
        sessions: usize,
        #[arg(long, default_value_t = DEFAULT_DECISIONS)]
        decisions: usize,
        #[arg(long)]
        json: bool,
    },
    /// Record an agent session in a demand
    Session {
        #[command(subcommand)]
        command: SessionCommand,
    },
}

#[derive(Subcommand)]
pub enum HooksCommand {
    /// Merge fio hooks into the current project's agent settings
    Install {
        #[arg(value_enum)]
        agent: Agent,
        /// fio executable used by hooks (defaults to this executable)
        #[arg(long)]
        executable: Option<PathBuf>,
        /// Pin new native sessions to a demand instead of the workspace default
        #[arg(long)]
        demand: Option<DemandId>,
    },
    /// Remove fio's hooks from the current project's agent settings
    Uninstall {
        #[arg(value_enum)]
        agent: Agent,
    },
}

#[derive(Subcommand)]
pub enum SessionCommand {
    /// Start a session record (default agent: codex)
    Start(StartArgs),
    /// End a session and record its summary and next steps
    End(EndArgs),
    /// Update a session summary without closing the session
    Update(UpdateArgs),
    /// List open sessions in the workspace
    List {
        #[arg(long)]
        demand: Option<DemandId>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Args)]
pub struct StartArgs {
    #[arg(long, value_enum, default_value = "codex")]
    agent: Agent,
    /// Demand for this session (defaults to the active demand)
    #[arg(long)]
    demand: Option<DemandId>,
    #[arg(long)]
    objective: Option<String>,
    /// Native agent session ID, when known
    #[arg(long)]
    session_id: Option<String>,
    #[arg(long)]
    transcript: Option<String>,
    /// Return the local ID, demand and path as JSON
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
pub struct EndArgs {
    /// Local ID returned by session start (required when multiple sessions are open)
    id: Option<String>,
    #[arg(long)]
    summary: Option<String>,
    #[arg(long)]
    next: Option<String>,
    /// Update the demand Current state section
    #[arg(long)]
    state: Option<String>,
}

#[derive(Args)]
pub struct UpdateArgs {
    /// Local fio session ID
    id: String,
    #[arg(long)]
    summary: Option<String>,
    #[arg(long)]
    next: Option<String>,
    /// Replace the demand Current state section
    #[arg(long)]
    state: Option<String>,
}

pub fn run(command: Option<Command>) -> Result<()> {
    let Some(command) = command else {
        return tui::run();
    };
    match command {
        Command::Hook { agent, demand } => hooks::handle(agent, demand),
        Command::Hooks {
            command:
                HooksCommand::Install {
                    agent,
                    executable,
                    demand,
                },
        } => hooks::configure(agent, executable, demand.as_ref()),
        Command::Hooks {
            command: HooksCommand::Uninstall { agent },
        } => hooks::remove(agent),
        Command::Search { term, json, limit } => search(&term, json, limit),
        Command::Init { name } => {
            let workspace = Workspace::init(&std::env::current_dir()?, name.as_deref())?;
            println!(
                "Project '{}' initialized in {}",
                workspace.name(),
                workspace.root().display()
            );
            Ok(())
        }
        Command::Projects { json } => emit(json, &registry::projects()?, |projects| {
            for project in projects {
                println!("{}\t{}", project.name, project.workspace.display());
            }
        }),
        Command::List {
            status,
            all_projects,
            json,
        } => list(status, all_projects, json),
        Command::New {
            id,
            title,
            priority,
        } => {
            demands::create(&Workspace::locate()?, &id, &title, priority)?;
            println!("Demand '{id}' created");
            Ok(())
        }
        Command::Show { id, json } => show(&id, json),
        Command::Status { id, status } => {
            demands::load_for_update(&Workspace::locate()?, &id)?.set_status(status)?;
            println!("Demand '{id}': {status}");
            Ok(())
        }
        Command::Priority { id, priority } => {
            demands::load_for_update(&Workspace::locate()?, &id)?.set_priority(priority)?;
            println!("Demand '{id}': {priority} priority");
            Ok(())
        }
        Command::Link { id, paths } => {
            let workspace = Workspace::locate()?;
            demands::load_for_update(&workspace, &id)?.link(&workspace, paths)?;
            println!("Documents linked to '{id}'");
            Ok(())
        }
        Command::Decision { .. }
        | Command::Ask { .. }
        | Command::Debt { .. }
        | Command::Answer { .. }
        | Command::Resolve { .. } => item_command(command),
        Command::Use { id } => {
            state::select(&Workspace::locate()?, &id)?;
            println!("Active demand: {id}");
            Ok(())
        }
        Command::Context {
            id,
            sessions,
            decisions,
            json,
        } => {
            let workspace = Workspace::locate()?;
            let id = match id {
                Some(id) => id,
                None => state::active(&workspace)?,
            };
            let demand = demands::load(&workspace, &id)?;
            let package = context::build(&workspace, &demand, sessions, decisions)?;
            if json {
                print_json(&package)
            } else {
                print!("{}", package.markdown());
                Ok(())
            }
        }
        Command::Session { command } => run_session(&Workspace::locate()?, command),
    }
}

fn search(term: &str, json: bool, limit: usize) -> Result<()> {
    let catalog = Catalog::load()?;
    warn(&catalog.warnings);
    let hits = Index::rebuild(&catalog)?.search(term, limit)?;
    emit(json, &hits, |hits| {
        for hit in hits {
            println!(
                "{}/{} [{}] {}\n{}\n{}",
                hit.project,
                hit.demand,
                hit.kind.name(),
                hit.title,
                hit.snippet,
                hit.path.display()
            );
        }
    })
}

fn show(id: &DemandId, json: bool) -> Result<()> {
    let demand = demands::load(&Workspace::locate()?, id)?;
    if json {
        let sessions = sessions::read(&demand)?;
        print_json(&DemandView::new(&demand, &sessions)?)
    } else {
        print!("{}", demand.document.render());
        Ok(())
    }
}

fn list(status: Option<Status>, all_projects: bool, json: bool) -> Result<()> {
    let workspaces = if all_projects {
        let mut opened = Vec::new();
        for project in registry::projects()? {
            match Workspace::open(project.workspace.clone()) {
                Ok(workspace) => opened.push(workspace),
                Err(error) => warn(&[format!("{}: {error:#}", project.name)]),
            }
        }
        opened
    } else {
        vec![Workspace::locate()?]
    };
    let mut summaries = Vec::new();
    for workspace in &workspaces {
        let scanned = demands::list(workspace, status)?;
        warn(&scanned.warnings);
        summaries.extend(scanned.items);
    }
    emit(json, &summaries, |summaries| {
        for entry in summaries {
            println!(
                "{}\t{}\t{}\t{}",
                entry.demand.project, entry.demand.id, entry.demand.status, entry.demand.title
            );
        }
    })
}

fn item_command(command: Command) -> Result<()> {
    let workspace = Workspace::locate()?;
    match command {
        Command::Decision { id, text } => add_item(&workspace, &id, ItemKind::Decision, &text),
        Command::Ask { id, text } => add_item(&workspace, &id, ItemKind::Question, &text),
        Command::Debt { id, text } => add_item(&workspace, &id, ItemKind::Debt, &text),
        Command::Answer { id, item, text } => {
            demands::load_for_update(&workspace, &id)?.complete_item(
                ItemKind::Question,
                &item,
                Some(&text),
            )?;
            println!("Question '{item}' answered");
            Ok(())
        }
        Command::Resolve { id, item } => {
            demands::load_for_update(&workspace, &id)?.complete_item(
                ItemKind::Debt,
                &item,
                None,
            )?;
            println!("Debt '{item}' resolved");
            Ok(())
        }
        _ => unreachable!("only item commands are routed here"),
    }
}

fn add_item(workspace: &Workspace, id: &DemandId, kind: ItemKind, text: &str) -> Result<()> {
    let item = demands::load_for_update(workspace, id)?.add_item(kind, text)?;
    println!("Item '{item}' recorded in '{id}'");
    Ok(())
}

fn run_session(workspace: &Workspace, command: SessionCommand) -> Result<()> {
    match command {
        SessionCommand::Start(args) => {
            let handle = sessions::start(
                workspace,
                StartOptions {
                    agent: args.agent,
                    demand: args.demand,
                    objective: args.objective,
                    session_id: args.session_id,
                    transcript: args.transcript,
                },
            )?;
            if args.json {
                print_json(&handle)
            } else {
                println!("Session {} started at {}", handle.id, handle.path.display());
                Ok(())
            }
        }
        SessionCommand::Update(args) => {
            let notes = Notes {
                summary: args.summary,
                next: args.next,
                state: args.state,
            };
            let path = sessions::update(workspace, &args.id, &notes)?;
            println!("Session updated at {}", path.display());
            Ok(())
        }
        SessionCommand::End(args) => {
            let notes = Notes {
                summary: args.summary,
                next: args.next,
                state: args.state,
            };
            let path = sessions::end(workspace, args.id.as_deref(), &notes)?;
            println!("Session ended at {}", path.display());
            Ok(())
        }
        SessionCommand::List { demand, json } => {
            let scanned = sessions::open(workspace, demand.as_ref())?;
            warn(&scanned.warnings);
            let open: Vec<_> = scanned.items.iter().map(OpenSession::new).collect();
            emit(json, &open, |open| {
                for session in open {
                    println!(
                        "{}\t{}\t{}\t{}",
                        session.id,
                        session.demand,
                        session.agent.name(),
                        session.started.format("%Y-%m-%dT%H:%M:%S%:z")
                    );
                }
            })
        }
    }
}

fn warn(warnings: &[String]) {
    for warning in warnings {
        eprintln!("Warning: {warning}");
    }
}

fn print_json(value: &impl Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

/// Prints `value` as JSON, or through `text` otherwise.
fn emit<T: Serialize>(json: bool, value: &T, text: impl FnOnce(&T)) -> Result<()> {
    if json {
        print_json(value)
    } else {
        text(value);
        Ok(())
    }
}
