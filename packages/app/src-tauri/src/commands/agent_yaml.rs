//! `commands::agent_yaml` — agent.yaml 预算字段定向改写
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

/// 预算字段快照（读命令返回 / 写命令成功后回显）
#[derive(Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct AgentYamlFields {
    pub max_total_tokens: Option<u64>,
    pub tool_max_rounds: Option<u64>,
}

impl AgentYamlFields {
    fn from_config(cfg: &AgentFileConfig) -> Self {
        Self {
            max_total_tokens: cfg.max_total_tokens.map(|v| v as u64),
            tool_max_rounds: cfg.tool_max_rounds.map(|v| v as u64),
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
}
