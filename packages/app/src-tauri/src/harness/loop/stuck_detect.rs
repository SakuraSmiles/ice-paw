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
}
