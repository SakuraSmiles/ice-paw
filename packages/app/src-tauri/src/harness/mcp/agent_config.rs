//! `read_agent_config` 工具：读取当前 agent 自己的 `agent.yaml` 配置
//!
//! `Always` 授权（读自己的配置，安全）。走 `execute_with_context`：按 agent_id
//! 读取已登记的固定 config_dir，与 Agent 默认执行目录和项目工作区无关。

use async_trait::async_trait;

use crate::error::{AppError, AppResult};

use super::client::{McpClient, ToolContext};
use super::types::AuthorizationLevel;

pub struct ReadAgentConfigTool;

#[async_trait]
impl McpClient for ReadAgentConfigTool {
    fn name(&self) -> &str {
        "read_agent_config"
    }

    fn description(&self) -> &str {
        "Read this agent's own configuration file (agent.yaml): temperature, max_tokens, \
system prompt, tool limits, etc. Takes no arguments."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object", "properties": {} })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::Always
    }

    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(
            "read_agent_config 必须通过 execute_with_context 调用".into(),
        ))
    }

    async fn execute_with_context(&self, _args: &str, ctx: &ToolContext) -> AppResult<String> {
        let agent = crate::db::repo::agent::get_by_id(&ctx.pool, &ctx.agent_id).await?;
        let dir = agent.config_dir.ok_or_else(|| AppError::NotFound {
            resource: "agent_config_dir",
            id: ctx.agent_id.clone(),
        })?;
        let cfg_path = std::path::Path::new(&dir).join("agent.yaml");
        let content = tokio::fs::read_to_string(&cfg_path).await.map_err(|e| {
            AppError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("读取 agent.yaml 失败（可能未生成）: {e}"),
            ))
        })?;

        Ok(serde_json::json!({
            "agent_id": ctx.agent_id,
            "path": cfg_path.to_string_lossy(),
            "content": content,
        })
        .to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reads_only_managed_config_when_workspaces_change() {
        use crate::db::{models::NewAgent, repo};
        let root =
            std::env::temp_dir().join(format!("icepaw-config-tool-{}", uuid::Uuid::new_v4()));
        let workspace = root.join("work");
        let project = root.join("project");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(workspace.join("agent.yaml"), "system_prompt: workspace").unwrap();
        std::fs::write(project.join("agent.yaml"), "system_prompt: project").unwrap();
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        repo::preferences::set(&pool, "default_workspace_path", &root.to_string_lossy())
            .await
            .unwrap();
        let input: NewAgent = serde_json::from_value(serde_json::json!({
            "id": "editor", "name": "editor", "provider": "custom", "model": "model",
            "api_key": "", "workspace_path": workspace.to_string_lossy(),
            "config_dir": project.to_string_lossy()
        }))
        .unwrap();
        let row = repo::agent::create(&pool, &input, "editor", "editor")
            .await
            .unwrap();
        let dir = std::path::PathBuf::from(row.config_dir.unwrap());
        assert_eq!(dir, root.join("agents/editor"), "客户端不能指定配置目录");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("agent.yaml"), "system_prompt: managed").unwrap();
        let mut ctx = crate::harness::tool_executor::build_tool_ctx(
            &pool,
            "conv".into(),
            "editor".into(),
            None,
            None,
        )
        .await;
        assert_eq!(
            std::path::Path::new(ctx.workspace.as_ref().unwrap())
                .canonicalize()
                .unwrap(),
            workspace.canonicalize().unwrap()
        );
        // 项目会话覆盖执行目录，也不能覆盖 read_agent_config 的配置来源。
        ctx.workspace = Some(project.to_string_lossy().into_owned());
        let result = ReadAgentConfigTool
            .execute_with_context("{}", &ctx)
            .await
            .unwrap();
        let result: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(result["content"], "system_prompt: managed");
        assert_eq!(
            result["path"],
            dir.join("agent.yaml").to_string_lossy().as_ref()
        );
        // 新配置丢失时报告缺失，旧工作区配置不能复活。
        std::fs::remove_file(dir.join("agent.yaml")).unwrap();
        assert!(ReadAgentConfigTool
            .execute_with_context("{}", &ctx)
            .await
            .is_err());
        pool.close().await;
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn execute_without_context_returns_error() {
        let tool = ReadAgentConfigTool;
        let result = tool.execute("{}").await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("execute_with_context"));
    }

    #[test]
    fn has_correct_name_and_auth_level() {
        let tool = ReadAgentConfigTool;
        assert_eq!(tool.name(), "read_agent_config");
        assert!(matches!(
            tool.authorization_level(),
            AuthorizationLevel::Always
        ));
    }
}
