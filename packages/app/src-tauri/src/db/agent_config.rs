//! Agent 配置目录：出生时固定归属，工具 workspace 永不决定 YAML 读写位置。
//!
//! 存量迁移只复制原始字节、保留源文件，不生成配置（router/worker 可以仅有 DB
//! 配置）。目标冲突或 IO 失败阻止启动；文件完成后才登记 config_dir，可安全重跑。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use sqlx::{SqliteConnection, SqlitePool};

use crate::error::{AppError, AppResult};

/// 读取根目录，不调用 preferences::get_all（后者会创建用户目录）。
async fn managed_root(pool: &SqlitePool) -> AppResult<PathBuf> {
    let stored = super::repo::preferences::get(pool, "default_workspace_path").await?;
    let configured = stored.map(|value| serde_json::from_str::<String>(&value).unwrap_or(value));
    let root = match configured.filter(|value| !value.trim().is_empty()) {
        Some(path) => PathBuf::from(path),
        None => {
            let home = std::env::var_os("USERPROFILE")
                .or_else(|| std::env::var_os("HOME"))
                .ok_or_else(|| AppError::Validation(
                    "无法定位 Agent 配置目录：未配置 IcePaw 工作空间且用户主目录不可用。请配置工作空间后重试。".into(),
                ))?;
            PathBuf::from(home)
                .join("Documents")
                .join("icepaw-workspaces")
        }
    };
    if !root.is_absolute() {
        return Err(AppError::Validation(
            "无法定位 Agent 配置目录：IcePaw 工作空间不是绝对路径。请将工作空间设为完整路径后重试。".into(),
        ));
    }
    Ok(root)
}

fn directory_for(root: &Path, agent_id: &str) -> AppResult<PathBuf> {
    // 保留历史自定义 ID（中文、内部点等），只拒绝不安全的跨平台目录组件。
    // Windows 设备名即使带扩展名也保留，不能仅靠当前宿主 Path 解析判断。
    let reserved = agent_id
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    if agent_id.is_empty()
        || matches!(agent_id, "." | "..")
        || agent_id.ends_with(['.', ' '])
        || agent_id.chars().any(|c| {
            c.is_control() || matches!(c, '/' | '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*')
        })
        || matches!(reserved.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (reserved.len() == 4
            && (reserved.starts_with("COM") || reserved.starts_with("LPT"))
            && matches!(reserved.as_bytes()[3], b'1'..=b'9'))
    {
        return Err(AppError::Validation(format!(
            "无法定位 Agent 配置目录：Agent ID {agent_id:?} 不是安全的目录名。请修复该 Agent 标识后重试。"
        )));
    }
    let agents = root.join("agents");
    let dir = agents.join(agent_id);
    for path in [&agents, &dir] {
        match std::fs::symlink_metadata(path) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(AppError::Validation(format!(
                    "无法定位 Agent 配置目录：{} 是符号链接。请将托管目录恢复为普通目录后重试。",
                    path.display()
                )));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    debug_assert_eq!(dir.parent(), Some(agents.as_path()));
    Ok(dir)
}

/// 唯一托管路径计算入口；只读，不创建目录或改写偏好。
pub async fn managed_dir(pool: &SqlitePool, agent_id: &str) -> AppResult<PathBuf> {
    let dir = directory_for(&managed_root(pool).await?, agent_id)?;
    ensure_available(&mut *pool.acquire().await?, agent_id, &dir).await?;
    Ok(dir)
}

fn directory_identity(path: &Path) -> String {
    // 已存在目录解析真实位置（含根目录别名）；尚未创建则尽量解析父目录。
    let resolved = path
        .canonicalize()
        .or_else(|_| {
            path.parent()
                .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))?
                .canonicalize()
                .map(|parent| parent.join(path.file_name().unwrap_or_default()))
        })
        .unwrap_or_else(|_| path.to_path_buf());
    crate::infra::path_norm::auth_cache_key(&resolved.to_string_lossy())
}

/// 必须在最终登记事务内调用；managed_dir 的预检不能替代写入时的仲裁。
pub(crate) async fn ensure_available(
    connection: &mut SqliteConnection,
    agent_id: &str,
    dir: &Path,
) -> AppResult<()> {
    let occupied: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, config_dir FROM agents WHERE config_dir IS NOT NULL AND id <> ?",
    )
    .bind(agent_id)
    .fetch_all(connection)
    .await?;
    let identity = directory_identity(dir);
    if let Some((owner, _)) = occupied
        .into_iter()
        .find(|(_, path)| directory_identity(Path::new(path)) == identity)
    {
        return Err(AppError::Validation(format!(
            "Agent 配置目录不可用：{} 已归属于 Agent {owner}，不同 Agent 不能共用配置目录。请使用不同标识后重试。",
            dir.display()
        )));
    }
    Ok(())
}

fn optional_bytes(path: &Path) -> std::io::Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

#[derive(Debug, Clone, Copy)]
enum MigrationOutcome {
    Copied,
    AlreadyPresent,
    NoLegacyYaml,
}

impl MigrationOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Copied => "copied",
            Self::AlreadyPresent => "already_present",
            Self::NoLegacyYaml => "no_legacy_yaml",
        }
    }
}

fn prepare_config(workspace: Option<&str>, dir: &Path) -> AppResult<MigrationOutcome> {
    let source = match workspace.filter(|ws| !ws.is_empty()) {
        Some(ws) => {
            // 路径离线/不可访问不等于“没有 YAML”，不能登记空目录掩盖旧配置。
            let metadata = std::fs::metadata(ws)?;
            if !metadata.is_dir() {
                return Err(AppError::Validation(format!(
                    "旧 Agent 工作区不是目录：{ws}"
                )));
            }
            optional_bytes(&Path::new(ws).join("agent.yaml"))?
        }
        None => None,
    };
    let target = dir.join("agent.yaml");
    // 不跟随目标符号链接读取/写入其他位置的配置。
    match std::fs::symlink_metadata(&target) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(AppError::Validation(format!(
                "托管配置文件不能是符号链接：{}",
                target.display()
            )));
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let existing = optional_bytes(&target)?;
    match (source, existing) {
        (None, Some(_)) => Err(AppError::Validation(format!(
            "Agent 配置迁移未完成：旧 Agent 没有 YAML，但 {} 已存在。为避免采用无来源配置，文件已保留，请确认其归属后重试。",
            target.display()
        ))),
        (Some(source), Some(existing)) if source != existing => {
            Err(AppError::Validation(format!(
                "Agent 配置迁移未完成：{} 已存在且与旧配置不同。两个文件均已保留，请确认应保留的内容后重试。",
                target.display()
            )))
        }
        (Some(source), None) => {
            std::fs::create_dir_all(dir)?;
            // create_new 保证即使发生并发也不覆盖别人的目标。写完并同步才登记 DB；
            // 异常退出留下的未完成目标下次会报冲突，原配置始终保留可恢复。
            let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&target)?;
            if let Err(error) = file.write_all(&source).and_then(|()| file.sync_all()) {
                drop(file);
                let _ = std::fs::remove_file(&target);
                return Err(error.into());
            }
            Ok(MigrationOutcome::Copied)
        }
        (Some(_), Some(_)) => Ok(MigrationOutcome::AlreadyPresent),
        (None, None) => Ok(MigrationOutcome::NoLegacyYaml),
    }
}

/// 启动错误出口按此稳定前缀区分配置迁移失败与 WebView2 等其他错误。
pub(crate) const MIGRATION_FAILURE_PREFIX: &str = "Agent 配置迁移未完成：";

struct MigrationProgress {
    started: Instant,
    total: Option<usize>,
    succeeded: usize,
    copied: usize,
    already_present: usize,
    no_legacy_yaml: usize,
    stage: &'static str,
    agent_id: Option<String>,
    source_path: Option<PathBuf>,
    target_path: Option<PathBuf>,
}

impl MigrationProgress {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            total: None,
            succeeded: 0,
            copied: 0,
            already_present: 0,
            no_legacy_yaml: 0,
            stage: "scan_pending",
            agent_id: None,
            source_path: None,
            target_path: None,
        }
    }
}

async fn migrate_one(
    pool: &SqlitePool,
    root: &Path,
    id: &str,
    workspace: Option<String>,
    progress: &mut MigrationProgress,
) -> AppResult<MigrationOutcome> {
    progress.agent_id = Some(id.to_string());
    progress.source_path = workspace
        .as_deref()
        .filter(|ws| !ws.is_empty())
        .map(|ws| Path::new(ws).join("agent.yaml"));
    // 仅作失败诊断；真正用于文件操作的路径仍必须通过 directory_for 校验。
    progress.target_path = Some(root.join("agents").join(id).join("agent.yaml"));
    progress.stage = "resolve_directory";
    let dir = directory_for(root, id)?;
    progress.target_path = Some(dir.join("agent.yaml"));
    progress.stage = "begin_transaction";
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    progress.stage = "check_ownership";
    ensure_available(&mut tx, id, &dir).await?;
    progress.stage = "prepare_file";
    let output = dir.clone();
    let outcome =
        tokio::task::spawn_blocking(move || prepare_config(workspace.as_deref(), &output))
            .await
            .map_err(|e| AppError::Internal(format!("配置文件迁移任务未完成：{e}")))??;
    progress.stage = "register_directory";
    sqlx::query("UPDATE agents SET config_dir = ? WHERE id = ? AND config_dir IS NULL")
        .bind(dir.to_string_lossy().as_ref())
        .bind(id)
        .execute(&mut *tx)
        .await?;
    progress.stage = "commit";
    tx.commit().await?;
    Ok(outcome)
}

async fn migrate_pending(pool: &SqlitePool, progress: &mut MigrationProgress) -> AppResult<()> {
    let rows: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT id, workspace_path FROM agents WHERE config_dir IS NULL ORDER BY id",
    )
    .fetch_all(pool)
    .await?;
    progress.total = Some(rows.len());
    if rows.is_empty() {
        return Ok(());
    }
    tracing::info!(target: "ice_paw.agent_config", event = "started", total = rows.len(),
        "开始迁移 Agent 配置目录");
    progress.stage = "resolve_root";
    let root = managed_root(pool).await?;
    for (id, workspace) in rows {
        let agent_started = Instant::now();
        let outcome = migrate_one(pool, &root, &id, workspace, progress).await?;
        progress.succeeded += 1;
        match outcome {
            MigrationOutcome::Copied => progress.copied += 1,
            MigrationOutcome::AlreadyPresent => progress.already_present += 1,
            MigrationOutcome::NoLegacyYaml => progress.no_legacy_yaml += 1,
        }
        // 提交完成才记成功；日志不读取或记录 YAML、正文、凭据。
        tracing::info!(target: "ice_paw.agent_config", event = "agent_migrated",
            agent_id = %id, outcome = outcome.as_str(),
            source_path = %progress.source_path.as_deref().unwrap_or_else(|| Path::new("")).display(),
            target_path = %progress.target_path.as_deref().unwrap_or_else(|| Path::new("")).display(),
            succeeded = progress.succeeded, remaining = progress.total.unwrap_or(0) - progress.succeeded,
            elapsed_ms = agent_started.elapsed().as_millis() as u64,
            "Agent 配置目录已登记");
    }
    Ok(())
}

/// SQL migration 后、Agent 服务初始化前执行。每行登记一次，已登记目录永不重算。
pub async fn migrate_legacy_configs(pool: &SqlitePool) -> AppResult<()> {
    let mut progress = MigrationProgress::new();
    let result = migrate_pending(pool, &mut progress).await;
    let elapsed_ms = progress.started.elapsed().as_millis() as u64;
    let total = progress.total.unwrap_or(0);
    match result {
        Ok(()) if total == 0 => {
            tracing::info!(target: "ice_paw.agent_config", event = "skipped", reason = "no_pending",
                total = 0, succeeded = 0, remaining = 0, elapsed_ms,
                "Agent 配置迁移跳过：无待迁移项");
            Ok(())
        }
        Ok(()) => {
            tracing::info!(target: "ice_paw.agent_config", event = "completed", total,
                succeeded = progress.succeeded, remaining = 0, copied = progress.copied,
                already_present = progress.already_present, no_legacy_yaml = progress.no_legacy_yaml,
                elapsed_ms, "Agent 配置迁移完成");
            Ok(())
        }
        Err(error) => {
            let counts = match progress.total {
                Some(total) => format!(
                    "已完成 {} 个，待完成 {} 个。",
                    progress.succeeded,
                    total.saturating_sub(progress.succeeded)
                ),
                None => "待迁移数量尚未确定。".into(),
            };
            tracing::error!(target: "ice_paw.agent_config", event = "failed", stage = progress.stage,
                agent_id = progress.agent_id.as_deref().unwrap_or(""),
                source_path = %progress.source_path.as_deref().unwrap_or_else(|| Path::new("")).display(),
                target_path = %progress.target_path.as_deref().unwrap_or_else(|| Path::new("")).display(),
                reason = %error, total_known = progress.total.is_some(), total,
                succeeded = progress.succeeded, remaining = total.saturating_sub(progress.succeeded),
                copied = progress.copied, already_present = progress.already_present,
                no_legacy_yaml = progress.no_legacy_yaml, elapsed_ms,
                "Agent 配置迁移失败，未继续启动");
            Err(AppError::Validation(format!(
                "{MIGRATION_FAILURE_PREFIX}阶段 {}，Agent {}：{error}。\n源文件：{}\n目标文件：{}\n{counts}原配置已保留，请检查配置目录后重新启动。",
                progress.stage, progress.agent_id.as_deref().unwrap_or("（尚未定位）"),
                progress.source_path.as_deref().map(|p| p.display().to_string()).unwrap_or_else(|| "（无旧工作区或尚未定位）".into()),
                progress.target_path.as_deref().map(|p| p.display().to_string()).unwrap_or_else(|| "（尚未定位）".into()),
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{models::Agent, repo};
    use std::sync::{Arc, Mutex};
    use tracing::instrument::WithSubscriber;

    #[derive(Clone, Default)]
    struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

    impl Write for CapturedLogs {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    async fn capture_migration(pool: &SqlitePool) -> (AppResult<()>, String) {
        let logs = CapturedLogs::default();
        let writer = logs.clone();
        // subscriber 仅附着本次 future，不改全局默认值，不污染并行测试。
        let subscriber = tracing_subscriber::fmt()
            .with_env_filter("off,ice_paw.agent_config=info")
            .with_ansi(false)
            .without_time()
            .with_writer(move || writer.clone())
            .finish();
        let result = migrate_legacy_configs(pool)
            .with_subscriber(subscriber)
            .await;
        let text = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
        (result, text)
    }

    async fn legacy_named(pool: &SqlitePool, id: &str, workspace: Option<&Path>) {
        sqlx::query("INSERT INTO agents (id, name, provider, model, api_key_ref, workspace_path) VALUES (?, ?, 'mock', 'mock', ?, ?)")
            .bind(id).bind(id).bind(id).bind(workspace.map(|p| p.to_string_lossy().to_string()))
            .execute(pool).await.unwrap();
    }

    fn assert_migration_failure(result: AppResult<()>, logs: &str, stage: &str) {
        let AppError::Validation(message) = result.unwrap_err() else {
            panic!("migration failure must use the stable validation envelope");
        };
        assert!(message.starts_with(MIGRATION_FAILURE_PREFIX), "{message}");
        assert!(logs.contains("ERROR"), "{logs}");
        assert!(logs.contains("event=\"failed\""), "{logs}");
        assert!(logs.contains(&format!("stage=\"{stage}\"")), "{logs}");
        assert!(!logs.contains("event=\"completed\""), "{logs}");
    }

    struct Sandbox(PathBuf);
    impl Sandbox {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("icepaw-config-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&root).unwrap();
            Self(root)
        }
    }
    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    async fn pool(root: &Path) -> SqlitePool {
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
        pool
    }
    async fn legacy(pool: &SqlitePool, workspace: Option<&Path>) {
        sqlx::query("INSERT INTO agents (id, name, provider, model, api_key_ref, workspace_path) VALUES ('a1', 'a1', 'mock', 'mock', 'a1', ?)")
            .bind(workspace.map(|p| p.to_string_lossy().to_string())).execute(pool).await.unwrap();
    }

    #[tokio::test]
    async fn migration_logs_outcomes_counts_and_idempotent_skip_without_contents() {
        let sandbox = Sandbox::new();
        let root = sandbox.0.join("managed");
        let source = sandbox.0.join("source");
        std::fs::create_dir_all(&source).unwrap();
        let secret = "system_prompt: PRIVATE-MESSAGE-CONTENT\napi_key: PRIVATE-KEY-CONTENT\n";
        std::fs::write(source.join("agent.yaml"), secret).unwrap();
        let already = root.join("agents/b-already");
        std::fs::create_dir_all(&already).unwrap();
        std::fs::write(already.join("agent.yaml"), secret).unwrap();
        let pool = pool(&root).await;
        legacy_named(&pool, "a-copy", Some(&source)).await;
        legacy_named(&pool, "b-already", Some(&source)).await;
        legacy_named(&pool, "c-db-only", None).await;
        let (result, logs) = capture_migration(&pool).await;
        result.unwrap();
        assert_eq!(
            logs.matches("event=\"agent_migrated\"").count(),
            3,
            "{logs}"
        );
        assert!(logs.contains("event=\"started\" total=3"), "{logs}");
        for outcome in ["copied", "already_present", "no_legacy_yaml"] {
            assert!(logs.contains(&format!("outcome=\"{outcome}\"")), "{logs}");
        }
        let summary = logs
            .lines()
            .find(|line| line.contains("event=\"completed\""))
            .unwrap();
        for field in [
            "total=3",
            "succeeded=3",
            "remaining=0",
            "copied=1",
            "already_present=1",
            "no_legacy_yaml=1",
            "elapsed_ms=",
        ] {
            assert!(summary.contains(field), "{summary}");
        }
        assert!(
            !logs.contains("PRIVATE-"),
            "configuration contents leaked: {logs}"
        );
        assert!(!logs.contains("api_key"));
        let (result, logs) = capture_migration(&pool).await;
        result.unwrap();
        assert_eq!(logs.lines().count(), 1, "{logs}");
        assert!(
            logs.contains("event=\"skipped\" reason=\"no_pending\""),
            "{logs}"
        );
        assert!(!logs.contains("PRIVATE-"));
    }

    #[tokio::test]
    async fn migration_conflict_logs_partial_progress_and_paths_without_contents() {
        let sandbox = Sandbox::new();
        let root = sandbox.0.join("managed");
        let source = sandbox.0.join("source");
        let conflict = root.join("agents").join("z-conflict");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&conflict).unwrap();
        std::fs::write(source.join("agent.yaml"), "PRIVATE-SOURCE-CONTENT").unwrap();
        std::fs::write(conflict.join("agent.yaml"), "PRIVATE-TARGET-CONTENT").unwrap();
        let pool = pool(&root).await;
        legacy_named(&pool, "a-first", None).await;
        legacy_named(&pool, "z-conflict", Some(&source)).await;
        let (result, logs) = capture_migration(&pool).await;
        assert_migration_failure(result, &logs, "prepare_file");
        let failure = logs
            .lines()
            .find(|line| line.contains("event=\"failed\""))
            .unwrap();
        for field in [
            "agent_id=\"z-conflict\"",
            "total_known=true",
            "total=2",
            "succeeded=1",
            "remaining=1",
            "no_legacy_yaml=1",
            "reason=",
        ] {
            assert!(failure.contains(field), "{failure}");
        }
        assert!(
            failure.contains(source.join("agent.yaml").to_string_lossy().as_ref()),
            "{failure}"
        );
        assert!(
            failure.contains(conflict.join("agent.yaml").to_string_lossy().as_ref()),
            "{failure}"
        );
        assert!(!logs.contains("PRIVATE-"));
        assert!(repo::agent::get_by_id(&pool, "z-conflict")
            .await
            .unwrap()
            .config_dir
            .is_none());
    }

    #[tokio::test]
    async fn migration_scan_root_and_directory_failures_share_log_envelope() {
        let empty_pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let (result, logs) = capture_migration(&empty_pool).await;
        assert!(result
            .as_ref()
            .unwrap_err()
            .to_string()
            .contains("待迁移数量尚未确定"));
        assert_migration_failure(result, &logs, "scan_pending");
        assert!(logs.contains("total_known=false"), "{logs}");

        let pool = pool(Path::new("relative-invalid-root")).await;
        legacy_named(&pool, "a1", None).await;
        let (result, logs) = capture_migration(&pool).await;
        assert_migration_failure(result, &logs, "resolve_root");
        assert!(
            logs.contains("total_known=true total=1 succeeded=0 remaining=1"),
            "{logs}"
        );

        let sandbox = Sandbox::new();
        repo::preferences::set(
            &pool,
            "default_workspace_path",
            &sandbox.0.to_string_lossy(),
        )
        .await
        .unwrap();
        sqlx::query("UPDATE agents SET id = '../bad' WHERE id = 'a1'")
            .execute(&pool)
            .await
            .unwrap();
        let (result, logs) = capture_migration(&pool).await;
        assert_migration_failure(result, &logs, "resolve_directory");
        assert!(logs.contains("agent_id=\"../bad\""), "{logs}");
    }

    #[tokio::test]
    async fn migration_registration_failure_has_no_success_log_or_registration() {
        let sandbox = Sandbox::new();
        let pool = pool(&sandbox.0).await;
        legacy_named(&pool, "a1", None).await;
        sqlx::query("CREATE TRIGGER reject_config_registration BEFORE UPDATE OF config_dir ON agents BEGIN SELECT RAISE(ABORT, 'registration blocked for test'); END")
            .execute(&pool).await.unwrap();
        let (result, logs) = capture_migration(&pool).await;
        assert_migration_failure(result, &logs, "register_directory");
        assert!(logs.contains("succeeded=0 remaining=1"), "{logs}");
        assert!(!logs.contains("event=\"agent_migrated\""), "{logs}");
        assert!(repo::agent::get_by_id(&pool, "a1")
            .await
            .unwrap()
            .config_dir
            .is_none());
    }

    #[tokio::test]
    async fn migration_ownership_failure_is_logged_before_file_preparation() {
        let sandbox = Sandbox::new();
        let pool = pool(&sandbox.0).await;
        legacy_named(&pool, "owner", None).await;
        legacy_named(&pool, "a1", None).await;
        sqlx::query("UPDATE agents SET config_dir = ? WHERE id = 'owner'")
            .bind(sandbox.0.join("agents/a1").to_string_lossy().as_ref())
            .execute(&pool)
            .await
            .unwrap();
        let (result, logs) = capture_migration(&pool).await;
        assert_migration_failure(result, &logs, "check_ownership");
        assert!(logs.contains("agent_id=\"a1\""), "{logs}");
        assert!(logs.contains("total=1 succeeded=0 remaining=1"), "{logs}");
        assert!(!logs.contains("event=\"agent_migrated\""), "{logs}");
        assert!(!sandbox.0.join("agents/a1").exists());
    }

    #[tokio::test]
    async fn custom_workspace_migration_preserves_bytes_and_is_idempotent() {
        let sandbox = Sandbox::new();
        let root = sandbox.0.join("managed");
        let workspace = sandbox.0.join("project");
        std::fs::create_dir_all(&workspace).unwrap();
        let bytes = b"\xef\xbb\xbf# original\r\nsystem_prompt: retained\r\n";
        std::fs::write(workspace.join("agent.yaml"), bytes).unwrap();
        let pool = pool(&root).await;
        legacy(&pool, Some(&workspace)).await;
        migrate_legacy_configs(&pool).await.unwrap();
        let row = repo::agent::get_by_id(&pool, "a1").await.unwrap();
        let dir = root.join("agents").join("a1");
        assert_eq!(
            row.config_dir.as_deref(),
            Some(dir.to_string_lossy().as_ref())
        );
        assert_eq!(std::fs::read(dir.join("agent.yaml")).unwrap(), bytes);
        assert_eq!(std::fs::read(workspace.join("agent.yaml")).unwrap(), bytes);
        // 偏好与旧文件后续变化不能移动/覆盖已登记的配置。
        repo::preferences::set(
            &pool,
            "default_workspace_path",
            &sandbox.0.join("new-root").to_string_lossy(),
        )
        .await
        .unwrap();
        std::fs::write(workspace.join("agent.yaml"), "system_prompt: untrusted").unwrap();
        migrate_legacy_configs(&pool).await.unwrap();
        assert_eq!(
            repo::agent::get_by_id(&pool, "a1")
                .await
                .unwrap()
                .config_dir,
            row.config_dir
        );
        assert_eq!(std::fs::read(dir.join("agent.yaml")).unwrap(), bytes);
        assert_eq!(
            row.load_file_config().unwrap().system_prompt.as_deref(),
            Some("retained")
        );
        assert_eq!(
            Agent::from_row_with_file_config(row).system_prompt,
            "retained"
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn migration_rejects_case_aliases_even_with_identical_yaml() {
        let sandbox = Sandbox::new();
        let root = sandbox.0.join("managed");
        let source = sandbox.0.join("source");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("agent.yaml"), "system_prompt: same").unwrap();
        let pool = pool(&root).await;
        for id in ["Editor", "editor"] {
            sqlx::query("INSERT INTO agents (id, name, provider, model, api_key_ref, workspace_path) VALUES (?, ?, 'mock', 'mock', ?, ?)")
                .bind(id).bind(id).bind(id).bind(source.to_string_lossy().as_ref()).execute(&pool).await.unwrap();
        }
        assert!(migrate_legacy_configs(&pool).await.is_err());
        let registered: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM agents WHERE config_dir IS NOT NULL")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            registered, 1,
            "两个 Agent 不可共享同一个配置目录，即使 YAML 内容相同"
        );
        assert_eq!(
            std::fs::read_to_string(source.join("agent.yaml")).unwrap(),
            "system_prompt: same"
        );
    }

    #[tokio::test]
    async fn conflicting_target_is_not_overwritten_or_registered() {
        let sandbox = Sandbox::new();
        let workspace = sandbox.0.join("project");
        let root = sandbox.0.join("managed");
        let target = root.join("agents").join("a1");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(workspace.join("agent.yaml"), "source").unwrap();
        std::fs::write(target.join("agent.yaml"), "target").unwrap();
        let pool = pool(&root).await;
        legacy(&pool, Some(&workspace)).await;
        assert!(migrate_legacy_configs(&pool).await.is_err());
        assert_eq!(
            std::fs::read_to_string(target.join("agent.yaml")).unwrap(),
            "target"
        );
        assert_eq!(
            std::fs::read_to_string(workspace.join("agent.yaml")).unwrap(),
            "source"
        );
        assert!(repo::agent::get_by_id(&pool, "a1")
            .await
            .unwrap()
            .config_dir
            .is_none());
        // 文件复制成功、DB 未登记的中断窗口：同字节目标允许重跑完成。
        std::fs::write(target.join("agent.yaml"), "source").unwrap();
        migrate_legacy_configs(&pool).await.unwrap();
        assert!(repo::agent::get_by_id(&pool, "a1")
            .await
            .unwrap()
            .config_dir
            .is_some());
    }

    #[tokio::test]
    async fn missing_legacy_workspace_is_an_error_not_config_loss() {
        let sandbox = Sandbox::new();
        let pool = pool(&sandbox.0).await;
        legacy(&pool, Some(&sandbox.0.join("offline"))).await;
        assert!(migrate_legacy_configs(&pool).await.is_err());
        assert!(repo::agent::get_by_id(&pool, "a1")
            .await
            .unwrap()
            .config_dir
            .is_none());
    }

    #[tokio::test]
    async fn db_only_agent_does_not_adopt_unrelated_existing_yaml() {
        let sandbox = Sandbox::new();
        let target = sandbox.0.join("agents").join("a1");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("agent.yaml"), "system_prompt: unrelated").unwrap();
        let pool = pool(&sandbox.0).await;
        legacy(&pool, None).await;
        assert!(migrate_legacy_configs(&pool).await.is_err());
        assert!(repo::agent::get_by_id(&pool, "a1")
            .await
            .unwrap()
            .config_dir
            .is_none());
        assert_eq!(
            std::fs::read_to_string(target.join("agent.yaml")).unwrap(),
            "system_prompt: unrelated"
        );
    }

    #[tokio::test]
    async fn db_only_agent_does_not_gain_yaml_or_read_workspace_yaml() {
        let sandbox = Sandbox::new();
        let pool = pool(&sandbox.0).await;
        legacy(&pool, None).await;
        migrate_legacy_configs(&pool).await.unwrap();
        let row = repo::agent::get_by_id(&pool, "a1").await.unwrap();
        assert!(row.config_dir.is_some());
        assert!(!sandbox.0.join("agents/a1/agent.yaml").exists());
        let workspace = sandbox.0.join("project");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(workspace.join("agent.yaml"), "system_prompt: untrusted").unwrap();
        let mut row = row;
        row.workspace_path = Some(workspace.to_string_lossy().to_string());
        assert!(row.load_file_config().is_none());
        row.config_dir = None;
        assert!(row.load_file_config().is_none());
        assert!(!Agent::from_row_with_file_config(row).config_from_file);
    }

    #[test]
    fn ids_cannot_escape_managed_agents_directory() {
        let root = std::env::temp_dir();
        for bad in [
            "", ".", "..", "../other", "a/b", "a\\b", "C:agent", "CON", "LPT1", "CON.txt",
            "unsafe.", "unsafe ", "a?b",
        ] {
            assert!(directory_for(&root, bad).is_err(), "accepted {bad:?}");
        }
        assert_eq!(
            directory_for(&root, "a1-safe_id").unwrap(),
            root.join("agents/a1-safe_id")
        );
        assert_eq!(
            directory_for(&root, "我的 Agent.v2").unwrap(),
            root.join("agents/我的 Agent.v2")
        );
    }
}
