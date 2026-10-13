use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
};

use serde_json::Value;

/// A temporary workspace plus an isolated HOME/XDG tree, so the user's real
/// registry, cache and settings are never touched.
struct Sandbox {
    root: PathBuf,
    workspace: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "fio-cli-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let sandbox = Self { root, workspace };
        sandbox.ok(&["init", "--name", "demo"]);
        sandbox
    }

    fn run(&self, args: &[&str]) -> Output {
        self.run_in(&self.workspace, args, "")
    }

    fn run_in(&self, directory: &std::path::Path, args: &[&str], stdin: &str) -> Output {
        let home = self.root.join("home");
        let mut child = Command::new(env!("CARGO_BIN_EXE_fio"))
            .args(args)
            .current_dir(directory)
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", home.join("config"))
            .env("XDG_CACHE_HOME", home.join("cache"))
            .env("XDG_DATA_HOME", home.join("data"))
            .env("XDG_STATE_HOME", home.join("state"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }

    /// Feeds one lifecycle event to `fio hook claude` and parses its JSON reply.
    fn hook(&self, event: &Value) -> Value {
        let output = self.run_in(&self.workspace, &["hook", "claude"], &event.to_string());
        assert!(output.status.success());
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn ok(&self, args: &[&str]) -> String {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "fio {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }

    fn json(&self, args: &[&str]) -> Value {
        serde_json::from_str(&self.ok(args)).unwrap()
    }

    fn fails(&self, args: &[&str]) -> String {
        let output = self.run(args);
        assert!(!output.status.success(), "fio {args:?} should have failed");
        String::from_utf8(output.stderr).unwrap()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn init_creates_the_workspace_and_registers_the_project() {
    let sandbox = Sandbox::new();
    assert!(sandbox.workspace.join(".fio/project.toml").is_file());
    let projects = sandbox.json(&["projects", "--json"]);
    assert_eq!(projects[0]["name"], "demo");
}

#[test]
fn new_then_list_shows_the_demand() {
    let sandbox = Sandbox::new();
    assert_eq!(
        sandbox.ok(&["new", "login", "Fix login"]),
        "Demand 'login' created\n"
    );
    let listed = sandbox.json(&["list", "--json"]);
    assert_eq!(listed[0]["id"], "login");
    assert_eq!(listed[0]["status"], "backlog");
    assert_eq!(listed[0]["priority"], "medium");
}

#[test]
fn list_filters_by_status() {
    let sandbox = Sandbox::new();
    sandbox.ok(&["new", "a", "A"]);
    sandbox.ok(&["new", "b", "B"]);
    sandbox.ok(&["status", "b", "done"]);
    let done = sandbox.json(&["list", "--status", "done", "--json"]);
    assert_eq!(done.as_array().unwrap().len(), 1);
    assert_eq!(done[0]["id"], "b");
}

#[test]
fn items_flow_from_creation_to_completion() {
    let sandbox = Sandbox::new();
    sandbox.ok(&["new", "a", "A"]);
    sandbox.ok(&["decision", "a", "Use Rust"]);
    sandbox.ok(&["ask", "a", "Which db?"]);
    sandbox.ok(&["debt", "a", "Add tests"]);
    sandbox.ok(&["answer", "a", "Q1", "SQLite"]);
    sandbox.ok(&["resolve", "a", "D1"]);

    let shown = sandbox.json(&["show", "a", "--json"]);
    assert_eq!(shown["decisions"][0]["id"], "DEC1");
    assert_eq!(shown["questions"][0]["done"], true);
    assert_eq!(shown["debts"][0]["done"], true);
}

#[test]
fn answering_an_unknown_question_fails_without_touching_the_demand() {
    let sandbox = Sandbox::new();
    sandbox.ok(&["new", "a", "A"]);
    let before = fs::read_to_string(sandbox.workspace.join(".fio/demands/a/demand.md")).unwrap();
    let error = sandbox.fails(&["answer", "a", "Q9", "nope"]);
    assert!(error.contains("missing or duplicated"), "{error}");
    let after = fs::read_to_string(sandbox.workspace.join(".fio/demands/a/demand.md")).unwrap();
    assert_eq!(before, after);
}

#[test]
fn sub_demands_are_grouped_under_the_parent() {
    let sandbox = Sandbox::new();
    sandbox.ok(&["new", "p", "Parent"]);
    sandbox.ok(&["new", "c", "Child", "--parent", "p"]);
    assert!(
        sandbox
            .workspace
            .join(".fio/demands/p/c/demand.md")
            .is_file()
    );
    let error = sandbox.fails(&["new", "gc", "Grandchild", "--parent", "c"]);
    assert!(error.contains("only one level"), "{error}");

    let shown = sandbox.json(&["show", "p", "--json"]);
    assert_eq!(shown["children"][0]["id"], "c");
}

#[test]
fn context_uses_the_active_demand_when_no_id_is_given() {
    let sandbox = Sandbox::new();
    sandbox.ok(&["new", "a", "A"]);
    assert!(sandbox.fails(&["context"]).contains("No active demand"));
    sandbox.ok(&["use", "a"]);
    let context = sandbox.json(&["context", "--json"]);
    assert_eq!(context["id"], "a");
    assert!(sandbox.ok(&["context"]).starts_with("# a — A\n"));
}

#[test]
fn session_lifecycle_through_the_cli() {
    let sandbox = Sandbox::new();
    sandbox.ok(&["new", "a", "A"]);
    let started = sandbox.json(&[
        "session",
        "start",
        "--demand",
        "a",
        "--agent",
        "claude",
        "--objective",
        "Work",
        "--json",
    ]);
    let id = started["id"].as_str().unwrap().to_owned();

    let open = sandbox.json(&["session", "list", "--json"]);
    assert_eq!(open[0]["id"], id.as_str());

    sandbox.ok(&[
        "session",
        "end",
        &id,
        "--summary",
        "Finished",
        "--next",
        "Nothing",
    ]);
    assert_eq!(
        sandbox.json(&["session", "list", "--json"]),
        Value::Array(vec![])
    );

    let context = sandbox.json(&["context", "a", "--json"]);
    assert_eq!(context["sessions"][0]["summary"], "Finished");
    assert!(context["sessions"][0]["ended"].is_string());
}

#[test]
fn commands_outside_a_workspace_explain_how_to_initialize() {
    let sandbox = Sandbox::new();
    let output = Command::new(env!("CARGO_BIN_EXE_fio"))
        .args(["list"])
        .current_dir(&sandbox.root)
        .env("HOME", sandbox.root.join("home"))
        .env("XDG_CONFIG_HOME", sandbox.root.join("home/config"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("fio init"));
}

#[test]
fn search_finds_demands_and_sessions_with_prefix_and_diacritic_matching() {
    let sandbox = Sandbox::new();
    sandbox.ok(&["new", "a", "Autenticação"]);
    sandbox.ok(&["decision", "a", "Adopt zebrafish tokens"]);
    sandbox.ok(&["new", "b", "Other"]);
    sandbox.ok(&[
        "session",
        "start",
        "--demand",
        "b",
        "--agent",
        "claude",
        "--objective",
        "Investigate quokka",
        "--json",
    ]);

    let by_title = sandbox.json(&["search", "autenticacao", "--json"]);
    assert_eq!(by_title[0]["demand"], "a");
    let by_prefix = sandbox.json(&["search", "zebra", "--json"]);
    assert_eq!(by_prefix[0]["kind"], "demand");
    let in_session = sandbox.json(&["search", "quokka", "--json"]);
    assert_eq!(in_session[0]["kind"], "session");
    assert_eq!(in_session[0]["demand"], "b");
    assert_eq!(
        sandbox.json(&["search", "!!!", "--json"]),
        Value::Array(vec![])
    );
    assert_eq!(
        sandbox.json(&["search", "nonexistentterm", "--json"]),
        Value::Array(vec![])
    );
}

#[test]
fn search_limit_caps_the_number_of_hits() {
    let sandbox = Sandbox::new();
    for id in ["a", "b", "c"] {
        sandbox.ok(&["new", id, "shared keyword"]);
    }
    assert_eq!(
        sandbox
            .json(&["search", "keyword", "--limit", "2", "--json"])
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn all_projects_warns_about_a_project_that_can_no_longer_be_opened() {
    let sandbox = Sandbox::new();
    sandbox.ok(&["new", "a", "A"]);
    let other = sandbox.root.join("other");
    fs::create_dir_all(&other).unwrap();
    let output = sandbox.run_in(&other, &["init", "--name", "other"], "");
    assert!(output.status.success());
    fs::remove_file(other.join(".fio/project.toml")).unwrap();

    let output = sandbox.run(&["list", "--all-projects", "--json"]);
    assert!(output.status.success());
    let listed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(listed[0]["id"], "a");
    assert!(String::from_utf8_lossy(&output.stderr).contains("other"));
}

#[test]
fn hooks_install_merges_and_uninstall_restores_user_settings() {
    let sandbox = Sandbox::new();
    let settings = sandbox.workspace.join(".claude/settings.local.json");
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    let user =
        r#"{"model":"x","hooks":{"Stop":[{"hooks":[{"type":"command","command":"echo hi"}]}]}}"#;
    fs::write(&settings, user).unwrap();

    sandbox.ok(&["hooks", "install", "claude"]);
    let installed: Value = serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();
    assert_eq!(installed["model"], "x");
    assert_eq!(installed["hooks"]["Stop"].as_array().unwrap().len(), 2);
    assert!(installed["hooks"]["SessionStart"].is_array());

    sandbox.ok(&["hooks", "uninstall", "claude"]);
    let restored: Value = serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();
    assert_eq!(restored, serde_json::from_str::<Value>(user).unwrap());
}

#[test]
fn uninstall_deletes_a_settings_file_that_only_held_fio_hooks() {
    let sandbox = Sandbox::new();
    sandbox.ok(&["hooks", "install", "codex"]);
    let settings = sandbox.workspace.join(".codex/hooks.json");
    assert!(settings.is_file());
    sandbox.ok(&["hooks", "uninstall", "codex"]);
    assert!(!settings.exists());
}

#[test]
fn hook_session_start_links_the_pinned_demand_and_stop_asks_for_a_summary() {
    let sandbox = Sandbox::new();
    sandbox.ok(&["new", "a", "A"]);
    sandbox.ok(&["use", "a"]);
    let cwd = sandbox.workspace.to_str().unwrap();

    let start = sandbox.hook(&serde_json::json!({
        "session_id": "native-1", "cwd": cwd, "hook_event_name": "SessionStart", "source": "startup"
    }));
    let context = start["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(context.contains("# a — A"));
    let open = sandbox.json(&["session", "list", "--json"]);
    assert_eq!(open[0]["session_id"], "native-1");

    let stop = sandbox.hook(&serde_json::json!({
        "session_id": "native-1", "cwd": cwd, "hook_event_name": "Stop"
    }));
    assert_eq!(stop["decision"], "block");

    sandbox.hook(&serde_json::json!({
        "session_id": "native-1", "cwd": cwd, "hook_event_name": "SessionEnd"
    }));
    assert_eq!(
        sandbox.json(&["session", "list", "--json"]),
        Value::Array(vec![])
    );
}

#[test]
fn hook_never_fails_the_agent_on_bad_input() {
    let sandbox = Sandbox::new();
    let output = sandbox.run_in(&sandbox.workspace, &["hook", "claude"], "not json");
    assert!(output.status.success());
    let reply: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        reply["systemMessage"]
            .as_str()
            .unwrap()
            .contains("Invalid lifecycle event")
    );

    let unknown = sandbox.hook(&serde_json::json!({
        "session_id": "n", "cwd": "/", "hook_event_name": "PreToolUse"
    }));
    assert_eq!(unknown, serde_json::json!({}));
}

#[test]
fn hook_outside_a_workspace_is_a_silent_no_op() {
    let sandbox = Sandbox::new();
    let reply = sandbox.hook(&serde_json::json!({
        "session_id": "n", "cwd": sandbox.root.to_str().unwrap(), "hook_event_name": "SessionStart"
    }));
    assert_eq!(reply, serde_json::json!({}));
}
