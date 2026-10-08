//! MCP server 凭据槽位化（2026-10-08 P1 体检收敛）
//!
//! env/headers 的**值**存 Stronghold 槽位 `mcpserver:{id}`（密文 JSON
//! `{"env": {...}, "headers": {...}}`）；DB `env`/`headers` 列只存**哨兵清单**
//! （`{"KEY": "__SLOT__"}`，键名保留供前端显示「已配置 N 项」）——库文件或
//! list 命令回传被拿走都取不到凭据（治「明文落库 + 原样回显 webview」双面）。
//! 镜像 ModelProfile `profile:{id}` 模式。
//!
//! ## 字段形态三态（判据函数）
//! - 空对象 `{}`：无凭据（bundled server 天然如此），无需槽位
//! - 清单形态：有键且值全为 `__SLOT__` → 已外置，实值在槽位
//! - 实值形态：存在非哨兵值 → 未迁移/直写（读侧兼容直用，boot 迁移收编）
//!
//! ## 读写路径
//! - 写：`mcp_cmd` create/update 脱水（实值入槽位 + DB 落清单）；
//!   update 的 `None` 字段先 `resolve_value` 读当前实值再合成完整槽位
//! - 读（spawn 单点）：`McpServerManager::start_server` 入口 `hydrate_into`——
//!   覆盖 boot 启动 / cmd 重启 / lazy_restart / retry 全路径；list/get 命令与
//!   entries 展示**不**水合（清单天然干）
//! - 迁移：boot MCP 启动任务内 `migrate_plaintext_rows`（幂等行级）
//!
//! 用户真想设字面量 `"__SLOT__"` 值的概率为零——哨兵不复用为用户值。

use serde_json::{json, Value};
use sqlx::SqlitePool;
use tauri::AppHandle;

use crate::error::{AppError, AppResult};
use crate::harness::mcp::types::McpServerConfig;

pub(crate) const SLOT_PREFIX: &str = "mcpserver:";
pub(crate) const SLOT_SENTINEL: &str = "__SLOT__";

pub(crate) fn slot_of(server_id: &str) -> String {
    format!("{SLOT_PREFIX}{server_id}")
}

/// 清单形态：有键且值全为哨兵（已外置标记）
pub(crate) fn is_manifest(v: &Value) -> bool {
    match v.as_object() {
        Some(m) if !m.is_empty() => m.values().all(|x| x.as_str() == Some(SLOT_SENTINEL)),
        _ => false,
    }
}

/// 实值形态：存在非哨兵值（需外置 / 可直接使用）
pub(crate) fn has_plaintext(v: &Value) -> bool {
    match v.as_object() {
        Some(m) if !m.is_empty() => m.values().any(|x| x.as_str() != Some(SLOT_SENTINEL)),
        _ => false,
    }
}

/// 实值 → 哨兵清单（键名保留）
pub(crate) fn to_manifest(v: &Value) -> Value {
    match v.as_object() {
        Some(m) => {
            let out: serde_json::Map<String, Value> = m
                .iter()
                .map(|(k, _)| (k.clone(), json!(SLOT_SENTINEL)))
                .collect();
            Value::Object(out)
        }
        None => Value::Object(serde_json::Map::new()),
    }
}

/// 完整 `{env, headers}` 实值写入槽位（空字段存空对象——hydrate 对空对象
/// 跳过，语义一致）。
pub(crate) fn store_secrets(
    app: &AppHandle,
    server_id: &str,
    env: &Value,
    headers: &Value,
) -> AppResult<()> {
    let payload = json!({
        "env": env.as_object().cloned().unwrap_or_default(),
        "headers": headers.as_object().cloned().unwrap_or_default(),
    });
    crate::crypto::store_slot_json(app, &slot_of(server_id), &payload)
}

/// 解析某字段当前生效实值（update 读-改-写用）：实值/空对象 → 原样；
/// 清单 → 槽位取（缺失 → Err 引导重填）。
pub(crate) fn resolve_value(
    app: &AppHandle,
    server_id: &str,
    field: &str,
    v: &Value,
) -> AppResult<Value> {
    if !is_manifest(v) {
        return Ok(v.clone());
    }
    match crate::crypto::fetch_slot_json(app, &slot_of(server_id))? {
        Some(secrets) => Ok(secrets.get(field).cloned().unwrap_or_else(|| json!({}))),
        None => Err(AppError::Validation(
            "该 Server 的凭据实值已不在安全存储中（槽位缺失）。请在表单中重新填写环境变量/请求头后保存。".into(),
        )),
    }
}

/// spawn 前把 config 的清单形态字段还原为槽位实值（就地改写）。
///
/// - 两字段全空 → 无凭据，跳过（bundled 不碰 Stronghold）
/// - 清单字段 → 槽位取实值；槽位缺失 = 值不可恢复 → **Err**（宁启动失败，
///   勿把哨兵串发出去污染请求）
/// - 实值字段 → 未迁移态直用（兼容）+ warn
pub(crate) fn hydrate_into(app: &AppHandle, config: &mut McpServerConfig) -> AppResult<()> {
    if !config.env.is_object() {
        config.env = json!({});
    }
    if !config.headers.is_object() {
        config.headers = json!({});
    }
    let env_manifest = is_manifest(&config.env);
    let headers_manifest = is_manifest(&config.headers);
    if !env_manifest && !headers_manifest {
        if has_plaintext(&config.env) || has_plaintext(&config.headers) {
            tracing::warn!(
                target: "ice_paw.mcp",
                server = %config.name,
                "env/headers 仍为实值形态（未外置），spawn 直用——boot 迁移未跑到或失败，下次启动收编"
            );
        }
        return Ok(());
    }
    match crate::crypto::fetch_slot_json(app, &slot_of(&config.id)) {
        Ok(Some(secrets)) => {
            if env_manifest {
                config.env = secrets.get("env").cloned().unwrap_or_else(|| json!({}));
            }
            if headers_manifest {
                config.headers = secrets.get("headers").cloned().unwrap_or_else(|| json!({}));
            }
            Ok(())
        }
        Ok(None) => Err(AppError::Internal(
            "MCP Server 凭据槽位缺失: 环境变量/请求头的实值不在安全存储中且无法恢复。请在 设置 → MCP/工具集 打开该 Server 重新填写并保存。".into(),
        )),
        Err(e) => Err(e),
    }
}

/// boot 存量收编（幂等，行级）：env/headers 含实值的行 → 值入槽位 + DB 改
/// 哨兵清单。槽位写失败 → DB 保持实值（老路径可用）下次重放；两步都幂等
/// （槽位同值覆盖；DB 已清单则跳过）。须在 MCP 启动之前跑（spawn 即需实值）。
pub(crate) async fn migrate_plaintext_rows(app: &AppHandle, pool: &SqlitePool) {
    let configs = match crate::db::repo::mcp_server::list_all(pool).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(target: "ice_paw.mcp", "凭据槽位化迁移：读配置失败: {e}");
            return;
        }
    };
    for cfg in configs {
        let env_plain = has_plaintext(&cfg.env);
        let headers_plain = has_plaintext(&cfg.headers);
        if !env_plain && !headers_plain {
            continue;
        }
        // 读-改-写：保留槽位内已外置字段的既有实值（混合形态只补实值字段）
        let mut secrets = crate::crypto::fetch_slot_json(app, &slot_of(&cfg.id))
            .unwrap_or_else(|e| {
                tracing::warn!(target: "ice_paw.mcp", "读槽位 {} 失败（按空处理）: {e}", cfg.id);
                None
            })
            .unwrap_or_else(|| json!({}));
        if env_plain {
            secrets["env"] = cfg.env.clone();
        }
        if headers_plain {
            secrets["headers"] = cfg.headers.clone();
        }
        if let Err(e) = store_secrets(app, &cfg.id, &secrets["env"], &secrets["headers"]) {
            tracing::warn!(
                target: "ice_paw.mcp",
                server = %cfg.name,
                "凭据槽位化迁移：槽位写失败（DB 保持实值，下次启动重放）: {e}"
            );
            continue;
        }
        let new_env = if env_plain {
            to_manifest(&cfg.env)
        } else {
            cfg.env.clone()
        };
        let new_headers = if headers_plain {
            to_manifest(&cfg.headers)
        } else {
            cfg.headers.clone()
        };
        if let Err(e) =
            crate::db::repo::mcp_server::update_secrets_manifest(pool, &cfg.id, &new_env, &new_headers)
                .await
        {
            tracing::warn!(
                target: "ice_paw.mcp",
                server = %cfg.name,
                "凭据槽位化迁移：DB 清单化失败（槽位已有值，下次启动重放）: {e}"
            );
        } else {
            tracing::info!(
                target: "ice_paw.mcp",
                server = %cfg.name,
                "凭据已外置到 Stronghold 槽位（env 明文={} headers 明文={}）",
                env_plain,
                headers_plain
            );
        }
    }
}

// =========================================================================
// 单测
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_shape_predicates() {
        let empty = json!({});
        let manifest = json!({"Authorization": "__SLOT__", "GLM_KEY": "__SLOT__"});
        let plain = json!({"Authorization": "Bearer real"});
        let mixed = json!({"A": "__SLOT__", "B": "real"});

        assert!(!is_manifest(&empty) && !has_plaintext(&empty));
        assert!(is_manifest(&manifest) && !has_plaintext(&manifest));
        assert!(!is_manifest(&plain) && has_plaintext(&plain));
        // 混合形态按实值处理（迁移收编后才会出现纯清单）
        assert!(!is_manifest(&mixed) && has_plaintext(&mixed));
    }

    #[test]
    fn to_manifest_keeps_keys_masks_values() {
        let plain = json!({"Authorization": "Bearer sk-secret", "X-Key": "abc"});
        let m = to_manifest(&plain);
        assert_eq!(m["Authorization"], json!(SLOT_SENTINEL));
        assert_eq!(m["X-Key"], json!(SLOT_SENTINEL));
        // 键名保留（前端显示「已配置 2 项」的数据源）
        assert_eq!(m.as_object().unwrap().len(), 2);
    }

    #[test]
    fn slot_key_format() {
        assert_eq!(slot_of("srv-1"), "mcpserver:srv-1");
    }
}
