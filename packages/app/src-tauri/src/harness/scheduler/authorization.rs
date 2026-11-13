//! 定时任务预授权的本回合快照。
//!
//! due 队列会串行等待前一个任务完成，入队时的任务行可能已经过时。
//! 每次实际执行前重新读取档位，只返回本轮授权集合，不写会话授权记忆。

use std::collections::HashSet;

use sqlx::SqlitePool;

use crate::db::repo::task;
use crate::error::{AppError, AppResult};
use crate::harness::mcp::{AuthorizationLevel, McpRegistry};

/// 读取任务当前预授权档位并生成独立集合。
///
/// 读取失败或任务已删除均停止本次执行，不能使用排队时的旧档位继续授权。
/// `all` 保持现有范围：全局注册表里的 Confirm 工具，屏幕共享入口除外。
pub(super) async fn resolve_task_tool_grants(
    pool: &SqlitePool,
    task_id: &str,
    registry: &McpRegistry,
) -> AppResult<(String, HashSet<String>)> {
    let current = task::get_by_id(pool, task_id)
        .await?
        .ok_or_else(|| AppError::NotFound {
            resource: "scheduled_task",
            id: task_id.to_string(),
        })?;
    let mode = current.preauth;
    let grants = match mode.as_str() {
        "commands" => HashSet::from(["run_command".to_string()]),
        "all" => registry
            .snapshot()
            .await
            .into_iter()
            .filter_map(|(name, tool)| {
                (name != "request_screen_session"
                    && tool.authorization_level() == AuthorizationLevel::Confirm)
                    .then_some(name)
            })
            .collect(),
        _ => HashSet::new(),
    };
    Ok((mode, grants))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use serde_json::{json, Value};
    use sqlx::sqlite::SqlitePoolOptions;

    use super::*;
    use crate::harness::mcp::McpClient;

    struct AuthOnlyTool {
        name: &'static str,
        level: AuthorizationLevel,
    }

    #[async_trait]
    impl McpClient for AuthOnlyTool {
        fn name(&self) -> &str {
            self.name
        }

        fn description(&self) -> &str {
            "Authorization snapshot test tool"
        }

        fn parameters(&self) -> Value {
            json!({"type": "object"})
        }

        fn authorization_level(&self) -> AuthorizationLevel {
            self.level
        }

        async fn execute(&self, _args: &str) -> AppResult<String> {
            panic!("resolving grants must never execute a tool")
        }
    }

    async fn registry() -> McpRegistry {
        let registry = McpRegistry::new();
        for (name, level) in [
            ("run_command", AuthorizationLevel::Confirm),
            ("other_confirm", AuthorizationLevel::Confirm),
            ("request_screen_session", AuthorizationLevel::Confirm),
            ("copy_file", AuthorizationLevel::PathWhitelist),
            ("read_status", AuthorizationLevel::Always),
        ] {
            registry
                .register(Arc::new(AuthOnlyTool { name, level }))
                .await;
        }
        registry
    }

    async fn pool(mode: &str) -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO agents (id, name, provider, model, api_key_ref) VALUES ('auth-agent', 'Auth test', 'anthropic', 'test', '')")
            .execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO scheduled_tasks
             (id, name, agent_id, schedule_kind, schedule_data, prompt, preauth, enabled, next_run)
             VALUES ('auth-task', 'Auth test', 'auth-agent', 'interval', '{\"minutes\":10}', 'test', ?, 1, '2000-01-01 00:00:00')",
        )
        .bind(mode)
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    async fn set_mode(pool: &SqlitePool, mode: &str) {
        let result = sqlx::query("UPDATE scheduled_tasks SET preauth = ? WHERE id = 'auth-task'")
            .bind(mode)
            .execute(pool)
            .await
            .unwrap();
        assert_eq!(result.rows_affected(), 1);
    }

    #[tokio::test]
    async fn commands_to_none_returns_an_empty_new_snapshot() {
        let pool = pool("commands").await;
        let registry = registry().await;
        let (mode, commands) = resolve_task_tool_grants(&pool, "auth-task", &registry)
            .await
            .unwrap();
        assert_eq!(mode, "commands");
        assert_eq!(commands, HashSet::from(["run_command".to_string()]));

        set_mode(&pool, "none").await;
        let (mode, none) = resolve_task_tool_grants(&pool, "auth-task", &registry)
            .await
            .unwrap();
        assert_eq!(mode, "none");
        assert!(none.is_empty(), "previous commands must not accumulate");
        assert!(
            commands.contains("run_command"),
            "running snapshots stay independent"
        );
    }

    #[tokio::test]
    async fn all_to_commands_removes_other_confirm_grants_and_keeps_existing_scope() {
        let pool = pool("all").await;
        let registry = registry().await;
        let (mode, all) = resolve_task_tool_grants(&pool, "auth-task", &registry)
            .await
            .unwrap();
        assert_eq!(mode, "all");
        assert_eq!(
            all,
            HashSet::from(["run_command".to_string(), "other_confirm".to_string()])
        );
        assert!(!all.contains("request_screen_session"));
        assert!(!all.contains("copy_file"));
        assert!(!all.contains("read_status"));

        set_mode(&pool, "commands").await;
        let (mode, commands) = resolve_task_tool_grants(&pool, "auth-task", &registry)
            .await
            .unwrap();
        assert_eq!(mode, "commands");
        assert_eq!(commands, HashSet::from(["run_command".to_string()]));
        assert!(
            all.contains("other_confirm"),
            "prior snapshot is not mutated"
        );
    }

    #[tokio::test]
    async fn queued_task_uses_saved_mode_instead_of_due_scan_snapshot() {
        let pool = pool("all").await;
        let registry = registry().await;
        let queued = task::due_tasks(&pool, "2099-01-01 00:00:00")
            .await
            .unwrap()
            .pop()
            .expect("due task");
        assert_eq!(queued.preauth, "all");

        // Simulate a user edit while an earlier task occupies the serial scheduler.
        set_mode(&pool, "none").await;
        let (mode, grants) = resolve_task_tool_grants(&pool, &queued.id, &registry)
            .await
            .unwrap();
        assert_eq!(mode, "none");
        assert!(grants.is_empty());
        assert_eq!(
            queued.preauth, "all",
            "the stale queue row must not be authoritative"
        );
    }

    #[tokio::test]
    async fn unknown_modes_never_expand_authority() {
        let pool = pool("unrecognized").await;
        let registry = registry().await;
        for unknown in ["unrecognized", "", "ALL"] {
            set_mode(&pool, unknown).await;
            let (mode, grants) = resolve_task_tool_grants(&pool, "auth-task", &registry)
                .await
                .unwrap();
            assert_eq!(mode, unknown, "retain the saved value for diagnostics");
            assert!(grants.is_empty());
        }
    }

    #[tokio::test]
    async fn deleted_task_fails_instead_of_reusing_earlier_grants() {
        let pool = pool("all").await;
        let registry = registry().await;
        assert!(!resolve_task_tool_grants(&pool, "auth-task", &registry)
            .await
            .unwrap()
            .1
            .is_empty());
        sqlx::query("DELETE FROM scheduled_tasks WHERE id = 'auth-task'")
            .execute(&pool)
            .await
            .unwrap();
        let error = resolve_task_tool_grants(&pool, "auth-task", &registry)
            .await
            .unwrap_err();
        assert!(
            matches!(error, AppError::NotFound { resource: "scheduled_task", id } if id == "auth-task")
        );
    }

    #[tokio::test]
    async fn database_failure_is_propagated_instead_of_reusing_earlier_grants() {
        let pool = pool("all").await;
        let registry = registry().await;
        assert!(!resolve_task_tool_grants(&pool, "auth-task", &registry)
            .await
            .unwrap()
            .1
            .is_empty());
        pool.close().await;
        let error = resolve_task_tool_grants(&pool, "auth-task", &registry)
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::Database(sqlx::Error::PoolClosed)));
    }
}
