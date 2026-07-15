//! 预置助手（Preset Agents）自动植入
//!
//! 启动时从 `~/.openclaw/openclaw.json` 读取 `models.providers.<x>.apiKey`，
//! 对每个预置助手（DeepSeek Flash / MiniMax M3）：
//!   - 若 agents 表里已存在同 (provider, model) → 跳过
//!   - 否则：`crypto::store_api_key()` 写入 Stronghold + `repo::agent::create()` 落 DB
//!
//! 设计要点：
//!   - **幂等**：每次启动都跑，已存在的不会重复创建（匹配 (provider, model)）。
//!   - **容错**：openclaw.json 缺失 / 解析失败 / 单个预设失败 → 不阻塞 App 启动，
//!     失败信息进 `SeedResult.errors`，由前端或日志展示。
//!   - **解耦**：核心逻辑 `seed_preset_agents_impl` 接收 `&AppHandle, &SqlitePool`，
//!     同时被 Tauri command（前端调用）和 setup 钩子（启动自动跑）复用。

use std::path::PathBuf;

use serde::Serialize;
use sqlx::SqlitePool;
use tauri::AppHandle;
use tracing::{info, warn};
use uuid::Uuid;

use crate::crypto;
use crate::db::models::NewAgent;
use crate::db::repo;
use crate::error::AppResult;

// =========================================================================
// 对外返回结构
// =========================================================================

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeedResult {
    /// 新建的预设助手列表
    pub created: Vec<SeedEntry>,
    /// 已存在（按 provider+model 去重命中）而跳过的列表
    pub skipped: Vec<SeedEntry>,
    /// 单个预设创建过程中的非致命错误（不影响 App 启动）
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeedEntry {
    pub name: String,
    pub provider: String,
    pub model: String,
}

// =========================================================================
// Tauri command 入口
// =========================================================================

/// 植入预置助手
///
/// 可被前端手动调用（用于"重置/重新植入"功能），启动时 setup 钩子也会自动跑。
/// 总是返回 `Ok(SeedResult)`；任何失败都收进 `errors` 字段，不让 IPC 抛错。
#[tauri::command]
pub async fn seed_preset_agents(
    app: AppHandle,
    state: tauri::State<'_, SqlitePool>,
) -> AppResult<SeedResult> {
    let pool = state.inner().clone();
    Ok(seed_preset_agents_impl(&app, &pool).await)
}

// =========================================================================
// 核心实现
// =========================================================================

/// 内部幂等逻辑：被 Tauri command 和 `lib::run` 的 setup 钩子共用
///
/// 返回值即最终对外结构。**总是返回 `Ok`**，确保不会因预设植入问题
/// 阻塞 App 启动。
pub async fn seed_preset_agents_impl(
    app: &AppHandle,
    pool: &SqlitePool,
) -> SeedResult {
    let mut result = SeedResult {
        created: Vec::new(),
        skipped: Vec::new(),
        errors: Vec::new(),
    };

    // 1) 读 openclaw.json
    let cfg = match load_openclaw_config() {
        Ok(c) => c,
        Err(reason) => {
            // 配置文件不可用 = 没有任何预设要植入，App 启动不受影响
            info!(
                target: "ice_paw.seed",
                "跳过 preset agents 植入: {}",
                reason
            );
            return result;
        }
    };

    // 2) 根据 cfg 拼出 Preset 列表（缺 apiKey 的 provider 静默跳过）
    let presets = build_presets(&cfg);
    if presets.is_empty() {
        info!(
            target: "ice_paw.seed",
            "openclaw.json 中没有任何 provider 带 apiKey，无预设可植入"
        );
        return result;
    }

    // 3) 已有 agent 列表（小表，全表扫足够；O(N) 简洁优于专门的 SQL）
    //    若 list 失败则放弃本次 seed（DB 应该有严重问题），让上层决定是否 abort
    let existing = match repo::agent::list(pool).await {
        Ok(rows) => rows,
        Err(e) => {
            warn!(
                target: "ice_paw.seed",
                "list agents 失败，跳过 preset 植入: {e}"
            );
            result
                .errors
                .push(format!("list agents 失败: {e}"));
            return result;
        }
    };

    // 4) 逐个 preset 处理
    for preset in presets {
        let already_exists = existing
            .iter()
            .any(|r| r.provider == preset.provider && r.model == preset.model);

        if already_exists {
            info!(
                target: "ice_paw.seed",
                "已存在，跳过: {} ({}/{})",
                preset.name, preset.provider, preset.model
            );
            result.skipped.push(SeedEntry {
                name: preset.name.to_string(),
                provider: preset.provider.to_string(),
                model: preset.model.to_string(),
            });
            continue;
        }

        match create_one(app, pool, &preset).await {
            Ok(()) => {
                info!(
                    target: "ice_paw.seed",
                    "已创建: {} ({}/{})",
                    preset.name, preset.provider, preset.model
                );
                result.created.push(SeedEntry {
                    name: preset.name.to_string(),
                    provider: preset.provider.to_string(),
                    model: preset.model.to_string(),
                });
            }
            Err(e) => {
                let msg = format!(
                    "{} ({}/{}): {}",
                    preset.name, preset.provider, preset.model, e
                );
                warn!(target: "ice_paw.seed", "创建失败: {msg}");
                result.errors.push(msg);
            }
        }
    }
    result
}

/// 创建一个预设助手（store_api_key + repo::agent::create）
async fn create_one(app: &AppHandle, pool: &SqlitePool, p: &Preset) -> AppResult<()> {
    let id = Uuid::new_v4().to_string();
    // stronghold 引用 key = agent_id（M2 约定：api_key_ref 与 id 同值）
    let base_url_ref = p.base_url.as_str();
    crypto::store_api_key(app, &id, &p.api_key, Some(base_url_ref))?;

    let input = NewAgent {
        name: p.name.to_string(),
        provider: p.provider.to_string(),
        model: p.model.to_string(),
        system_prompt: p.system_prompt.to_string(),
        api_key: p.api_key.clone(),
        base_url: Some(p.base_url.clone()),
        temperature: p.temperature,
        max_tokens: p.max_tokens,
        extra_params: None,
        sort_order: 0,
        cache_prompt: true,
    };
    let _row = repo::agent::create(pool, &input, &id, &id).await?;
    Ok(())
}

// =========================================================================
// 内部结构 & 配置加载
// =========================================================================

/// 单个预设助手的内存表示
///
/// `name` / `provider` / `model` / `system_prompt` 全部来自硬编码模板，用
/// `&'static str` 零分配。`base_url` / `api_key` 来自 openclaw.json（运行时
/// 解析得到），必须是 `String`。
struct Preset {
    name: &'static str,
    provider: &'static str,
    model: &'static str,
    base_url: String,
    api_key: String,
    system_prompt: &'static str,
    temperature: f64,
    max_tokens: i32,
}

/// 加载 `~/.openclaw/openclaw.json` 解析为 `serde_json::Value`
///
/// 失败原因（"file missing" / "not json" / "home dir unknown"）以 String
/// 形式回传，调用方把它当 info 日志打印即可，**不会上升为错误**。
fn load_openclaw_config() -> Result<serde_json::Value, String> {
    let path = openclaw_config_path()
        .ok_or_else(|| "找不到 home 目录（$HOME / %USERPROFILE%）".to_string())?;
    if !path.exists() {
        return Err(format!("配置文件不存在: {}", path.display()));
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("读取 {} 失败: {e}", path.display()))?;
    serde_json::from_str(&text)
        .map_err(|e| format!("解析 openclaw.json 失败: {e}"))
}

/// 解析 `~/.openclaw/openclaw.json` 的绝对路径
///
/// 优先级与 `commands::chat_context::get_home_dir` 一致：
///   1. Windows: `%USERPROFILE%`
///   2. Unix (macOS/Linux): `$HOME`
///
/// 不引入 `dirs` crate，理由：
///   - 整个项目目前没有 `dirs` 依赖
///   - 现有 `chat_context::get_home_dir` 用的就是 env-var 方案，保持一致
fn openclaw_config_path() -> Option<PathBuf> {
    let home = std::env::var("USERPROFILE")
        .ok()
        .filter(|p| !p.is_empty())
        .or_else(|| std::env::var("HOME").ok().filter(|p| !p.is_empty()))?;
    Some(PathBuf::from(home).join(".openclaw").join("openclaw.json"))
}

/// 把 `openclaw.json` 的 providers 映射成 `Preset` 列表
///
/// 每个 provider 的预设模板（name/model/system_prompt 等）硬编码在这里。
/// 任何 provider 缺 `apiKey` 会被静默跳过（不打 warning：
/// 用户可能就没装那个 provider，缺 key 是正常情况）。
fn build_presets(cfg: &serde_json::Value) -> Vec<Preset> {
    let mut out = Vec::new();

    let providers = match cfg
        .get("models")
        .and_then(|m| m.get("providers"))
        .and_then(|p| p.as_object())
    {
        Some(p) => p,
        None => return out,
    };

    // --- DeepSeek Flash ---
    if let Some(preset) = build_deepseek_flash(providers.get("deepseek")) {
        out.push(preset);
    }

    // --- MiniMax M3 ---
    if let Some(preset) = build_minimax_m3(providers.get("minimax-cn")) {
        out.push(preset);
    }

    out
}

fn build_deepseek_flash(p: Option<&serde_json::Value>) -> Option<Preset> {
    let p = p?;
    let api_key = p.get("apiKey").and_then(|v| v.as_str())?;
    // baseUrl 可选；缺则用预设默认值
    let base_url = p
        .get("baseUrl")
        .and_then(|v| v.as_str())
        .unwrap_or("https://api.deepseek.com");
    Some(Preset {
        name: "DeepSeek Flash",
        provider: "DeepSeek",
        model: "deepseek-v4-flash",
        base_url: base_url.to_string(),
        api_key: api_key.to_string(),
        system_prompt: "你是一个高效的AI助手，擅长快速回答问题和处理日常任务。",
        temperature: 0.7,
        max_tokens: 4096,
    })
}

fn build_minimax_m3(p: Option<&serde_json::Value>) -> Option<Preset> {
    let p = p?;
    let api_key = p.get("apiKey").and_then(|v| v.as_str())?;
    let base_url = p
        .get("baseUrl")
        .and_then(|v| v.as_str())
        .unwrap_or("https://api.minimaxi.com/anthropic");
    Some(Preset {
        name: "MiniMax M3",
        provider: "MiniMax",
        model: "minimax-cn/M3",
        base_url: base_url.to_string(),
        api_key: api_key.to_string(),
        system_prompt:
            "你是一个全能的AI助手，具备强大的语言理解和生成能力，可以处理各类对话任务。",
        temperature: 0.7,
        max_tokens: 4096,
    })
}

// =========================================================================
// Tests
// =========================================================================
//
// 本次 Sprint 暂不新增单测，保持 `cargo test --lib` 仍为 98 passed。
// build_presets 是纯函数，未来需要时可单独补 cfg(test) 模块；
// store_api_key / repo::agent::create 的覆盖已由其它模块提供。

