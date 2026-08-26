//! `commands::agent_yaml` — agent.yaml 定向改写（预算标量 + system_prompt 块）
//!
//! 背景：yaml 覆盖 DB（`AgentFileConfig::apply_to`），AgentForm 想暴露
//! `max_total_tokens` / `tool_max_rounds` 就必须直写 yaml 文件——而 agent.yaml
//! 里有用户的中文注释、system_prompt 块值、CRLF/BOM，常规「反序列化→改→再
//! 序列化」会把它们全部洗掉。故做**逐行补丁**而非往返序列化：
//!
//! - 只匹配**列 0 的顶层标量键**（缩进子键 / 注释行 / 相似前缀键均不误伤）
//! - 其余行逐字节保留（`split_inclusive('\n')` 保行尾，BOM 单独摘出回填）
//! - 写前三道安全闸（白名单 / 重解析 / 目标字段回读），任一不过**文件不动**
//! - 原子写：同目录 `.tmp` → rename
//!
//! 语义对齐 B1：显式值 = 硬上限（触顶即停、不自动续期）；注释掉 = 恢复
//! 按上下文窗口自适应 3× + 自动续期 2 次。前端「高级设置」据此把
//! 「空 = 默认自适应」做成一等选项。
//!
//! 2026-08-23 增多行块通道 [`patch_agent_yaml_block`] + [`set_agent_system_prompt`]：
//! 「风格预设」三档（前端素材）整块写入 `system_prompt: |`——标量补丁装不下
//! 多行文本，两函数同款纪律、各自命令入口。

use std::sync::Arc;

use tauri::State;

use super::agent_cmd::AgentCmd;
use crate::db::models::AgentFileConfig;
use crate::error::{AppError, AppResult};

/// 可改写键白名单（新增键须同步 `validate_patched` 的回读分支）
const WRITABLE_FIELDS: &[&str] = &["max_total_tokens", "tool_max_rounds"];

/// 改写动作：设值 / 注释掉（注释掉 = 恢复默认自适应 + 自动续期）
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum YamlPatchAction {
    Set(u64),
    CommentOut,
}

/// 字段快照（读命令返回 / 写命令成功后回显）
#[derive(Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct AgentYamlFields {
    pub max_total_tokens: Option<u64>,
    pub tool_max_rounds: Option<u64>,
    /// 现有 system_prompt——前端「风格预设」覆盖确认的判据（2026-08-23 加入）
    pub system_prompt: Option<String>,
    /// 现有 Word 样式偏好块——D12 双轨承载（提案卡显示 / 摘除判据）
    pub word_style_profile: Option<String>,
}

impl AgentYamlFields {
    fn from_config(cfg: &AgentFileConfig) -> Self {
        Self {
            max_total_tokens: cfg.max_total_tokens.map(|v| v as u64),
            tool_max_rounds: cfg.tool_max_rounds.map(|v| v as u64),
            system_prompt: cfg.system_prompt.clone(),
            word_style_profile: cfg.word_style_profile.clone(),
        }
    }
}

/// 列 0 顶层标量键匹配：行首即 key 且其后紧跟 `:`。
/// `max_total_tokens_x:`（前缀相似）、`  max_total_tokens:`（缩进子键）、
/// `# max_total_tokens:`（注释行）均不匹配。
fn line_is_active_key(line: &str, key: &str) -> bool {
    line.strip_prefix(key)
        .is_some_and(|rest| rest.starts_with(':'))
}

/// 行尾（`\r\n` / `\n` / 文件末行无换行），Set 替换行时原样保留
fn line_end(line: &str) -> &str {
    if line.ends_with("\r\n") {
        "\r\n"
    } else if line.ends_with('\n') {
        "\n"
    } else {
        ""
    }
}

/// 逐行补丁 agent.yaml（纯函数，不碰文件系统）。
///
/// - `Set`：目标行整行替换为 `key: value`（行内旧注释随行丢弃，行尾保留）；
///   无目标行则**追加**（与既有内容之间恰好一个换行）
/// - `CommentOut`：目标行前缀 `# `（行内注释原样保留）；无目标行 = no-op
/// - 只替换**首个**匹配行；若文件本就有重复键，补丁后重解析闸会报错兜底
pub fn patch_agent_yaml(content: &str, key: &str, action: &YamlPatchAction) -> String {
    // BOM 单独摘出：不参与匹配、输出时原样回填（首行键在列 0 也可命中）
    let (bom, body) = match content.strip_prefix('\u{FEFF}') {
        Some(rest) => ("\u{FEFF}", rest),
        None => ("", content),
    };

    let mut out = String::with_capacity(content.len() + key.len() + 16);
    let mut found = false;
    for line in body.split_inclusive('\n') {
        if !found && line_is_active_key(line, key) {
            found = true;
            match action {
                YamlPatchAction::Set(v) => {
                    out.push_str(key);
                    out.push_str(": ");
                    out.push_str(&v.to_string());
                    out.push_str(line_end(line));
                }
                YamlPatchAction::CommentOut => {
                    out.push_str("# ");
                    out.push_str(line);
                }
            }
        } else {
            out.push_str(line);
        }
    }
    if !found {
        if let YamlPatchAction::Set(v) = action {
            if !body.is_empty() && !body.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(key);
            out.push_str(": ");
            out.push_str(&v.to_string());
            out.push('\n');
        }
    }
    format!("{bom}{out}")
}

/// 多行块键整体替换（`key: |` + 缩进块体）——「风格预设」写 `system_prompt` 用
/// （2026-08-23，docs/agent-prompt-draft.md）。
///
/// 与 [`patch_agent_yaml`] 同款逐行补丁纪律（BOM 摘出回填、非目标行逐字节保留），
/// 差异在目标键的值占多行：目标键行 + 其后所有「缩进 >0 或纯空白」行（旧块体，
/// 兼容 `|`/`>`/行内标量三种既有形态）整段替换为 2 空格缩进的新文本；无目标键
/// 则追加。块内空行零缩进合法，原样输出；生成行沿用文件既有行尾风格（CRLF 不混）。
pub fn patch_agent_yaml_block(content: &str, key: &str, text: &str) -> String {
    let (bom, body) = match content.strip_prefix('\u{FEFF}') {
        Some(rest) => ("\u{FEFF}", rest),
        None => ("", content),
    };
    let nl = if body.contains("\r\n") { "\r\n" } else { "\n" };
    let mut block = format!("{key}: |{nl}");
    for line in text.lines() {
        if line.trim().is_empty() {
            block.push_str(nl);
        } else {
            block.push_str("  ");
            block.push_str(line);
            block.push_str(nl);
        }
    }

    let mut out = String::with_capacity(content.len() + block.len());
    let mut replaced = false;
    let mut in_block = false; // 正在跳过旧块体
    for line in body.split_inclusive('\n') {
        if !replaced && !in_block && line_is_active_key(line, key) {
            out.push_str(&block);
            replaced = true;
            in_block = true;
            continue;
        }
        if in_block {
            let bare = line.trim_end_matches(['\r', '\n']);
            if bare.trim().is_empty() || bare.starts_with([' ', '\t']) {
                continue; // 旧块体行（含空行）：吞掉
            }
            in_block = false; // 列 0 有内容的行：块体结束，正常输出
        }
        out.push_str(line);
    }
    if !replaced {
        if !body.is_empty() && !body.ends_with('\n') {
            out.push_str(nl);
        }
        out.push_str(&block);
    }
    format!("{bom}{out}")
}

/// 写前三道安全闸（纯函数）：白名单 → 整文件重解析 → 目标字段回读校验。
/// 任一不过返回 Err，调用方据此放弃写盘（原文件不动）。
fn validate_patched(
    new_content: &str,
    field: &str,
    action: &YamlPatchAction,
) -> AppResult<AgentYamlFields> {
    if !WRITABLE_FIELDS.contains(&field) {
        return Err(AppError::Validation(format!(
            "字段 {field} 不在 agent.yaml 可改写白名单内（仅 {WRITABLE_FIELDS:?}）"
        )));
    }
    let cfg: AgentFileConfig = serde_yaml::from_str(new_content).map_err(|e| {
        AppError::Validation(format!("改写后 agent.yaml 无法解析（已放弃写入）: {e}"))
    })?;
    let field_matches = match (field, action) {
        ("max_total_tokens", YamlPatchAction::Set(v)) => cfg.max_total_tokens == Some(*v as usize),
        ("tool_max_rounds", YamlPatchAction::Set(v)) => cfg.tool_max_rounds == Some(*v as u32),
        ("max_total_tokens", YamlPatchAction::CommentOut) => cfg.max_total_tokens.is_none(),
        ("tool_max_rounds", YamlPatchAction::CommentOut) => cfg.tool_max_rounds.is_none(),
        _ => false, // 白名单外，不可达
    };
    if !field_matches {
        return Err(AppError::Validation(format!(
            "改写后 {field} 回读值与请求不符（已放弃写入）"
        )));
    }
    Ok(AgentYamlFields::from_config(&cfg))
}

/// 读取某 agent 的 yaml 预算字段。文件缺失 / 无 workspace / 解析失败 → None
/// （前端语义：全部默认自适应，字段区显示空）。
fn read_fields(workspace_path: &Option<String>) -> Option<AgentYamlFields> {
    let dir = workspace_path.as_ref()?;
    let yaml_path = std::path::Path::new(dir).join("agent.yaml");
    let content = std::fs::read_to_string(yaml_path).ok()?;
    let cfg: AgentFileConfig = serde_yaml::from_str(&content).ok()?;
    Some(AgentYamlFields::from_config(&cfg))
}

/// 读取 agent.yaml 预算字段（None = 默认自适应 + 自动续期）
#[tauri::command]
pub async fn get_agent_yaml_fields(
    cmd: State<'_, Arc<dyn AgentCmd>>,
    agent_id: String,
) -> AppResult<AgentYamlFields> {
    let row = cmd.inner().get(&agent_id).await?;
    Ok(read_fields(&row.workspace_path).unwrap_or_default())
}

/// 设置 / 注释掉单个预算字段。
///
/// `value = Some(v)`：设为显式值（硬上限，v ≥ 1）；
/// `value = None`：注释掉该行（恢复默认自适应 + 自动续期）。
/// 安全闸全部先于写盘——失败时原文件一个字节都不动。
#[tauri::command]
pub async fn set_agent_yaml_field(
    cmd: State<'_, Arc<dyn AgentCmd>>,
    agent_id: String,
    field: String,
    value: Option<u64>,
) -> AppResult<AgentYamlFields> {
    let row = cmd.inner().get(&agent_id).await?;
    let dir = row.workspace_path.clone().ok_or_else(|| {
        AppError::Validation(format!("agent {agent_id} 未配置工作区目录，无 agent.yaml"))
    })?;
    let yaml_path = std::path::Path::new(&dir).join("agent.yaml");
    if !yaml_path.exists() {
        return Err(AppError::NotFound {
            resource: "agent.yaml",
            id: yaml_path.display().to_string(),
        });
    }
    let content = std::fs::read_to_string(&yaml_path)?;

    let action = match value {
        Some(0) => {
            return Err(AppError::Validation(
                "值必须 ≥ 1（清空请传 null = 注释掉恢复默认）".into(),
            ));
        }
        Some(v) => YamlPatchAction::Set(v),
        None => YamlPatchAction::CommentOut,
    };

    let new_content = patch_agent_yaml(&content, &field, &action);
    let fields = validate_patched(&new_content, &field, &action)?;

    // 原子写：同目录 .tmp → rename（半途失败不会留下截断的 yaml）
    let tmp_path = yaml_path.with_extension("yaml.tmp");
    std::fs::write(&tmp_path, &new_content)?;
    std::fs::rename(&tmp_path, &yaml_path)?;
    tracing::info!(
        target: "ice_paw.agent",
        "agent.yaml 已改写: {field} → {action:?}（{}）",
        yaml_path.display()
    );
    Ok(fields)
}

/// 块补丁的写前闸：整文件重解析 + `system_prompt` 回读比对（`|` 块解析值恒带
/// 尾换行，trim_end 对齐）。不过则调用方放弃写盘（原文件不动）。
fn validate_system_prompt_patched(new_content: &str, text: &str) -> AppResult<AgentYamlFields> {
    let cfg: AgentFileConfig = serde_yaml::from_str(new_content).map_err(|e| {
        AppError::Validation(format!("改写后 agent.yaml 无法解析（已放弃写入）: {e}"))
    })?;
    let matches = cfg
        .system_prompt
        .as_deref()
        .is_some_and(|v| v.trim_end() == text);
    if !matches {
        return Err(AppError::Validation(
            "改写后 system_prompt 回读值与请求不符（已放弃写入）".into(),
        ));
    }
    Ok(AgentYamlFields::from_config(&cfg))
}

/// 写 agent.yaml `system_prompt` 多行块（整块替换）——前端「风格预设」入口的
/// 落盘通道（2026-08-23）。预设是**素材不是档位**：落盘即用户文本，后续编辑
/// 与系统无关；已有内容会被**覆盖**（是否确认由前端负责）。安全闸与
/// [`set_agent_yaml_field`] 同款：改写后重解析 + 回读比对，不过则原文件不动。
#[tauri::command]
pub async fn set_agent_system_prompt(
    cmd: State<'_, Arc<dyn AgentCmd>>,
    agent_id: String,
    text: String,
) -> AppResult<AgentYamlFields> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err(AppError::Validation(
            "system_prompt 文本不能为空（清空请直接编辑 agent.yaml）".into(),
        ));
    }
    let row = cmd.inner().get(&agent_id).await?;
    let dir = row.workspace_path.clone().ok_or_else(|| {
        AppError::Validation(format!("agent {agent_id} 未配置工作区目录，无 agent.yaml"))
    })?;
    let yaml_path = std::path::Path::new(&dir).join("agent.yaml");
    if !yaml_path.exists() {
        return Err(AppError::NotFound {
            resource: "agent.yaml",
            id: yaml_path.display().to_string(),
        });
    }
    let content = std::fs::read_to_string(&yaml_path)?;

    let new_content = patch_agent_yaml_block(&content, "system_prompt", &text);
    let fields = validate_system_prompt_patched(&new_content, &text)?;

    // 原子写：同目录 .tmp → rename（半途失败不会留下截断的 yaml）
    let tmp_path = yaml_path.with_extension("yaml.tmp");
    std::fs::write(&tmp_path, &new_content)?;
    std::fs::rename(&tmp_path, &yaml_path)?;
    tracing::info!(
        target: "ice_paw.agent",
        "agent.yaml 已改写: system_prompt ← {} 行（{}）",
        text.lines().count(),
        yaml_path.display()
    );
    Ok(fields)
}

/// 多行块键整体摘除（键行 + 其后缩进块体）——[`set_agent_word_profile`] 的
/// 摘除通道（None / 空串 = 移除偏好块，prompt 不再注入）。与
/// [`patch_agent_yaml_block`] 同款块体吞行口径；无活跃键行 = no-op（幂等）。
pub fn patch_agent_yaml_remove_block(content: &str, key: &str) -> String {
    let (bom, body) = match content.strip_prefix('\u{FEFF}') {
        Some(rest) => ("\u{FEFF}", rest),
        None => ("", content),
    };
    let mut out = String::with_capacity(content.len());
    let mut removing = false;
    let mut removed = false;
    for line in body.split_inclusive('\n') {
        if !removed && line_is_active_key(line, key) {
            removed = true;
            removing = true;
            continue;
        }
        if removing {
            let bare = line.trim_end_matches(['\r', '\n']);
            if bare.trim().is_empty() || bare.starts_with([' ', '\t']) {
                continue; // 块体行（含空行）：随键行一起摘除
            }
            removing = false; // 列 0 有内容的行：块体结束，正常输出
        }
        out.push_str(line);
    }
    format!("{bom}{out}")
}

/// word_style_profile 的写前闸：重解析 + 回读比对（Some → trim_end 对齐；
/// None → 须确实为 None）。不过则调用方放弃写盘。
fn validate_word_profile_patched(
    new_content: &str,
    expect: Option<&str>,
) -> AppResult<AgentYamlFields> {
    let cfg: AgentFileConfig = serde_yaml::from_str(new_content).map_err(|e| {
        AppError::Validation(format!("改写后 agent.yaml 无法解析（已放弃写入）: {e}"))
    })?;
    let matches = match expect {
        Some(text) => cfg
            .word_style_profile
            .as_deref()
            .is_some_and(|v| v.trim_end() == text),
        None => cfg.word_style_profile.is_none(),
    };
    if !matches {
        return Err(AppError::Validation(
            "改写后 word_style_profile 回读值与请求不符（已放弃写入）".into(),
        ));
    }
    Ok(AgentYamlFields::from_config(&cfg))
}

/// 写 / 摘除 agent.yaml `word_style_profile` 多行块（D12 双轨承载之一）。
///
/// `text = Some(非空)`：整块替换（自由文字，不解析不校验——原文注入 system
/// prompt「Word 文档样式偏好」小节）；`text = None 或空白`：**摘除**（键行 +
/// 块体整体删，prompt 不再注入——与提案通道「""=摘除」语义对齐）。
/// 安全闸与 [`set_agent_system_prompt`] 同款，不过则原文件不动。
#[tauri::command]
pub async fn set_agent_word_profile(
    cmd: State<'_, Arc<dyn AgentCmd>>,
    agent_id: String,
    text: Option<String>,
) -> AppResult<AgentYamlFields> {
    let trimmed = text.unwrap_or_default().trim().to_string();
    let row = cmd.inner().get(&agent_id).await?;
    let dir = row.workspace_path.clone().ok_or_else(|| {
        AppError::Validation(format!("agent {agent_id} 未配置工作区目录，无 agent.yaml"))
    })?;
    let yaml_path = std::path::Path::new(&dir).join("agent.yaml");
    if !yaml_path.exists() {
        return Err(AppError::NotFound {
            resource: "agent.yaml",
            id: yaml_path.display().to_string(),
        });
    }
    let content = std::fs::read_to_string(&yaml_path)?;

    let (new_content, fields, is_removal) = if trimmed.is_empty() {
        let nc = patch_agent_yaml_remove_block(&content, "word_style_profile");
        let f = validate_word_profile_patched(&nc, None)?;
        (nc, f, true)
    } else {
        let nc = patch_agent_yaml_block(&content, "word_style_profile", &trimmed);
        let f = validate_word_profile_patched(&nc, Some(&trimmed))?;
        (nc, f, false)
    };

    // 原子写：同目录 .tmp → rename（半途失败不会留下截断的 yaml）
    let tmp_path = yaml_path.with_extension("yaml.tmp");
    std::fs::write(&tmp_path, &new_content)?;
    std::fs::rename(&tmp_path, &yaml_path)?;
    tracing::info!(
        target: "ice_paw.agent",
        "agent.yaml 已改写: word_style_profile {}（{}）",
        if is_removal { "摘除" } else { "写入" },
        yaml_path.display()
    );
    Ok(fields)
}

// ============================================================================
// 镜像行同步（A，2026-08-26 生产反馈：UI 显示智谱而 yaml 停在出生时的 deepseek）
// ============================================================================

/// `provider` / `model` / `base_url` 三行是创建时 [`write_default_agent_yaml`]
/// 写入的**信息性镜像**：`AgentFileConfig` 不解析它们（serde 静默忽略未知键，
/// 运行时 provider/model/base_url 全部来自 DB 行），但对用户它们是文件里的
/// 「配置真相」。update() 只改 DB 不同步镜像 → UI 与文件分裂，直接误导排障
/// （生产实例：用户看到 yaml 是 deepseek，怀疑端点配错，实际运行时根本不读）。
///
/// 同步语义：文件存在才补丁（不创建）；活跃行替换 / 缺失追加；base_url 为空
/// 时移除活跃行（与创建时「空不写」对称）。与 [`patch_agent_yaml`] 同款逐行
/// 纪律：BOM 摘出回填、非目标行逐字节保留。
pub fn patch_agent_yaml_string(content: &str, key: &str, value: &str) -> String {
    let (bom, body) = match content.strip_prefix('\u{FEFF}') {
        Some(rest) => ("\u{FEFF}", rest),
        None => ("", content),
    };

    let mut out = String::with_capacity(content.len() + key.len() + value.len() + 4);
    let mut found = false;
    for line in body.split_inclusive('\n') {
        if !found && line_is_active_key(line, key) {
            found = true;
            out.push_str(key);
            out.push_str(": ");
            out.push_str(value);
            out.push_str(line_end(line));
        } else {
            out.push_str(line);
        }
    }
    if !found {
        if !body.is_empty() && !body.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(key);
        out.push_str(": ");
        out.push_str(value);
        out.push('\n');
    }
    format!("{bom}{out}")
}

/// 移除某键的活跃行（整行删，含行尾）；无活跃行 = no-op。镜像 base_url 清空用。
pub fn remove_active_scalar_line(content: &str, key: &str) -> String {
    let (bom, body) = match content.strip_prefix('\u{FEFF}') {
        Some(rest) => ("\u{FEFF}", rest),
        None => ("", content),
    };
    let mut out = String::with_capacity(content.len());
    for line in body.split_inclusive('\n') {
        if line_is_active_key(line, key) {
            continue;
        }
        out.push_str(line);
    }
    format!("{bom}{out}")
}

/// 一次性同步三行镜像（provider / model / base_url）。
pub fn sync_agent_yaml_mirror(
    content: &str,
    provider: &str,
    model: &str,
    base_url: Option<&str>,
) -> String {
    let out = patch_agent_yaml_string(content, "provider", provider);
    let out = patch_agent_yaml_string(&out, "model", model);
    match base_url.filter(|s| !s.is_empty()) {
        Some(url) => patch_agent_yaml_string(&out, "base_url", url),
        None => remove_active_scalar_line(&out, "base_url"),
    }
}

/// 读某键活跃行的值（`key: value` 冒号后 trim）——镜像回读闸用（镜像键不是
/// `AgentFileConfig` 字段，回读走行级而非 serde 字段）。
fn active_scalar_value(content: &str, key: &str) -> Option<String> {
    let body = content.strip_prefix('\u{FEFF}').unwrap_or(content);
    body.split_inclusive('\n')
        .find(|line| line_is_active_key(line, key))
        .map(|line| {
            line.trim_end_matches(['\r', '\n'])
                .strip_prefix(key)
                .and_then(|rest| rest.strip_prefix(':'))
                .map(|v| v.trim().to_string())
                .unwrap_or_default()
        })
}

/// 镜像同步的写前闸：整文件重解析（用户其余配置不得被破坏）+ 三行行级回读
/// 比对。不过则调用方放弃写盘。
fn validate_mirror_sync(
    new_content: &str,
    provider: &str,
    model: &str,
    base_url: Option<&str>,
) -> AppResult<()> {
    serde_yaml::from_str::<AgentFileConfig>(new_content).map_err(|e| {
        AppError::Validation(format!("镜像同步后 agent.yaml 无法解析（已放弃写入）: {e}"))
    })?;
    let expect_url = base_url.filter(|s| !s.is_empty());
    if active_scalar_value(new_content, "provider").as_deref() != Some(provider)
        || active_scalar_value(new_content, "model").as_deref() != Some(model)
        || active_scalar_value(new_content, "base_url").as_deref() != expect_url
    {
        return Err(AppError::Validation(
            "镜像同步后 provider/model/base_url 回读值与请求不符（已放弃写入）".into(),
        ));
    }
    Ok(())
}

/// 对某 agent 工作区的 agent.yaml 做镜像行同步（update() 调用）。
///
/// - 文件不存在 → Ok（**不创建**——镜像只跟随已存在的文件）
/// - 内容无变化（逐字节相等）→ 直接返回，不动文件
/// - 写前闸 + 原子写（同目录 .tmp → rename）
pub fn sync_agent_yaml_mirror_file(
    workspace_dir: &str,
    provider: &str,
    model: &str,
    base_url: Option<&str>,
) -> AppResult<()> {
    let yaml_path = std::path::Path::new(workspace_dir).join("agent.yaml");
    if !yaml_path.exists() {
        return Ok(());
    }
    let content = std::fs::read_to_string(&yaml_path)?;
    let new_content = sync_agent_yaml_mirror(content.as_str(), provider, model, base_url);
    if new_content == content {
        return Ok(());
    }
    validate_mirror_sync(&new_content, provider, model, base_url)?;

    let tmp_path = yaml_path.with_extension("yaml.tmp");
    std::fs::write(&tmp_path, &new_content)?;
    std::fs::rename(&tmp_path, &yaml_path)?;
    tracing::info!(
        target: "ice_paw.agent",
        "agent.yaml 镜像已同步: provider={provider} model={model}（{}）",
        yaml_path.display()
    );
    Ok(())
}

// ============================================================================
// 单元测试（纯函数层：patch 逐字节语义 + 三道安全闸）
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// 典型用户 yaml（含中文注释 + system_prompt 块 + 两个目标键）
    fn sample_yaml() -> String {
        [
            "# agent.yaml — Agent 行为和角色配置",
            "provider: glm",
            "model: glm-5.2",
            "system_prompt: |",
            "  你是一个品牌设计助手。",
            "  第二行保留缩进。",
            "temperature: 0.7",
            "# 工具调用最大轮数（默认 50 + 自动续期 2 次）",
            "tool_max_rounds: 50",
            "max_total_tokens: 800000",
            "base_url: https://open.bigmodel.cn/api/coding/paas/v4",
        ]
        .join("\n")
            + "\n"
    }

    #[test]
    fn set_overwrites_active_line_only() {
        let out = patch_agent_yaml(
            &sample_yaml(),
            "max_total_tokens",
            &YamlPatchAction::Set(1_000_000),
        );
        assert!(out.contains("max_total_tokens: 1000000\n"));
        assert!(!out.contains("800000"));
        // 其余行原样：中文注释、system_prompt 块、别的键
        assert!(out.contains("# agent.yaml — Agent 行为和角色配置"));
        assert!(out.contains("  你是一个品牌设计助手。"));
        assert!(out.contains("tool_max_rounds: 50"));
        assert!(out.contains("base_url: https://open.bigmodel.cn/api/coding/paas/v4"));
    }

    #[test]
    fn comment_out_preserves_inline_comment() {
        let yaml = "tool_max_rounds: 50 # 备注说明\nprovider: glm\n";
        let out = patch_agent_yaml(yaml, "tool_max_rounds", &YamlPatchAction::CommentOut);
        assert_eq!(out, "# tool_max_rounds: 50 # 备注说明\nprovider: glm\n");
    }

    #[test]
    fn set_missing_key_appends() {
        let yaml = "provider: glm\n";
        let out = patch_agent_yaml(yaml, "max_total_tokens", &YamlPatchAction::Set(500000));
        assert_eq!(out, "provider: glm\nmax_total_tokens: 500000\n");
        // 末行无换行时补一个再追加
        let out2 = patch_agent_yaml(
            "provider: glm",
            "tool_max_rounds",
            &YamlPatchAction::Set(30),
        );
        assert_eq!(out2, "provider: glm\ntool_max_rounds: 30\n");
    }

    #[test]
    fn comment_out_missing_or_already_commented_is_noop() {
        // 缺键 → 原样返回
        let yaml = "provider: glm\n";
        assert_eq!(
            patch_agent_yaml(yaml, "max_total_tokens", &YamlPatchAction::CommentOut),
            yaml
        );
        // 只有注释行（非列 0 活跃键）→ 原样返回
        let commented = "# max_total_tokens: 3000000\nprovider: glm\n";
        assert_eq!(
            patch_agent_yaml(commented, "max_total_tokens", &YamlPatchAction::CommentOut),
            commented
        );
        // Set 到只有注释行的文件 → 追加活跃行，注释行不动
        let out = patch_agent_yaml(commented, "max_total_tokens", &YamlPatchAction::Set(7));
        assert!(out.starts_with("# max_total_tokens: 3000000\n"));
        assert!(out.ends_with("max_total_tokens: 7\n"));
    }

    #[test]
    fn crlf_line_endings_preserved() {
        let yaml = "provider: glm\r\nmax_total_tokens: 800000\r\n";
        let set = patch_agent_yaml(yaml, "max_total_tokens", &YamlPatchAction::Set(1));
        assert!(set.contains("max_total_tokens: 1\r\n"));
        assert!(!set.contains("800000"));
        let out = patch_agent_yaml(yaml, "max_total_tokens", &YamlPatchAction::CommentOut);
        assert!(out.contains("# max_total_tokens: 800000\r\n"));
    }

    #[test]
    fn bom_preserved_and_first_line_key_matches() {
        let yaml = "\u{FEFF}max_total_tokens: 800000\nprovider: glm\n";
        let out = patch_agent_yaml(yaml, "max_total_tokens", &YamlPatchAction::Set(9));
        assert!(out.starts_with('\u{FEFF}'));
        assert!(out.contains("max_total_tokens: 9\n"));
        // 缺键追加时 BOM 也保留
        let out2 = patch_agent_yaml(
            "\u{FEFF}provider: glm\n",
            "tool_max_rounds",
            &YamlPatchAction::Set(5),
        );
        assert!(out2.starts_with('\u{FEFF}'));
        assert!(out2.ends_with("tool_max_rounds: 5\n"));
    }

    #[test]
    fn indented_and_prefixed_keys_not_matched() {
        let yaml =
            "extra_params:\n  max_total_tokens: 1\nmax_total_tokens_x: 2\n# max_total_tokens: 3\n";
        let out = patch_agent_yaml(yaml, "max_total_tokens", &YamlPatchAction::CommentOut);
        assert_eq!(out, yaml, "缩进子键/前缀相似键/注释行一律不误伤");
    }

    #[test]
    fn system_prompt_block_preserved_byte_for_byte() {
        let yaml = sample_yaml();
        let out = patch_agent_yaml(
            &yaml.clone(),
            "tool_max_rounds",
            &YamlPatchAction::CommentOut,
        );
        // 除目标行加前缀外，整文件其余部分逐字节一致
        let expected = yaml.replace("tool_max_rounds: 50", "# tool_max_rounds: 50");
        assert_eq!(out, expected);
    }

    #[test]
    fn validate_rejects_non_whitelisted_field() {
        let yaml = "provider: glm\n";
        let patched = patch_agent_yaml(yaml, "provider", &YamlPatchAction::Set(1));
        let err = validate_patched(&patched, "provider", &YamlPatchAction::Set(1)).unwrap_err();
        assert!(matches!(err, AppError::Validation(_)));
    }

    #[test]
    fn validate_rejects_when_reparse_fails_or_field_mismatches() {
        // 文件里另一字段本就坏了（temperature 非数字）：补丁后整文件仍解析失败 → 拒绝
        let broken = "provider: glm\ntemperature: not_a_number\nmax_total_tokens: 5\n";
        let patched = patch_agent_yaml(broken, "max_total_tokens", &YamlPatchAction::Set(6));
        assert!(validate_patched(&patched, "max_total_tokens", &YamlPatchAction::Set(6)).is_err());

        // 重复键：补丁只改首个，第二个仍活跃 → serde 重解析 duplicate 报错 → 拒绝
        let dup = "tool_max_rounds: 50\ntool_max_rounds: 60\n";
        let patched2 = patch_agent_yaml(dup, "tool_max_rounds", &YamlPatchAction::Set(70));
        assert!(validate_patched(&patched2, "tool_max_rounds", &YamlPatchAction::Set(70)).is_err());

        // u64 超出 tool_max_rounds 的 u32 范围 → 解析失败 → 拒绝
        let patched3 = patch_agent_yaml(
            "provider: glm\n",
            "tool_max_rounds",
            &YamlPatchAction::Set(u64::MAX),
        );
        assert!(validate_patched(
            &patched3,
            "tool_max_rounds",
            &YamlPatchAction::Set(u64::MAX)
        )
        .is_err());
    }

    #[test]
    fn validate_accepts_valid_patches_and_reads_back() {
        let patched = patch_agent_yaml(
            &sample_yaml(),
            "max_total_tokens",
            &YamlPatchAction::Set(1_000_000),
        );
        let fields = validate_patched(
            &patched,
            "max_total_tokens",
            &YamlPatchAction::Set(1_000_000),
        )
        .unwrap();
        assert_eq!(fields.max_total_tokens, Some(1_000_000));
        assert_eq!(fields.tool_max_rounds, Some(50)); // 另一字段不受影响

        // CommentOut：目标字段须回读为 None（另一字段照常保留）
        let commented = patch_agent_yaml(
            &sample_yaml(),
            "tool_max_rounds",
            &YamlPatchAction::CommentOut,
        );
        let fields2 =
            validate_patched(&commented, "tool_max_rounds", &YamlPatchAction::CommentOut).unwrap();
        assert_eq!(fields2.tool_max_rounds, None);
        assert_eq!(fields2.max_total_tokens, Some(800000));
    }

    // ---- 多行块补丁（patch_agent_yaml_block / set_agent_system_prompt 通道） ----

    /// 风格预设文本形态：多段 + 空行 + 列表（与前端 stylePresets 三档同构）
    fn preset_text() -> String {
        "你是 Alpha，一名工程助手。\n\n做事方式：\n- 先确认再动手\n- 改代码前先读目标文件".into()
    }

    #[test]
    fn block_patch_replaces_existing_block() {
        let out = patch_agent_yaml_block(&sample_yaml(), "system_prompt", &preset_text());
        assert!(out.contains(
            "system_prompt: |\n  你是 Alpha，一名工程助手。\n\n  做事方式：\n  - 先确认再动手\n  - 改代码前先读目标文件\n"
        ));
        // 旧块体整体消失（两行都被换掉）
        assert!(!out.contains("品牌设计助手"));
        assert!(!out.contains("第二行保留缩进"));
        // 前后键与中文注释原样
        assert!(out.contains("provider: glm"));
        assert!(out.contains("temperature: 0.7"));
        assert!(out.contains("# 工具调用最大轮数（默认 50 + 自动续期 2 次）"));
        assert!(out.contains("tool_max_rounds: 50"));
    }

    #[test]
    fn block_patch_missing_key_appends() {
        let out = patch_agent_yaml_block("provider: glm\n", "system_prompt", "你好");
        assert_eq!(out, "provider: glm\nsystem_prompt: |\n  你好\n");
        // 末行无换行：补一个再追加
        let out2 = patch_agent_yaml_block("provider: glm", "system_prompt", "你好");
        assert_eq!(out2, "provider: glm\nsystem_prompt: |\n  你好\n");
    }

    #[test]
    fn block_patch_inline_scalar_replaced() {
        // 用户手写的单行形态：键行替换为块，后续列 0 行不是块体、原样保留
        let out = patch_agent_yaml_block("system_prompt: 单行值\nprovider: glm\n", "system_prompt", "新文本");
        assert!(out.starts_with("system_prompt: |\n  新文本\n"));
        assert!(out.contains("provider: glm"));
        assert!(!out.contains("单行值"));
    }

    #[test]
    fn block_patch_crlf_and_bom_preserved() {
        let yaml = "\u{FEFF}provider: glm\r\nsystem_prompt: |\r\n  旧\r\ntool_max_rounds: 50\r\n";
        let out = patch_agent_yaml_block(yaml, "system_prompt", "新");
        assert!(out.starts_with('\u{FEFF}'));
        // 生成行沿用 CRLF（不混行尾）
        assert!(out.contains("system_prompt: |\r\n  新\r\n"));
        assert!(out.contains("tool_max_rounds: 50\r\n"));
        assert!(!out.contains("旧"));
    }

    #[test]
    fn block_patch_trailing_blank_lines_of_block_consumed() {
        // 旧块体后的空行属于块（替换后不残留空行堆积）
        let yaml = "system_prompt: |\n  旧\n\n\nprovider: glm\n";
        let out = patch_agent_yaml_block(yaml, "system_prompt", "新");
        assert!(out.starts_with("system_prompt: |\n  新\nprovider: glm\n"));
    }

    #[test]
    fn block_patch_indented_and_prefixed_keys_not_matched() {
        let yaml = "extra_params:\n  system_prompt: 内层\nsystem_prompt_x: 前缀\n# system_prompt: 注释\n";
        let out = patch_agent_yaml_block(yaml, "system_prompt", "新");
        // 无列 0 活跃键 → 追加，原有行一律不动
        assert_eq!(
            out,
            format!("{yaml}system_prompt: |\n  新\n")
        );
    }

    #[test]
    fn block_patch_roundtrip_parses_to_text() {
        let text = preset_text();
        let out = patch_agent_yaml_block(&sample_yaml(), "system_prompt", &text);
        let cfg: AgentFileConfig = serde_yaml::from_str(&out).unwrap();
        // | 块解析值恒带尾换行 → trim_end 对齐
        assert_eq!(cfg.system_prompt.as_deref().map(str::trim_end), Some(text.as_str()));
        // 其余字段不受影响
        assert_eq!(cfg.max_total_tokens, Some(800000));
        assert_eq!(cfg.tool_max_rounds, Some(50));
    }

    #[test]
    fn validate_system_prompt_gate() {
        // 正常补丁 → 过闸，回读一致
        let out = patch_agent_yaml_block(&sample_yaml(), "system_prompt", "新文本");
        let fields = validate_system_prompt_patched(&out, "新文本").unwrap();
        assert_eq!(fields.system_prompt.as_deref(), Some("新文本\n"));

        // 内容被篡改 → 回读不符 → 拒
        let tampered = out.replace("新文本", "别的");
        assert!(validate_system_prompt_patched(&tampered, "新文本").is_err());

        // 解析失败（类型不符）→ 拒
        assert!(validate_system_prompt_patched("system_prompt: [", "x").is_err());
    }

    // ---- word_style_profile（set_agent_word_profile 通道：块写 / 块摘除） ----

    #[test]
    fn word_profile_block_write_and_readback() {
        // 缺键追加 → 过闸；已有块整体替换 → 旧值消失
        let profile = "表头：黑体 11pt 深蓝底白字\n正文：宋体 10.5pt，行距 1.5";
        let out = patch_agent_yaml_block(&sample_yaml(), "word_style_profile", profile);
        let fields = validate_word_profile_patched(&out, Some(profile)).unwrap();
        assert_eq!(
            fields.word_style_profile.as_deref().map(str::trim_end),
            Some(profile)
        );
        // system_prompt 块与预算字段不受影响
        assert!(out.contains("  你是一个品牌设计助手。"));
        assert_eq!(fields.max_total_tokens, Some(800000));
    }

    #[test]
    fn word_profile_removal_drops_block_and_is_idempotent() {
        let with_block = patch_agent_yaml_block(
            &sample_yaml(),
            "word_style_profile",
            "表头黑体\n正文宋体",
        );
        let removed = patch_agent_yaml_remove_block(&with_block, "word_style_profile");
        assert!(!removed.contains("word_style_profile"));
        assert!(!removed.contains("表头黑体"), "块体随键行整体摘除");
        assert!(!removed.contains("正文宋体"));
        // 邻居逐字节保留
        assert!(removed.contains("system_prompt: |"));
        assert!(removed.contains("tool_max_rounds: 50"));
        let fields = validate_word_profile_patched(&removed, None).unwrap();
        assert_eq!(fields.word_style_profile, None);

        // 无键 = no-op（幂等摘除）；缩进子键 / 注释行不误伤
        assert_eq!(patch_agent_yaml_remove_block(&sample_yaml(), "word_style_profile"), sample_yaml());
        let nested = "extra_params:\n  word_style_profile: 内层\n# word_style_profile: 注释\n";
        assert_eq!(patch_agent_yaml_remove_block(nested, "word_style_profile"), nested);
    }

    #[test]
    fn word_profile_removal_crlf_bom() {
        let yaml = "\u{FEFF}provider: glm\r\nword_style_profile: |\r\n  偏好\r\ntool_max_rounds: 50\r\n";
        let out = patch_agent_yaml_remove_block(yaml, "word_style_profile");
        assert!(out.starts_with('\u{FEFF}'));
        assert_eq!(out, "\u{FEFF}provider: glm\r\ntool_max_rounds: 50\r\n");
    }

    #[test]
    fn word_profile_gate_rejects_mismatch() {
        // 回读不符（键仍在）→ 拒
        let with_block = patch_agent_yaml_block("provider: glm\n", "word_style_profile", "偏好");
        assert!(validate_word_profile_patched(&with_block, None).is_err());
        // 解析失败 → 拒
        assert!(validate_word_profile_patched("word_style_profile: [", Some("x")).is_err());
    }

    // ---- 镜像行同步（A：update() 后 provider/model/base_url 跟随 DB） ----

    /// 生产实例形态：出生 yaml 带 deepseek 镜像行 + base_url + 中文注释 + 块
    fn born_deepseek_yaml() -> String {
        [
            "# agent.yaml — Agent 行为和角色配置",
            "# 修改后即时生效，无需重启",
            "",
            "provider: deepseek",
            "model: deepseek-v4-flash",
            "system_prompt: |",
            "  你是一个工程助手。",
            "temperature: 0.7",
            "base_url: https://api.deepseek.com",
        ]
        .join("\n")
            + "\n"
    }

    #[test]
    fn mirror_sync_updates_lines_and_preserves_rest() {
        let out = sync_agent_yaml_mirror(
            &born_deepseek_yaml(),
            "glm",
            "glm-5.2",
            Some("https://open.bigmodel.cn/api/coding/paas/v4"),
        );
        assert!(out.contains("provider: glm\n"));
        assert!(out.contains("model: glm-5.2\n"));
        assert!(out.contains("base_url: https://open.bigmodel.cn/api/coding/paas/v4\n"));
        assert!(!out.contains("deepseek"));
        // 非目标行逐字节保留：注释、块体、temperature
        assert!(out.contains("# agent.yaml — Agent 行为和角色配置"));
        assert!(out.contains("  你是一个工程助手。"));
        assert!(out.contains("temperature: 0.7"));
        // 写前闸：重解析过 + 三行回读一致
        validate_mirror_sync(
            &out,
            "glm",
            "glm-5.2",
            Some("https://open.bigmodel.cn/api/coding/paas/v4"),
        )
        .unwrap();
    }

    #[test]
    fn mirror_sync_appends_when_lines_missing() {
        // 存量 yaml 无镜像行（旧格式/用户删除）→ 追加（文件不重排、其余行不动）
        let yaml = "system_prompt: |\n  你好\ntemperature: 0.3\n";
        let out = sync_agent_yaml_mirror(yaml, "glm", "glm-5.2", None);
        assert_eq!(
            out,
            format!("{yaml}provider: glm\nmodel: glm-5.2\n")
        );
        validate_mirror_sync(&out, "glm", "glm-5.2", None).unwrap();
    }

    #[test]
    fn mirror_sync_removes_base_url_when_none() {
        let out = sync_agent_yaml_mirror(&born_deepseek_yaml(), "ollama", "qwen3", None);
        assert!(!out.contains("base_url"));
        assert!(out.contains("provider: ollama\n"));
        assert!(out.contains("model: qwen3\n"));
        // 清空幂等：再同步一次逐字节不变
        let again = sync_agent_yaml_mirror(&out, "ollama", "qwen3", None);
        assert_eq!(out, again);
        // 从无到有再写回：Some 恢复行
        let restored = sync_agent_yaml_mirror(&out, "ollama", "qwen3", Some("http://localhost:11434/v1"));
        assert!(restored.contains("base_url: http://localhost:11434/v1\n"));
    }

    #[test]
    fn mirror_sync_idempotent_byte_equal() {
        // 值未变 → 逐字节相等（sync_agent_yaml_mirror_file 据此跳过写盘）
        let once = sync_agent_yaml_mirror(
            &born_deepseek_yaml(),
            "glm",
            "glm-5.2",
            Some("https://open.bigmodel.cn/api/coding/paas/v4"),
        );
        let twice = sync_agent_yaml_mirror(
            &once,
            "glm",
            "glm-5.2",
            Some("https://open.bigmodel.cn/api/coding/paas/v4"),
        );
        assert_eq!(once, twice);
    }

    #[test]
    fn mirror_sync_crlf_bom_preserved() {
        let yaml = "\u{FEFF}provider: deepseek\r\nmodel: deepseek-v4-flash\r\ntemperature: 0.7\r\n";
        let out = sync_agent_yaml_mirror(yaml, "glm", "glm-5.2", None);
        assert!(out.starts_with('\u{FEFF}'));
        assert!(out.contains("provider: glm\r\n"));
        assert!(out.contains("model: glm-5.2\r\n"));
        assert!(out.contains("temperature: 0.7\r\n"));
    }

    #[test]
    fn mirror_sync_quotes_and_nested_keys_not_matched() {
        // 注释行 / 缩进子键 / 前缀相似键一律不误伤；缺活跃键 → 追加
        let yaml =
            "# provider: glm\nextra:\n  provider: 内层\nprovider_x: 前缀\n";
        let out = sync_agent_yaml_mirror(yaml, "glm", "glm-5.2", None);
        assert!(out.starts_with("# provider: glm\n"));
        assert!(out.contains("  provider: 内层\n"));
        assert!(out.contains("provider_x: 前缀\n"));
        assert!(out.ends_with("provider: glm\nmodel: glm-5.2\n"));
    }

    #[test]
    fn mirror_gate_rejects_broken_yaml() {
        // 文件其余部分本就解析失败（未闭合序列）：补丁后重解析不过 → 拒绝写盘
        let broken = "provider: deepseek\nbroken: [unclosed\n";
        let patched = patch_agent_yaml_string(broken, "provider", "glm");
        assert!(validate_mirror_sync(&patched, "glm", "any", None).is_err());
        // 重解析过但行级回读不符（provider 行 ≠ 期望）→ 拒
        assert!(validate_mirror_sync("provider: glm\n", "deepseek", "m", None).is_err());
        assert!(validate_mirror_sync("provider: glm\nmodel: m\n", "glm", "m", Some("https://x")).is_err());
    }
}
