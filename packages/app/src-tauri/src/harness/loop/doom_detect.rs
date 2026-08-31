//! P10④ doom_loop 检测：同工具 + 同错误签名连续失败跟踪
//!
//! 背景（2026-08-22 失败样本诊断）：`stuck_detect` 按轮指纹（累计文本 + 工具调用
//! 签名集合）判停滞——「每次换一个文件名重试同类失败」的循环（如 write_file 连写
//! 8 个不存在目录下的文件）每轮签名都变，指纹恒变，永不触发，只能靠预算熔断兜底。
//!
//! 方案：在 tool_result 出口（loop_engine 阶段 E 之后）按「工具名 + [`error_kind`]」
//! 统计连续失败：
//! - 连败 [`NUDGE_AT`] 次起**逐次**在该条 tool_result 尾部追加纠正指令（不终止——
//!   给模型按恢复阶梯自救的机会；错误信息本身已是行为契约，指令只负责「停止再猜」；
//!   D15 前是 `==` 只提醒一次，弱模型连吃 3 次同类失败仍只被纠正一次）；
//! - 连败 [`TERMINATE_AT`] 次 → 终止回合（finish_reason="doom_loop"，对称清场剔除
//!   本轮 tool_use），防止无视指令的循环纯烧 token。
//!
//! 同工具任一次成功 → 该工具全部计数清零（模型已找到可用调用方式，不是死循环）。

use std::collections::HashMap;

/// 连败达到该次数 → 在 tool_result 尾部注入纠正指令
pub(crate) const NUDGE_AT: u32 = 3;
/// 连败达到该次数 → 终止回合
pub(crate) const TERMINATE_AT: u32 = 6;

/// 编译期锁定不变式：先 nudge 后终止（改阈值漏改语义时直接编译失败）
const _: () = assert!(TERMINATE_AT > NUDGE_AT);

/// 错误签名：首行截到首个冒号（含中文全角）为止。
///
/// 工具错误文案的稳定前缀（如「文件不存在」「write_file 写入失败」）即错误家族；
/// 其后的路径/原因各不相同——正好把「换文件名的同类失败」折叠成同一签名。裸 io
/// 错误（如「系统找不到指定的路径。 (os error 3)」）无冒号则取整行，同样稳定。
pub(crate) fn error_kind(err: &str) -> &str {
    let first_line = err.lines().next().unwrap_or("");
    match first_line.find([':', '：']) {
        Some(i) => first_line[..i].trim(),
        None => first_line.trim(),
    }
}

/// 纠正指令（追加到命中的 tool_result 尾部）。
///
/// 不复述错误本身（tool_result 已含全文），只给行为指令：停 → 按指引修正 → 只调
/// 一次 → 前提有误就核实或求助。控制在 4 行内（Codex A1 简洁默认）。
///
/// D15 八波⑤升级：连败超过 [`NUDGE_AT`]（loop_engine 按 `>=` 逐次注入）时追加
/// 升级段——停止一切重试、改为结构化报告。委派子会话的最终回复经 TurnSummary
/// 回传统筹者，同一措辞对人类/委派两语境通用。
pub(crate) fn nudge_text(tool: &str, streak: u32) -> String {
    let base = format!(
        "\n\n[System] {tool} 已连续 {streak} 次以同类方式失败。停止用同样方式重试：\
1) 重读上方错误信息，按其中的恢复指引修正参数；2) 修正后只调用一次并检查结果；\
3) 若错误反复指向同一前提（如路径不存在、依赖缺失），先用只读工具核实，或向用户说明障碍后再继续。"
    );
    if streak > NUDGE_AT {
        format!(
            "{base}\n[升级指令] 已连续 {streak} 次失败，超过提醒线。若这一轮修正后仍失败：\
停止用任何方式重试，直接在回复中报告 ①已完成什么 ②反复失败的工具与完整错误 ③剩余部分，\
由用户或委派方决定下一步。"
        )
    } else {
        base
    }
}

/// 连续失败跟踪器（每回合一个实例，跨工具轮存活）。
pub(crate) struct DoomLoopTracker {
    /// `"{tool}|{kind}"` → 连续失败次数
    streaks: HashMap<String, u32>,
}

impl DoomLoopTracker {
    pub(crate) fn new() -> Self {
        Self {
            streaks: HashMap::new(),
        }
    }

    /// 记一次失败。返回该签名当前连败数（调用方据此决定 nudge / 终止）。
    pub(crate) fn record_failure(&mut self, tool: &str, err_content: &str) -> u32 {
        let key = format!("{tool}|{}", error_kind(err_content));
        let n = self.streaks.entry(key).or_insert(0);
        *n += 1;
        *n
    }

    /// 记一次成功：同工具全部签名清零（找到可用方式 ≠ 死循环）。
    pub(crate) fn record_success(&mut self, tool: &str) {
        let prefix = format!("{tool}|");
        self.streaks.retain(|k, _| !k.starts_with(&prefix));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 生产案例形态：write_file 连败但路径各不相同——错误家族（首行冒号前）相同
    #[test]
    fn error_kind_strips_varying_paths() {
        let a = error_kind("write_file 写入失败: D:/a/1.svg: 系统找不到指定的路径。 (os error 3)");
        let b = error_kind("write_file 写入失败: D:/a/2.svg: 系统找不到指定的路径。 (os error 3)");
        assert_eq!(a, b);
        assert_eq!(a, "write_file 写入失败");

        // 全角冒号同样截断
        assert_eq!(error_kind("文件不存在：x.rs。请核对"), "文件不存在");

        // 无冒号取整行（裸 io 错误形态）
        assert_eq!(
            error_kind("系统找不到指定的路径。 (os error 3)"),
            "系统找不到指定的路径。 (os error 3)"
        );

        // 多行错误只看首行
        assert_eq!(error_kind("第一行: x\n第二行"), "第一行");
    }

    #[test]
    fn streak_accumulates_across_varying_paths() {
        let mut t = DoomLoopTracker::new();
        // 同工具同家族、不同路径：计数应连续累积（stuck_detect 正是漏掉这种形态）
        assert_eq!(t.record_failure("write_file", "写入失败: a.svg: os error 3"), 1);
        assert_eq!(t.record_failure("write_file", "写入失败: b.svg: os error 3"), 2);
        assert_eq!(t.record_failure("write_file", "写入失败: c.svg: os error 3"), 3);
    }

    #[test]
    fn different_error_families_count_independently() {
        let mut t = DoomLoopTracker::new();
        t.record_failure("write_file", "写入失败: a.svg: os error 3");
        assert_eq!(t.record_failure("write_file", "参数解析失败: xxx"), 1);
    }

    #[test]
    fn different_tools_count_independently() {
        let mut t = DoomLoopTracker::new();
        t.record_failure("write_file", "写入失败: a.svg: os error 3");
        assert_eq!(t.record_failure("edit_file", "写入失败: a.svg: os error 3"), 1);
    }

    #[test]
    fn success_resets_that_tool_only() {
        let mut t = DoomLoopTracker::new();
        t.record_failure("write_file", "写入失败: a: os error 3");
        t.record_failure("write_file", "写入失败: b: os error 3");
        t.record_failure("edit_file", "文件不存在: c");
        t.record_success("write_file");
        assert_eq!(t.record_failure("write_file", "写入失败: d: os error 3"), 1);
        assert_eq!(t.record_failure("edit_file", "文件不存在: e"), 2);
    }

    #[test]
    fn nudge_text_names_tool_and_count() {
        let s = nudge_text("write_file", 3);
        assert!(s.contains("write_file"));
        assert!(s.contains("3 次"));
        assert!(s.contains("停止用同样方式重试"));
        // 恰在提醒线：无升级段
        assert!(!s.contains("[升级指令]"), "streak==NUDGE_AT 是首轮提醒：{s}");
    }

    /// D15 八波⑤：连败超线后 nudge 带升级段（停止一切重试 + 结构化报告三件套）
    #[test]
    fn nudge_text_escalates_beyond_reminder_line() {
        for streak in [NUDGE_AT + 1, NUDGE_AT + 2, TERMINATE_AT - 1] {
            let s = nudge_text("edit_docx", streak);
            assert!(s.contains(&format!("{streak} 次以同类方式失败")), "{s}");
            assert!(s.contains("[升级指令]"), "streak={streak} 须带升级段：{s}");
            assert!(s.contains("停止用任何方式重试"), "{s}");
            assert!(s.contains("①已完成什么"), "结构化报告三件套：{s}");
            assert!(s.contains("②反复失败的工具与完整错误"), "{s}");
            assert!(s.contains("③剩余部分"), "{s}");
        }
    }
}
