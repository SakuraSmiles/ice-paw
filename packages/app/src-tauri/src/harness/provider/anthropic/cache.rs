//! P2-3: Anthropic prompt caching — cache_control 断点注入
//!
//! Anthropic 官方文档限制最多 4 个 cache_control 断点。
//! 本模块独立处理断点注入，与 HTTP 请求 / SSE 解析完全正交。
//!
//! 策略：
//! 1. 断点 1：system prompt 的最后一个 content block
//! 2. 断点 2~4：倒数第 3 条之前的 message（最多再加 3 个）
//!
//! Anthropic 的 cache_control 放在 content block 级别（非 message 级别）：
//! - system 使用数组格式，最后一个 block 带 cache_control
//! - message 的 content 如果是字符串，先转为单元素数组再附加 cache_control
//! - message 的 content 如果是数组，最后一个 block 附加 cache_control

/// Anthropic 缓存断点最大数量（官方限制 ≤ 4）
pub(crate) const MAX_CACHE_BREAKPOINTS: usize = 4;

/// 注入 cache_control 断点。
///
/// 导出为独立函数（从 `AnthropicAdapter::inject_cache_breakpoints` 关联方法重构而来），
/// 供 `mod.rs::stream_chat` 调用。
pub(crate) fn inject_cache_breakpoints(
    system: &mut Option<Vec<serde_json::Value>>,
    messages: &mut [serde_json::Value],
) {
    let mut breakpoints_used = 0;

    // 断点 1：system prompt 的最后一个 content block
    if let Some(blocks) = system.as_mut() {
        if let Some(last) = blocks.last_mut() {
            last["cache_control"] = serde_json::json!({ "type": "ephemeral" });
            breakpoints_used += 1;
        }
    }

    if breakpoints_used >= MAX_CACHE_BREAKPOINTS {
        return;
    }

    // 断点 2~4：倒数第 3 条之前的 message（跳过第 1 条 user 消息）
    let len = messages.len();
    if len <= 3 {
        return;
    }

    let cutoff = len.saturating_sub(3);
    for msg in messages.iter_mut().take(cutoff).skip(1) {
        if breakpoints_used >= MAX_CACHE_BREAKPOINTS {
            break;
        }

        // 在 content 的最后一个 block 上附加 cache_control
        if let Some(content) = msg.get_mut("content") {
            match content {
                serde_json::Value::String(_) => {
                    // 字符串 → 转为单元素数组，附带 cache_control
                    let text = content.as_str().unwrap_or("").to_string();
                    *content = serde_json::json!([
                        {
                            "type": "text",
                            "text": text,
                            "cache_control": { "type": "ephemeral" }
                        }
                    ]);
                }
                serde_json::Value::Array(blocks) => {
                    // 数组 → 在最后一个 block 上附加 cache_control
                    if let Some(last) = blocks.last_mut() {
                        last["cache_control"] = serde_json::json!({ "type": "ephemeral" });
                    }
                }
                _ => {}
            }
            breakpoints_used += 1;
        }
    }
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// 辅助：统计 system + messages 中的 cache_control 断点总数。
    /// 遍历 system 各 block 以及每个 message 的 content 各 block，
    /// 检查是否存在 `cache_control.type == "ephemeral"`。
    fn count_cache_breakpoints(
        system: &Option<Vec<serde_json::Value>>,
        messages: &[serde_json::Value],
    ) -> usize {
        let mut count = 0;

        if let Some(blocks) = system.as_ref() {
            for block in blocks {
                if block.get("cache_control").is_some() {
                    count += 1;
                }
            }
        }

        for msg in messages {
            if let Some(content) = msg.get("content") {
                match content {
                    serde_json::Value::String(_) => {
                        // 字符串 content 在注入后会变成数组（不可能保留字符串），
                        // 此处为防御性兜底：如果遇到，说明函数未处理。
                    }
                    serde_json::Value::Array(blocks) => {
                        for block in blocks {
                            if block.get("cache_control").is_some() {
                                count += 1;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        count
    }

    /// 系统提示存在、消息 ≤ 3 条 → 只在 system 最后一个 block 加 1 个断点；
    /// messages 上不应有任何 cache_control（因为 len <= 3 直接 return）。
    #[test]
    fn inject_cache_breakpoints_system_only() {
        let mut system = Some(vec![
            serde_json::json!({ "type": "text", "text": "你是助手" }),
        ]);
        let mut msgs = vec![
            serde_json::json!({ "role": "user", "content": "hi" }),
            serde_json::json!({ "role": "assistant", "content": "hello" }),
        ];

        inject_cache_breakpoints(&mut system, &mut msgs);

        // system 最后一个 block 必须有 cache_control
        let sys_blocks = system.as_ref().unwrap();
        assert_eq!(sys_blocks.len(), 1);
        assert_eq!(sys_blocks[0]["cache_control"]["type"], "ephemeral");

        // messages 不应被改写为数组（因为函数在 len <= 3 时直接 return）
        assert!(msgs[0]["content"].is_string(), "user content 应保持字符串");
        assert!(
            msgs[1]["content"].is_string(),
            "assistant content 应保持字符串"
        );

        // 总断点数 = 1
        assert_eq!(count_cache_breakpoints(&system, &msgs), 1);
    }

    /// 长对话（10 条消息）→ system 1 个 + messages 最多 3 个，总断点数 = 4。
    /// 验证 Anthropic 的 4 断点硬约束被正确遵守。
    #[test]
    fn inject_cache_breakpoints_long_conversation() {
        let mut system = Some(vec![
            serde_json::json!({ "type": "text", "text": "你是助手" }),
        ]);
        let mut msgs: Vec<serde_json::Value> = (0..10)
            .map(|i| {
                let role = if i % 2 == 0 { "user" } else { "assistant" };
                serde_json::json!({ "role": role, "content": format!("msg-{i}") })
            })
            .collect();

        inject_cache_breakpoints(&mut system, &mut msgs);

        // 总断点数 = 4（Anthropic 硬限制 = MAX_CACHE_BREAKPOINTS）
        let total = count_cache_breakpoints(&system, &msgs);
        assert_eq!(
            total, MAX_CACHE_BREAKPOINTS,
            "长对话应达到 4 断点上限，实际 = {total}"
        );

        // system 一定有 cache_control
        assert!(system.as_ref().unwrap()[0].get("cache_control").is_some());

        // messages[0]（首条 user）被 skip(1) 跳过 → content 仍为字符串，无 cache_control
        assert!(
            msgs[0]["content"].is_string(),
            "首条 user 的 content 应保持字符串"
        );

        // messages 末尾 3 条（indices 7, 8, 9）不应有 cache_control（cutoff = 7，take(7) 后 skip(1) = indices 1..7）
        for msg in msgs.iter().skip(7) {
            assert!(msg["content"].is_string(), "末尾消息 content 应保持字符串");
        }

        // messages[1..4] 中的 3 条（indices 1, 2, 3）应有 cache_control（system 已用 1 个，剩 3 个配额）
        for (idx, msg) in msgs.iter().enumerate().take(4).skip(1) {
            let blocks = msg["content"]
                .as_array()
                .unwrap_or_else(|| panic!("idx={idx} content 应被转换为数组"));
            assert_eq!(blocks.len(), 1, "字符串 content 转换后应有 1 个 block");
            assert_eq!(
                blocks[0]["cache_control"]["type"], "ephemeral",
                "idx={idx} 应该有 cache_control"
            );
        }

        // messages[4..7] 应保持字符串（断点配额已用完，不再处理）
        for msg in msgs.iter().take(7).skip(4) {
            assert!(
                msg["content"].is_string(),
                "断点配额用完，content 应保持字符串"
            );
        }
    }

    /// 没有 system 提示时，所有断点都注入到 messages 上（最多 4 个）。
    #[test]
    fn inject_cache_breakpoints_no_system() {
        let mut system: Option<Vec<serde_json::Value>> = None;
        let mut msgs: Vec<serde_json::Value> = (0..10)
            .map(|i| {
                let role = if i % 2 == 0 { "user" } else { "assistant" };
                serde_json::json!({ "role": role, "content": format!("msg-{i}") })
            })
            .collect();

        inject_cache_breakpoints(&mut system, &mut msgs);

        // system 必须保持 None
        assert!(system.is_none());

        // 总断点数应 ≤ 4，且 = 4（无 system 时 messages 可用满 4 个配额）
        let total = count_cache_breakpoints(&system, &msgs);
        assert!(
            total <= MAX_CACHE_BREAKPOINTS,
            "总断点数应 ≤ 4，实际 = {total}"
        );

        assert_eq!(
            total, MAX_CACHE_BREAKPOINTS,
            "无 system 时 10 条消息应用满 4 个断点配额"
        );

        // 末尾 3 条不应有断点
        for msg in msgs.iter().skip(7) {
            assert!(msg["content"].is_string(), "末尾消息 content 应保持字符串");
        }

        // 首条 user（idx=0）不应有断点（被 skip(1) 跳过）
        assert!(
            msgs[0]["content"].is_string(),
            "首条 user content 应保持字符串"
        );

        // messages[1..5] 中的 4 条应有 cache_control
        for (idx, msg) in msgs.iter().enumerate().take(5).skip(1) {
            let blocks = msg["content"]
                .as_array()
                .unwrap_or_else(|| panic!("idx={idx} content 应被转换为数组"));
            assert_eq!(blocks.len(), 1);
            assert_eq!(
                blocks[0]["cache_control"]["type"], "ephemeral",
                "idx={idx} 应该有 cache_control"
            );
        }
    }

    /// system 含多个 content block → 只在最后一个 block 加 cache_control，
    /// 前面的 block 必须保持原样不变。
    #[test]
    fn inject_cache_breakpoints_mixed_blocks() {
        let mut system = Some(vec![
            serde_json::json!({ "type": "text", "text": "block 1" }),
            serde_json::json!({ "type": "text", "text": "block 2" }),
            serde_json::json!({ "type": "text", "text": "block 3" }),
        ]);
        // 给一个 ≥ 4 条消息的场景，确保 messages 也会被处理
        let mut msgs = vec![
            serde_json::json!({ "role": "user", "content": "q1" }),
            serde_json::json!({ "role": "assistant", "content": "a1" }),
            serde_json::json!({ "role": "user", "content": "q2" }),
            serde_json::json!({ "role": "assistant", "content": "a2" }),
            serde_json::json!({ "role": "user", "content": "q3" }),
        ];

        inject_cache_breakpoints(&mut system, &mut msgs);

        let sys_blocks = system.as_ref().unwrap();
        // block 1 / block 2 不应有 cache_control
        assert!(
            sys_blocks[0].get("cache_control").is_none(),
            "block 1 不应有 cache_control"
        );
        assert!(
            sys_blocks[1].get("cache_control").is_none(),
            "block 2 不应有 cache_control"
        );
        // 只有 block 3（最后一个）应有 cache_control
        assert_eq!(
            sys_blocks[2]["cache_control"]["type"], "ephemeral",
            "block 3 应有 cache_control"
        );

        // block 1 / 2 的原始 text 必须保留（未被覆盖）
        assert_eq!(sys_blocks[0]["text"], "block 1");
        assert_eq!(sys_blocks[1]["text"], "block 2");
        assert_eq!(sys_blocks[2]["text"], "block 3");
    }
}
