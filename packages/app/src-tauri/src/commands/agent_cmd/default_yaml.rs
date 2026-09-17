//! 出生 agent.yaml 内容构造 + 落盘（从 `agent_cmd.rs` God module 拆出，U3-5 ③）。
//!
//! 只负责「新 agent 创建时若 workspace 下尚无 agent.yaml 则生成一份镜像文件」。
//! 运行时不读此文件（模型身份以 DB 行为准），它只是信息性镜像；后续 update 的
//! provider/model/base_url 镜像同步走 `commands/agent_yaml.rs::sync_agent_yaml_mirror_file`。

/// 在 Agent workspace 目录写入默认 agent.yaml。
///
/// - 仅在文件不存在时写入（不覆盖用户手动编辑的内容）
/// - 写入失败仅 warn，不阻断 Agent 创建
#[allow(clippy::too_many_arguments)]
pub(crate) fn write_default_agent_yaml(
    workspace_dir: &str,
    agent_name: &str,
    provider: &str,
    model: &str,
    system_prompt: Option<&str>,
    temperature: f64,
    max_tokens: i32,
    enabled_tools: Option<&[String]>,
    base_url: Option<&str>,
) {
    let yaml_path = std::path::Path::new(workspace_dir).join("agent.yaml");

    // 文件已存在则跳过
    if yaml_path.exists() {
        tracing::info!(
            target: "ice_paw.agent",
            "agent.yaml 已存在，跳过自动生成: {}",
            yaml_path.display()
        );
        return;
    }

    let content = build_default_agent_yaml_content(
        agent_name,
        provider,
        model,
        system_prompt,
        temperature,
        max_tokens,
        enabled_tools,
        base_url,
    );

    match std::fs::write(&yaml_path, &content) {
        Ok(()) => {
            tracing::info!(
                target: "ice_paw.agent",
                "已生成默认 agent.yaml: {}",
                yaml_path.display()
            );
        }
        Err(e) => {
            tracing::warn!(
                target: "ice_paw.agent",
                "写入 agent.yaml 失败（Agent 仍可用，忽略）: {} — {}",
                yaml_path.display(),
                e
            );
        }
    }
}

/// 构造默认 agent.yaml 内容（纯函数，为单测让路）。
///
/// 模板纪律：`tool_max_rounds` / `max_total_tokens` 一律**注释掉**——显式值
/// 是 B1 语义下的硬上限（触顶即停、不自动续期），写进模板会让所有新 agent
/// 默认失去自动续期额度；留空 = 软默认 + 自动续期（长任务不误杀）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_default_agent_yaml_content(
    agent_name: &str,
    provider: &str,
    model: &str,
    system_prompt: Option<&str>,
    temperature: f64,
    max_tokens: i32,
    enabled_tools: Option<&[String]>,
    base_url: Option<&str>,
) -> String {
    let default_sp = format!("{} 是一个 AI 助手。", agent_name);
    let sp = system_prompt
        .filter(|s| !s.is_empty())
        .unwrap_or(&default_sp);
    // YAML multiline: 每行缩进 2 空格
    let sp_indented = sp
        .lines()
        .map(|l| format!("  {}", l))
        .collect::<Vec<_>>()
        .join("\n");

    let mut content = format!(
        "# agent.yaml — Agent 行为和角色配置\n\
         # 修改后即时生效，无需重启\n\
         \n\
         provider: {}\n\
         model: {}\n\
         system_prompt: |\n{}\n\
         temperature: {}\n\
         max_tokens: {}\n\
         # 工具调用最大轮数（默认 50 + 自动续期 2 次；显式设置 = 硬上限，触顶即停不自动续期）\n\
         # tool_max_rounds: 50\n\
         # Token 预算上限（默认按上下文窗口自适应 3× + 自动续期 2 次；显式设置 = 硬上限，长对话会频繁中断）\n\
         # max_total_tokens: 3000000\n",
        provider, model, sp_indented, temperature, max_tokens,
    );

    if let Some(tools) = enabled_tools {
        if !tools.is_empty() {
            content.push_str("\nenabled_tools:\n");
            for t in tools {
                content.push_str(&format!("  - {}\n", t));
            }
        }
    }
    if let Some(url) = base_url {
        if !url.is_empty() {
            content.push_str(&format!("\nbase_url: {}\n", url));
        }
    }
    content
}
