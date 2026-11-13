//! Authorization regressions exercise the real executor, including its approval
//! channel and retained session grants. The counting tools deliberately perform
//! no filesystem writes: invocation itself is the protected side effect.

use std::collections::HashSet;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use tokio::sync::mpsc;
use uuid::Uuid;

use super::{execute_tool_round, inspect_tool_for_auth, ToolAuthRegistry};
use crate::db::models::HookConfig;
use crate::error::AppResult;
use crate::harness::authority::{AuthorizationDecision, PathAuthSession, PathWhitelistConfig};
use crate::harness::event_log::EventCtx;
use crate::harness::mcp::{AuthorizationLevel, McpClient, McpRegistry, ToolContext};
use crate::harness::r#loop::emitter::LoopEmitter;
use crate::infra::cancel::CancellationToken;
use crate::infra::protocol::{AuthScope, ContentBlock, ToolAuthResponse};

struct CountingTool {
    name: &'static str,
    calls: Arc<AtomicUsize>,
    level: AuthorizationLevel,
}

#[async_trait]
impl McpClient for CountingTool {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        "Count protected tool invocations"
    }

    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {
            "source": {"type": "string"}, "destination": {"type": "string"}
        }})
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        self.level
    }

    async fn execute(&self, _args: &str) -> AppResult<String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok("executed".to_string())
    }
}

struct AuthEmitter {
    requests: mpsc::UnboundedSender<Value>,
    events: Mutex<Vec<(String, Value)>>,
}

impl AuthEmitter {
    fn new() -> (Self, mpsc::UnboundedReceiver<Value>) {
        let (requests, receiver) = mpsc::unbounded_channel();
        (
            Self {
                requests,
                events: Mutex::new(Vec::new()),
            },
            receiver,
        )
    }

    fn count(&self, name: &str) -> usize {
        self.events
            .lock()
            .expect("events lock")
            .iter()
            .filter(|(event, _)| event == name)
            .count()
    }
}

impl LoopEmitter for AuthEmitter {
    fn emit(&self, event: &str, payload: Value) {
        self.events
            .lock()
            .expect("events lock")
            .push((event.to_string(), payload.clone()));
        if event == "chat:tool-auth-request" {
            self.requests.send(payload).expect("approval receiver");
        }
    }
}

enum Response {
    Decide { allowed: bool, scope: AuthScope },
    Cancel,
}

struct Fixture {
    registry: McpRegistry,
    auth: ToolAuthRegistry,
    session: PathAuthSession,
    whitelist: PathWhitelistConfig,
    ctx: ToolContext,
    assistant_id: String,
    calls: Arc<AtomicUsize>,
    root: PathBuf,
}

impl Fixture {
    async fn new() -> Self {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO agents (id, name, provider, model, api_key_ref) VALUES ('auth-agent', 'Auth test', 'anthropic', 'test', '')")
            .execute(&pool).await.unwrap();
        let conv_id = Uuid::new_v4().to_string();
        let assistant_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO conversations (id, agent_id, title) VALUES (?, 'auth-agent', 'Auth test')",
        )
        .bind(&conv_id)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO messages (id, conversation_id, role, content) VALUES (?, ?, 'assistant', '')")
            .bind(&assistant_id).bind(&conv_id).execute(&pool).await.unwrap();
        let registry = McpRegistry::new();
        let calls = Arc::new(AtomicUsize::new(0));
        for name in ["copy_file", "move_file"] {
            registry
                .register(Arc::new(CountingTool {
                    name,
                    calls: calls.clone(),
                    level: AuthorizationLevel::PathWhitelist,
                }))
                .await;
        }
        Self {
            registry,
            auth: ToolAuthRegistry::new(),
            session: PathAuthSession::new(),
            whitelist: PathWhitelistConfig::default(),
            ctx: ToolContext {
                conv_id,
                agent_id: "auth-agent".to_string(),
                project_id: None,
                workspace: None,
                pool,
                api_key: None,
                app_handle: None,
                proposal_registry: None,
                turn_id: None,
                cancel: None,
                tool_use_id: None,
            },
            assistant_id,
            calls,
            // These paths need not exist; tests never read, create or delete them.
            root: std::env::temp_dir().join(format!("ice-paw-auth-{}", Uuid::new_v4())),
        }
    }

    fn path(&self, relative: &str) -> String {
        self.root.join(relative).to_string_lossy().into_owned()
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    async fn register_confirm_tool(&self, name: &'static str) {
        self.registry
            .register(Arc::new(CountingTool {
                name,
                calls: self.calls.clone(),
                level: AuthorizationLevel::Confirm,
            }))
            .await;
    }

    async fn run(
        &self,
        emitter: &AuthEmitter,
        name: &str,
        args: &Value,
        cancel: &CancellationToken,
    ) -> Vec<ContentBlock> {
        let completed = [(
            Uuid::new_v4().to_string(),
            name.to_string(),
            args.to_string(),
        )];
        execute_tool_round(
            emitter,
            None,
            &self.registry,
            &self.auth,
            &self.session,
            &self.whitelist,
            &completed,
            &self.ctx,
            &self.assistant_id,
            &EventCtx::new(&self.ctx.conv_id, "auth-turn", &self.ctx.agent_id),
            cancel,
            &HookConfig::default(),
        )
        .await
        .expect("executor result")
    }

    async fn with_approval(
        &self,
        name: &str,
        args: &Value,
        response: Response,
        inspect: impl FnOnce(&Value),
    ) -> (Vec<ContentBlock>, AuthEmitter) {
        let (emitter, mut requests) = AuthEmitter::new();
        let cancel = CancellationToken::new();
        let calls_before = self.calls();
        let responder = async {
            let request = requests.recv().await.expect("approval request");
            assert_eq!(
                self.calls(),
                calls_before,
                "must await approval before invoking"
            );
            inspect(&request);
            match response {
                Response::Decide { allowed, scope } => {
                    assert!(
                        self.auth
                            .respond(ToolAuthResponse {
                                request_id: request["request_id"].as_str().unwrap().to_string(),
                                allowed,
                                scope,
                                delegation_grant: None,
                            })
                            .await
                    );
                }
                Response::Cancel => cancel.cancel(),
            }
        };
        let (blocks, ()) = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(self.run(&emitter, name, args, &cancel), responder)
        })
        .await
        .expect("executor must request approval and settle promptly");
        assert_eq!(self.auth.pending_count().await, 0);
        assert_eq!(emitter.count("chat:tool-auth-request"), 1);
        (blocks, emitter)
    }

    async fn without_approval(&self, name: &str, args: &Value) -> Vec<ContentBlock> {
        let (emitter, _requests) = AuthEmitter::new();
        let blocks = tokio::time::timeout(
            Duration::from_secs(5),
            self.run(&emitter, name, args, &CancellationToken::new()),
        )
        .await
        .expect("authorized invocation must not wait for approval");
        assert_eq!(emitter.count("chat:tool-auth-request"), 0);
        assert_eq!(self.auth.pending_count().await, 0);
        blocks
    }
}

fn assert_result(blocks: &[ContentBlock], expected_error: bool) {
    assert_eq!(blocks.len(), 1);
    match &blocks[0] {
        ContentBlock::ToolResult { is_error, .. } => {
            assert_eq!(is_error.unwrap_or(false), expected_error);
        }
        other => panic!("expected tool result, got {other:?}"),
    }
}

fn deny() -> Response {
    Response::Decide {
        allowed: false,
        scope: AuthScope::Once,
    }
}

#[tokio::test]
async fn granted_source_directory_does_not_authorize_copy_or_move_destination() {
    for name in ["copy_file", "move_file"] {
        let f = Fixture::new().await;
        let source = f.path("source/input.txt");
        let destination = f.path("destination/output.txt");
        f.session.mark_dir_authorized(&f.path("source")).await;
        let args = json!({"source": source, "destination": destination});
        let (blocks, _) = f
            .with_approval(name, &args, deny(), |request| {
                assert_eq!(request["file_path"], destination);
                assert_eq!(request["directory_path"], f.path("destination"));
                assert_eq!(
                    request["paths"],
                    json!([
                        {"key":"source", "path": source, "requires_approval": false},
                        {"key":"destination", "path": destination, "requires_approval": true}
                    ])
                );
            })
            .await;
        assert_result(&blocks, true);
        assert_eq!(f.calls(), 0, "denial must prevent {name}");
        assert!(!f.session.is_authorized(&destination, name).await);
        let audit_error: i64 = sqlx::query_scalar("SELECT is_error FROM tool_calls")
            .fetch_one(&f.ctx.pool)
            .await
            .unwrap();
        assert_eq!(
            audit_error, 1,
            "denied operation remains an error in the audit"
        );
    }
}

#[tokio::test]
async fn cancelling_destination_approval_never_invokes_tool_or_retains_grant() {
    let f = Fixture::new().await;
    f.session.mark_dir_authorized(&f.path("source")).await;
    let destination = f.path("destination/output.txt");
    let args = json!({"source": f.path("source/input.txt"), "destination": destination});
    let (blocks, emitter) = f
        .with_approval("move_file", &args, Response::Cancel, |request| {
            assert_eq!(request["file_path"], destination);
        })
        .await;
    assert_result(&blocks, true);
    assert_eq!(f.calls(), 0);
    assert_eq!(emitter.count("chat:tool-auth-request-cancel"), 1);
    assert!(!f.session.is_authorized(&destination, "move_file").await);
}

#[tokio::test]
async fn this_dir_grants_displayed_destination_without_expanding_source_exact_grant() {
    let f = Fixture::new().await;
    let source = f.path("source/input.txt");
    let destination = f.path("destination/output.txt");
    f.session.mark_authorized(&source).await;
    let args = json!({"source": source, "destination": destination});
    let (blocks, _) = f
        .with_approval(
            "copy_file",
            &args,
            Response::Decide {
                allowed: true,
                scope: AuthScope::ThisDir,
            },
            |request| {
                assert_eq!(request["file_path"], destination);
                assert_eq!(request["directory_path"], f.path("destination"));
            },
        )
        .await;
    assert_result(&blocks, false);
    assert_eq!(f.calls(), 1);
    assert!(
        f.session
            .is_authorized(&f.path("destination/other.txt"), "copy_file")
            .await
    );
    assert!(
        !f.session
            .is_authorized(&f.path("source/other.txt"), "copy_file")
            .await
    );
    assert!(!f.session.is_tool_authorized("copy_file").await);

    let next = json!({"source": f.path("source/other.txt"), "destination": f.path("destination/other.txt")});
    let (blocks, _) = f
        .with_approval("copy_file", &next, deny(), |request| {
            assert_eq!(request["file_path"], f.path("source/other.txt"));
            assert_eq!(request["paths"][1]["requires_approval"], false);
        })
        .await;
    assert_result(&blocks, true);
    assert_eq!(f.calls(), 1);
}

#[tokio::test]
async fn approval_of_two_ungranted_paths_retains_only_the_selected_scope() {
    for scope in [AuthScope::Once, AuthScope::ThisDir] {
        let f = Fixture::new().await;
        let source = f.path("source/input.txt");
        let destination = f.path("destination/output.txt");
        let args = json!({"source": source, "destination": destination});
        let (blocks, _) = f
            .with_approval(
                "copy_file",
                &args,
                Response::Decide {
                    allowed: true,
                    scope,
                },
                |request| {
                    assert_eq!(request["file_path"], source);
                    assert_eq!(request["directory_path"], f.path("source"));
                    assert!(request["paths"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .all(|path| path["requires_approval"] == true));
                },
            )
            .await;
        assert_result(&blocks, false);
        assert_eq!(f.calls(), 1, "approval permits this invocation");
        assert!(f.session.is_authorized(&source, "copy_file").await);
        assert!(!f.session.is_authorized(&destination, "copy_file").await);
        assert!(
            !f.session
                .is_authorized(&f.path("destination/other.txt"), "copy_file")
                .await
        );
        assert_eq!(
            f.session
                .is_authorized(&f.path("source/other.txt"), "copy_file")
                .await,
            scope == AuthScope::ThisDir
        );

        let (blocks, _) = f
            .with_approval("copy_file", &args, deny(), |request| {
                assert_eq!(request["file_path"], destination);
                assert_eq!(request["directory_path"], f.path("destination"));
            })
            .await;
        assert_result(&blocks, true);
        assert_eq!(
            f.calls(),
            1,
            "unselected destination still needs approval next time"
        );
    }
}

#[tokio::test]
async fn explicit_tool_grant_covers_new_paths_but_not_another_tool() {
    let f = Fixture::new().await;
    let args = json!({"source": f.path("source/input.txt"), "destination": f.path("destination/output.txt")});
    let (blocks, _) = f
        .with_approval(
            "copy_file",
            &args,
            Response::Decide {
                allowed: true,
                scope: AuthScope::ThisTool,
            },
            |_| {},
        )
        .await;
    assert_result(&blocks, false);
    assert!(f.session.is_tool_authorized("copy_file").await);
    let other = json!({"source": f.path("other-source/a.txt"), "destination": f.path("other-destination/b.txt")});
    assert_result(&f.without_approval("copy_file", &other).await, false);
    assert_eq!(f.calls(), 2);
    let (blocks, _) = f.with_approval("move_file", &other, deny(), |_| {}).await;
    assert_result(&blocks, true);
    assert_eq!(f.calls(), 2);
}

#[tokio::test]
async fn automatic_mode_allows_both_paths_without_leaking_grants_after_disable() {
    let f = Fixture::new().await;
    let args = json!({"source": f.path("source/input.txt"), "destination": f.path("destination/output.txt")});
    crate::harness::auto_approve::global()
        .set(&f.ctx.conv_id, true)
        .await;
    // Always clear global state, including on a failed/blocked execution assertion.
    let (emitter, _requests) = AuthEmitter::new();
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        f.run(&emitter, "copy_file", &args, &CancellationToken::new()),
    )
    .await;
    crate::harness::auto_approve::global()
        .set(&f.ctx.conv_id, false)
        .await;
    assert_result(&result.expect("automatic execution"), false);
    assert_eq!(emitter.count("chat:tool-auth-request"), 0);
    assert_eq!(f.calls(), 1);
    let (blocks, _) = f.with_approval("copy_file", &args, deny(), |_| {}).await;
    assert_result(&blocks, true);
    assert_eq!(f.calls(), 1);
}

#[tokio::test]
async fn mixed_workspace_whitelist_and_session_coverage_allows_every_path() {
    let mut f = Fixture::new().await;
    f.ctx.workspace = Some(f.path("workspace"));
    f.whitelist.allowed_paths = vec![f.path("whitelist")];
    f.session.mark_dir_authorized(&f.path("session")).await;
    for (source, destination) in [
        ("workspace", "whitelist"),
        ("whitelist", "workspace"),
        ("workspace", "session"),
        ("session", "workspace"),
        ("whitelist", "session"),
        ("session", "whitelist"),
    ] {
        let args = json!({"source": f.path(&format!("{source}/input.txt")),
            "destination": f.path(&format!("{destination}/output.txt"))});
        assert_result(&f.without_approval("copy_file", &args).await, false);
    }
    assert_eq!(f.calls(), 6);
}

#[tokio::test]
async fn one_sided_grants_require_the_other_path_in_both_directions() {
    let mut f = Fixture::new().await;
    for grant in ["exact", "directory", "whitelist"] {
        for authorized_key in ["source", "destination"] {
            f.session.clear().await;
            f.whitelist.allowed_paths.clear();
            let source = f.path("source/input.txt");
            let destination = f.path("destination/output.txt");
            let authorized_path = if authorized_key == "source" {
                &source
            } else {
                &destination
            };
            match grant {
                "exact" => f.session.mark_authorized(authorized_path).await,
                "directory" => f.session.mark_dir_authorized(&f.path(authorized_key)).await,
                "whitelist" => f.whitelist.allowed_paths.push(f.path(authorized_key)),
                _ => unreachable!(),
            }
            let args = json!({"source": source, "destination": destination});
            let inspection = inspect_tool_for_auth(
                &f.registry,
                "copy_file",
                &args.to_string(),
                &None,
                &f.whitelist,
                &f.session,
            )
            .await;
            let expected_path = if authorized_key == "source" {
                &destination
            } else {
                &source
            };
            match inspection.decision {
                AuthorizationDecision::Confirm { file_path, .. } => {
                    assert_eq!(&file_path, expected_path)
                }
                other => {
                    panic!("{grant} for {authorized_key} must not cover both paths: {other:?}")
                }
            }
            assert_eq!(inspection.paths.len(), 2);
            for path in inspection.paths {
                assert_eq!(path.requires_approval, path.key != authorized_key);
            }
        }
    }
}

#[tokio::test]
async fn directory_scope_uses_the_ungranted_paths_key_not_the_first_paths_key() {
    let f = Fixture::new().await;
    let authorized_dir = f.path("authorized");
    f.session.mark_dir_authorized(&authorized_dir).await;
    for (args, expected_dir) in [
        (
            json!({"dir": authorized_dir, "destination": f.path("target/output.txt")}),
            f.path("target"),
        ),
        (
            json!({"path": f.path("authorized/input.txt"), "dir": f.path("target")}),
            f.path("target"),
        ),
    ] {
        let inspection = inspect_tool_for_auth(
            &f.registry,
            "copy_file",
            &args.to_string(),
            &None,
            &f.whitelist,
            &f.session,
        )
        .await;
        assert!(inspection.decision.needs_confirm());
        assert_eq!(
            inspection.directory_path.as_deref(),
            Some(expected_dir.as_str())
        );
    }
}

#[tokio::test]
async fn empty_paths_and_confirm_tools_do_not_inherit_path_authorization() {
    let mut f = Fixture::new().await;
    let args = json!({"source": f.path("source/input.txt"), "destination": f.path("destination/output.txt")});
    f.session
        .mark_dir_authorized(&f.root.to_string_lossy())
        .await;
    f.whitelist
        .allowed_paths
        .push(f.root.to_string_lossy().into_owned());
    let empty = inspect_tool_for_auth(
        &f.registry,
        "copy_file",
        "{}",
        &Some(f.root.clone()),
        &f.whitelist,
        &f.session,
    )
    .await;
    assert!(
        empty.decision.needs_confirm(),
        "no paths is not proof of coverage"
    );
    assert!(empty.paths.is_empty());
    f.registry
        .register(Arc::new(CountingTool {
            name: "confirm_tool",
            calls: f.calls.clone(),
            level: AuthorizationLevel::Confirm,
        }))
        .await;
    let inspection = inspect_tool_for_auth(
        &f.registry,
        "confirm_tool",
        &args.to_string(),
        &Some(f.root.clone()),
        &f.whitelist,
        &f.session,
    )
    .await;
    assert!(
        inspection.decision.needs_confirm(),
        "path grants cannot authorize a Confirm tool"
    );
}

#[tokio::test]
async fn task_commands_to_none_requires_command_approval_again() {
    let mut f = Fixture::new().await;
    f.register_confirm_tool("run_command").await;
    let base = f.session.clone();
    let args = json!({"command": "count only"});

    f.session = base.with_turn_tool_grants(HashSet::from(["run_command".to_string()]));
    assert_result(&f.without_approval("run_command", &args).await, false);
    assert_eq!(f.calls(), 1);
    assert!(
        base.is_empty().await,
        "automatic grants never enter the base session"
    );

    // A new task or an ordinary chat obtains its own view of the same manual grants.
    f.session = base.with_turn_tool_grants(HashSet::new());
    let (blocks, _) = f
        .with_approval("run_command", &args, deny(), |request| {
            assert_eq!(request["tool_name"], "run_command");
        })
        .await;
    assert_result(&blocks, true);
    assert_eq!(
        f.calls(),
        1,
        "denied command adds no invocation after narrowing"
    );
    assert!(base.is_empty().await);
}

#[tokio::test]
async fn task_all_to_commands_revokes_other_confirm_tools() {
    let mut f = Fixture::new().await;
    f.register_confirm_tool("run_command").await;
    f.register_confirm_tool("confirm_tool").await;
    let base = f.session.clone();
    let args = json!({});

    f.session = base.with_turn_tool_grants(HashSet::from([
        "run_command".to_string(),
        "confirm_tool".to_string(),
    ]));
    assert_result(&f.without_approval("run_command", &args).await, false);
    assert_result(&f.without_approval("confirm_tool", &args).await, false);
    assert_eq!(f.calls(), 2);
    assert!(base.is_empty().await);

    f.session = base.with_turn_tool_grants(HashSet::from(["run_command".to_string()]));
    assert_result(&f.without_approval("run_command", &args).await, false);
    let (blocks, _) = f
        .with_approval("confirm_tool", &args, deny(), |request| {
            assert_eq!(request["tool_name"], "confirm_tool");
        })
        .await;
    assert_result(&blocks, true);
    assert_eq!(
        f.calls(),
        3,
        "only run_command remains automatically authorized"
    );
    assert!(base.is_empty().await);
}

#[tokio::test]
async fn manual_tool_approval_during_task_survives_the_next_none_turn() {
    let mut f = Fixture::new().await;
    f.register_confirm_tool("run_command").await;
    f.register_confirm_tool("confirm_tool").await;
    let base = f.session.clone();
    let args = json!({});

    f.session = base.with_turn_tool_grants(HashSet::from(["run_command".to_string()]));
    let (blocks, _) = f
        .with_approval(
            "confirm_tool",
            &args,
            Response::Decide {
                allowed: true,
                scope: AuthScope::ThisTool,
            },
            |_| {},
        )
        .await;
    assert_result(&blocks, false);
    assert!(base.is_tool_authorized("confirm_tool").await);
    assert!(!base.is_tool_authorized("run_command").await);

    f.session = base.with_turn_tool_grants(HashSet::new());
    assert_result(&f.without_approval("confirm_tool", &args).await, false);
    let (blocks, _) = f.with_approval("run_command", &args, deny(), |_| {}).await;
    assert_result(&blocks, true);
    assert_eq!(
        f.calls(),
        2,
        "manual approval persists while automatic approval expires"
    );
    assert!(base.is_tool_authorized("confirm_tool").await);
    assert!(!base.is_tool_authorized("run_command").await);
}

#[tokio::test]
async fn rejected_or_cancelled_task_approval_does_not_leak_into_the_next_turn() {
    for cancel_request in [false, true] {
        let mut f = Fixture::new().await;
        f.register_confirm_tool("run_command").await;
        f.register_confirm_tool("confirm_tool").await;
        let base = f.session.clone();
        let args = json!({});
        f.session = base.with_turn_tool_grants(HashSet::from(["run_command".to_string()]));

        let response = if cancel_request {
            Response::Cancel
        } else {
            Response::Decide {
                allowed: false,
                scope: AuthScope::ThisTool,
            }
        };
        let (blocks, _) = f
            .with_approval("confirm_tool", &args, response, |_| {})
            .await;
        assert_result(&blocks, true);
        assert_eq!(f.calls(), 0);
        assert!(base.is_empty().await);

        f.session = base.with_turn_tool_grants(HashSet::new());
        for tool in ["run_command", "confirm_tool"] {
            let (blocks, _) = f.with_approval(tool, &args, deny(), |_| {}).await;
            assert_result(&blocks, true);
        }
        assert_eq!(
            f.calls(),
            0,
            "neither rejected nor automatic grants survive"
        );
        assert!(base.is_empty().await);
    }
}
