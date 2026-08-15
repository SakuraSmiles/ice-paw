//! 停滞检测：连续 N 轮无文本进展 → 终止生成
//!
//! 从 `harness::loop_engine` 拆出，方便独立测试。

/// 计算本轮的"进度指纹"hash
///
/// 把 `all_text`（累计文本）和 `completed_calls` 的工具调用签名
///（`name:arguments` 字符串，由调用方在传入前 `sort_unstable()`）一起喂入
/// 64-bit hasher，产出一个稳定指纹。任何一项变化都会得到不同 hash。
///
/// 为什么用 hash 而不是直接字符串比较：
///   - 多轮工具调用后 `all_text` 可能累积数千字，逐字比较是 O(n²)
///   - 64-bit hasher 碰撞概率 ~1/2^64，足够鲁棒
///   - `DefaultHasher` 是 std 自带、无依赖
///
/// 工具签名（而非实例 ID）参与计算是为了让"相同文本但不同工具参数"不计入停滞。
/// 注意：调用方必须在传入前对 Vec 排序，以消除上游 HashMap 迭代顺序不确定性。
pub(crate) fn compute_round_key(all_text: &str, completed_call_ids: &[String]) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    all_text.hash(&mut h);
    for id in completed_call_ids {
        id.hash(&mut h);
    }
    h.finish()
}

/// 纯函数形式的停滞判定（便于单元测试）
///
/// 输入：本轮进度指纹 + 上一轮指纹 + 当前连续未进展计数 + 阈值
/// 输出：`(new_counter, should_terminate)` —— 调用方负责把 new_counter 写回
///
/// 规则：
/// - 本轮 hash 与上一轮相同 → `new_counter = stuck_counter + 1`，否则归零
/// - 当 `new_counter >= threshold` → 触发终止
pub(crate) fn should_terminate_stuck(
    round_key: u64,
    last_round_hash: Option<u64>,
    stuck_counter: u32,
    threshold: u32,
) -> (u32, bool) {
    let no_progress = Some(round_key) == last_round_hash;
    let new_counter = if no_progress {
        stuck_counter.saturating_add(1)
    } else {
        0
    };
    let should_terminate = new_counter >= threshold;
    (new_counter, should_terminate)
}

// =========================================================================
// 单元测试（从 loop_engine.rs 迁入）
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::budget::LoopBudget;

    #[test]
    fn compute_round_key_stable_for_same_input() {
        let k1 = compute_round_key("hello", &["git:status".into()]);
        let k2 = compute_round_key("hello", &["git:status".into()]);
        assert_eq!(k1, k2);
    }

    #[test]
    fn compute_round_key_differs_for_different_text() {
        let k1 = compute_round_key("hello", &[]);
        let k2 = compute_round_key("world", &[]);
        assert_ne!(k1, k2);
    }

    #[test]
    fn compute_round_key_differs_for_different_tools() {
        let k1 = compute_round_key("x", &["a".into()]);
        let k2 = compute_round_key("x", &["b".into()]);
        assert_ne!(k1, k2);
    }

    #[test]
    fn stuck_zero_when_progress() {
        let (counter, term) = should_terminate_stuck(1, Some(2), 5, 3);
        assert_eq!(counter, 0); // hash 不同 → 清零
        assert!(!term);
    }

    #[test]
    fn stuck_increments_when_no_progress() {
        let (counter, term) = should_terminate_stuck(1, Some(1), 2, 5);
        assert_eq!(counter, 3); // 相同 hash → +1
        assert!(!term); // 未达阈值
    }

    #[test]
    fn stuck_terminates_at_threshold() {
        let (counter, term) = should_terminate_stuck(1, Some(1), 2, 3);
        assert_eq!(counter, 3);
        assert!(term); // counter >= threshold
    }

    #[test]
    fn stuck_first_round_no_previous_hash() {
        let (counter, term) = should_terminate_stuck(42, None, 0, 3);
        assert_eq!(counter, 0);
        assert!(!term);
    }

    // ========================================================================
    // M2.1: 停滞检测单元测试（B1-4，从 loop_engine.rs 迁入）
    //
    // 全部使用已提取出的纯函数 `should_terminate_stuck` / `compute_round_key`
    // 测试，不依赖任何 IO / Tauri / DB，CI 友好且可在毫秒内跑完。
    // ========================================================================

    /// T1: stuck_threshold 默认值应为 5（dev1 评审：默认 3 误判率过高）
    #[test]
    fn stuck_detection_threshold_defaults_to_five() {
        let budget = LoopBudget::default();
        assert_eq!(
            budget.stuck_threshold, 5,
            "stuck_threshold 默认值应为 5（M2.1 修改）"
        );
    }

    /// T2: stuck_threshold 字段可被自定义覆盖
    #[test]
    fn stuck_detection_custom_threshold_accepted() {
        let budget = LoopBudget {
            max_tool_rounds: 5,
            max_attempts: 4,
            stuck_threshold: 7,
            max_total_tokens: 128_000,
            ..LoopBudget::default()
        };
        assert_eq!(budget.stuck_threshold, 7, "自定义 stuck_threshold 应被接受");

        // 验证自定义阈值在判定函数中能正确生效
        // 首轮 last_hash=None → counter=0；后续轮同 hash 累加
        // 7 轮连续相同 hash（threshold=7 时 counter 最多到 6）不应触发
        let mut counter: u32 = 0;
        let mut last_hash: Option<u64> = None;
        let key = compute_round_key("hello", &[]);
        for round in 1..=6 {
            let (c, terminate) = should_terminate_stuck(key, last_hash, counter, 7);
            counter = c;
            last_hash = Some(key);
            // 第 1 轮 counter=0，第 2~6 轮 counter=1..5
            let expected = if round == 1 { 0 } else { (round - 1) as u32 };
            assert_eq!(counter, expected, "第 {} 轮 counter={}", round, expected);
            assert!(!terminate, "前 6 轮在 threshold=7 时不应触发");
        }
        // 第 7 轮 counter=6 < 7，仍不触发
        let (c, terminate) = should_terminate_stuck(key, last_hash, counter, 7);
        assert_eq!(c, 6);
        assert!(!terminate, "第 7 轮 counter=6 仍 < threshold=7");
        // 第 8 轮 counter=7 达到阈值，应触发
        let (final_counter, terminate) = should_terminate_stuck(key, last_hash, c, 7);
        assert_eq!(final_counter, 7);
        assert!(terminate, "第 8 轮无进展在 threshold=7 时应触发");
    }

    /// T3: 连续 N 轮无进展（hash 完全相同）触发 stuck
    ///
    /// 验证默认 threshold=5 场景：
    /// - 首轮 last_hash=None → counter 归零（无法比较）
    /// - 第 2~5 轮 hash 相同 → counter 累加到 1,2,3,4
    /// - 第 6 轮 counter=5 ≥ threshold → 触发停滞
    #[test]
    fn stuck_detection_triggers_after_n_rounds_with_no_progress() {
        let threshold: u32 = 5;
        let mut counter: u32 = 0;
        let mut last_hash: Option<u64> = None;
        // 同一文本 + 同一组工具调用 → hash 永远相同
        let key = compute_round_key("stuck text", &["tool-1".to_string()]);

        // 前 5 轮：不触发
        for round in 1..=5 {
            let (new_counter, terminate) = should_terminate_stuck(key, last_hash, counter, threshold);
            counter = new_counter;
            last_hash = Some(key);
            // 第 1 轮 counter=0，后续轮累加
            assert_eq!(
                counter,
                if round == 1 { 0 } else { (round - 1) as u32 },
                "第 {} 轮 counter={}",
                round,
                counter
            );
            assert!(!terminate, "前 5 轮不应触发（threshold=5）");
        }
        // 第 6 轮：counter=5 ≥ threshold → 触发
        let (final_counter, terminate) = should_terminate_stuck(key, last_hash, counter, threshold);
        assert_eq!(final_counter, 5);
        assert!(terminate, "第 6 轮（threshold=5）应触发停滞");
    }

    /// T4: 工具调用签名变化 → 计数器归零
    ///
    /// 验证 dev1 设计的 and-condition：仅文本相同但工具签名变了不算停滞。
    /// 用 hash 直接验证：相同文本 + 不同 `name:arguments` → hash 不同 → counter 归零。
    #[test]
    fn stuck_detection_resets_on_tool_call_change() {
        let threshold: u32 = 3;

        // 1) 前 3 轮：相同文本 + 相同工具签名 → counter 累加
        // 第 1 轮 None → 0；第 2,3 轮相同 → counter 累加到 2
        // P0-1 fix: 使用 name:arguments 格式（与生产代码一致）
        let key_a =
            compute_round_key("hello", &["read_file:{\"path\":\"/a\"}".to_string()]);
        let mut counter: u32 = 0;
        let mut last_hash: Option<u64> = None;
        for _ in 0..3 {
            let (c, _) = should_terminate_stuck(key_a, last_hash, counter, threshold);
            counter = c;
            last_hash = Some(key_a);
        }
        assert_eq!(counter, 2, "3 轮相同 hash 后 counter 应为 2（首轮 0 + 累加 2）");

        // 2) 第 4 轮：相同文本但换工具参数 → counter 应归零
        let key_b =
            compute_round_key("hello", &["read_file:{\"path\":\"/b\"}".to_string()]);
        assert_ne!(key_a, key_b, "不同工具签名应产出不同 hash");
        let (new_counter, terminate) = should_terminate_stuck(key_b, last_hash, counter, threshold);
        assert_eq!(new_counter, 0, "工具变化时 counter 应归零");
        assert!(!terminate, "工具变化时不应触发停滞");
    }

    /// T5: 文本变化 → 计数器归零
    ///
    /// 验证 hash 包含 all_text：哪怕工具签名也相同，只要文本增长就不算停滞。
    /// 同时验证首轮（last_hash=None）counter 归零。
    #[test]
    fn stuck_detection_resets_on_text_change() {
        let threshold: u32 = 3;

        // 1) 前 3 轮：相同文本 → counter 累加到 2
        // P0-1 fix: 使用 name:arguments 格式（与生产代码一致）
        let key_a =
            compute_round_key("part1", &["read_file:{\"path\":\"/a\"}".to_string()]);
        let mut counter: u32 = 0;
        let mut last_hash: Option<u64> = None;
        for _ in 0..3 {
            let (c, _) = should_terminate_stuck(key_a, last_hash, counter, threshold);
            counter = c;
            last_hash = Some(key_a);
        }
        assert_eq!(counter, 2);

        // 2) 第 4 轮：文本增长 → counter 应归零
        let key_b =
            compute_round_key("part1 part2", &["read_file:{\"path\":\"/a\"}".to_string()]);
        assert_ne!(key_a, key_b, "文本变化应产出不同 hash");
        let (new_counter, terminate) = should_terminate_stuck(key_b, last_hash, counter, threshold);
        assert_eq!(new_counter, 0, "文本变化时 counter 应归零");
        assert!(!terminate, "文本增长时不应触发停滞");

        // 3) 首轮（last_hash = None）counter 归零（无法比较）
        let (first_counter, terminate) = should_terminate_stuck(key_a, None, 0, 1);
        assert_eq!(first_counter, 0, "首轮 last_hash=None → counter 归零");
        assert!(!terminate, "首轮不应触发");

        // 4) 不同文本 + 相同工具签名：hash 必然不同
        let key_c =
            compute_round_key("different", &["read_file:{\"path\":\"/a\"}".to_string()]);
        assert_ne!(key_a, key_c);
        let (reset_counter, terminate) = should_terminate_stuck(key_c, Some(key_a), 5, 3);
        assert_eq!(reset_counter, 0, "文本变化重置 counter");
        assert!(!terminate);
    }

    /// T6: 相同工具调用集合在不同顺序下产出相同 hash（P1-1 fix 验证）
    ///
    /// 模拟生产代码：先把 `name:arguments` 字符串收集进 Vec，再 `sort_unstable()`。
    /// 验证排序后的 Vec 与原始顺序不同的 Vec 在 sort 后产出相同 hash。
    #[test]
    fn stuck_detection_hash_independent_of_iteration_order() {
        let call_keys = vec![
            "read_file:{\"path\":\"/a\"}".to_string(),
            "write_file:{\"path\":\"/b\"}".to_string(),
            "bash:{}".to_string(),
        ];
        // 逆序版（模拟 HashMap::into_values() 在不同 run 下的不同迭代顺序）
        let reversed: Vec<String> = {
            let mut v = call_keys.clone();
            v.reverse();
            v
        };
        // shuffle 版（更激进的顺序扰动）
        let mut shuffled = call_keys.clone();
        shuffled.swap(0, 2);

        let mut sorted_original = call_keys.clone();
        sorted_original.sort_unstable();
        let mut sorted_reversed = reversed.clone();
        sorted_reversed.sort_unstable();
        let mut sorted_shuffled = shuffled.clone();
        sorted_shuffled.sort_unstable();

        let key_original = compute_round_key("same text", &sorted_original);
        let key_reversed = compute_round_key("same text", &sorted_reversed);
        let key_shuffled = compute_round_key("same text", &sorted_shuffled);

        assert_eq!(
            key_original, key_reversed,
            "sort 后逆序集合与正序集合应产出相同 hash"
        );
        assert_eq!(
            key_original, key_shuffled,
            "sort 后乱序集合与正序集合应产出相同 hash"
        );
        // 反向断言：未排序的原始 Vec 应该产生不同 hash（验证 sort 是必要的）
        assert_ne!(
            compute_round_key("same text", &call_keys),
            compute_round_key("same text", &reversed),
            "未 sort 的不同顺序应产出不同 hash（sort 是必需的）"
        );
    }

    /// T7: 工具调用实例 ID 变化不影响进度指纹（P0-1 fix 核心意图验证）
    ///
    /// 模拟两轮：实例 ID 不同（toolu_aaa vs toolu_bbb），
    /// 但 name+args 相同。验证 hash 相等——证明实例 ID 不参与计算。
    #[test]
    fn stuck_detection_hash_ignores_instance_id() {
        // 生产代码现在使用 name:args（不含实例 ID），两轮输入完全相同
        let round1_keys = vec!["read_file:{\"path\":\"/etc/hosts\"}".to_string()];
        let round2_keys = vec!["read_file:{\"path\":\"/etc/hosts\"}".to_string()];

        let key1 = compute_round_key("result", &round1_keys);
        let key2 = compute_round_key("result", &round2_keys);
        assert_eq!(
            key1, key2,
            "name+args 相同时 hash 必须相等（实例 ID 不参与计算）"
        );

        // 反向断言：如果错误地混入实例 ID，hash 会不同（这里手工拼接错误格式演示）
        // 使用 P0-1 改前的错误格式（带 toolu_ 实例 ID），验证它确实导致 hash 不稳定
        let buggy_round1 = vec!["toolu_aaa:read_file:{\"path\":\"/etc/hosts\"}".to_string()];
        let buggy_round2 = vec!["toolu_bbb:read_file:{\"path\":\"/etc/hosts\"}".to_string()];
        let buggy_key1 = compute_round_key("result", &buggy_round1);
        let buggy_key2 = compute_round_key("result", &buggy_round2);
        assert_ne!(
            buggy_key1, buggy_key2,
            "错误格式（含实例 ID）应产出不同 hash——证明实例 ID 必须从 hash 输入中排除"
        );
    }
}
