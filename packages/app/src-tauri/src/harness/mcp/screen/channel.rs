//! [`ScreenChannel`] —— 屏幕共享通道：授权与可见性的单位（批次④ 步骤 1）。
//!
//! 通道不是物理管道（GDI 截文本就无状态、多会话各自截屏零冲突），它提供三件
//! 东西：**一份授权**（上收自逐工具 Confirm——见 [`short_circuit`]）、
//! **一个可见运行态**（HUD，步骤 2）、**一套写者仲裁**（单鼠标资源分配，步骤 3）。
//!
//! 生命周期：进程级单例（[`global`]，先例 `ScreenState::global()`）、不持久化、
//! 重启即 Off。开/关由用户动作触发（聊天头开关 / `request_screen_session` 批准卡
//! / 终止键），通道生命周期事件走 tracing（`ice_paw.screen_channel`）不进
//! session_events——开/关无 conv/turn 容器，伪造 turn_id 会毒化 reconcile 的
//! turn 锚点分组（设计评审 A4，docs/computer-use-roadmap.md §4.11）。
//!
//! 步骤 1 语义边界：只有 status + attached 真生效；`paused` / `hud_monitor` /
//! `holder` / `queue` / `human_active` 是协议全量形状的占位（§4.9 单一全量事件
//! 从第一天就定型，前端/HUD 不随步骤演进改协议），对应机制在步骤 2-4 落地。
//!
//! 线程模型：`std::sync::Mutex` 包整体，临界区只有 clone/insert（微秒级），
//! 持锁期间无 await——async 上下文安全（同 `ScreenState` 纪律）。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use serde::Serialize;

use crate::harness::authority::AuthorizationDecision;

/// computer-use 家族固定集合（register_builtin 注册的 10 个内置名）。
/// 授权短路只对这批生效；`request_screen_session` 是通道入口本身（Confirm
/// 是它的存在意义），刻意不在集合内。
pub const SCREEN_TOOLS: &[&str] = &[
    "capture_screen",
    "capture_window",
    "list_windows",
    "mouse_move",
    "mouse_click",
    "mouse_drag",
    "mouse_scroll",
    "type_text",
    "press_key",
    "wait",
];

/// 附着会话信息（HUD「谁在用」；步骤 1 无 HUD，先供状态事件与日志）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AttachInfo {
    pub agent_name: String,
    pub conv_title: String,
    /// 「正在做什么」= 当前回合用户指令摘要（§4.2）；步骤 2 接 turn 上下文。
    pub purpose: String,
}

/// 附着会话（状态事件负载形态，键值拍平）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AttachedConv {
    pub conv_id: String,
    pub agent_name: String,
    pub conv_title: String,
    pub purpose: String,
}

/// `screen:channel-state` 单一全量事件负载（§4.9，形状跨步骤稳定）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ScreenChannelState {
    pub status: &'static str,
    pub paused: bool,
    /// 开启时刻（unix 秒；HUD 时长显示用，无自动过期 §4.6）。
    pub opened_at: Option<u64>,
    pub hud_monitor: usize,
    /// 按 conv_id 稳定排序（HashMap 序不定，负载须可断言/可渲染稳定）。
    pub attached: Vec<AttachedConv>,
    /// 写者令牌持有会话（步骤 3 起有值）。
    pub holder: Option<String>,
    /// 等待写令牌的会话（步骤 3 起有值）。
    pub queue: Vec<String>,
    /// 最近 2s 有物理输入（人类优先仲裁，步骤 4 起有值）。
    pub human_active: bool,
    /// 通道累计截图张数（capture 成功即计，HUD 成本块可选件）。
    pub screenshot_count: u64,
}

impl ScreenChannelState {
    fn off() -> Self {
        Self {
            status: "off",
            paused: false,
            opened_at: None,
            hud_monitor: 0,
            attached: Vec::new(),
            holder: None,
            queue: Vec::new(),
            human_active: false,
            screenshot_count: 0,
        }
    }
}

struct Active {
    opened_at_unix: u64,
    attached: HashMap<String, AttachInfo>,
    screenshot_count: u64,
}

impl Active {
    fn new() -> Self {
        Self {
            opened_at_unix: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            attached: HashMap::new(),
            screenshot_count: 0,
        }
    }
}

/// 屏幕共享通道（全局单例见 [`global`]；测试用 `ScreenChannel::new()` 隔离）。
pub struct ScreenChannel {
    inner: Mutex<Option<Active>>,
}

impl ScreenChannel {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }

    /// 开启通道（已开则仅附着本会话——§4.1 request_screen_session 批准语义）。
    /// 返回 true = 发生了 Off→Active 转换（供 tracing/事件决策）。
    pub fn open(&self, conv_id: &str, info: AttachInfo) -> bool {
        let mut g = self.lock();
        let newly = g.is_none();
        let active = g.get_or_insert_with(Active::new);
        active.attached.insert(conv_id.to_string(), info);
        newly
    }

    /// 仅在通道已 Active 时附着本会话（首用批准路径：批准加入 ≠ 开启通道）。
    /// 返回 true = 实际新附着（Off / 已附着返回 false，幂等）。
    pub fn attach_if_active(&self, conv_id: &str, info: AttachInfo) -> bool {
        let mut g = self.lock();
        match g.as_mut() {
            Some(active) => active.attached.insert(conv_id.to_string(), info).is_none(),
            None => false,
        }
    }

    /// 关闭通道。返回被清空的附着名单（Off 状态调用返回 None）。
    /// ScreenState 的坐标基准不随通道清空（坐标与通道生命周期解耦，§4.4）。
    pub fn stop(&self) -> Option<Vec<(String, AttachInfo)>> {
        let mut g = self.lock();
        g.take().map(|a| a.attached.into_iter().collect())
    }

    pub fn is_active(&self) -> bool {
        self.lock().is_some()
    }

    pub fn is_attached(&self, conv_id: &str) -> bool {
        self.lock()
            .as_ref()
            .is_some_and(|a| a.attached.contains_key(conv_id))
    }

    /// 短路热路径（每次 Confirm 决策都会问一次）：单次锁查询。
    pub fn is_active_and_attached(&self, conv_id: &str) -> bool {
        self.lock()
            .as_ref()
            .is_some_and(|a| a.attached.contains_key(conv_id))
    }

    /// 授权短路（§4.11）：computer-use 家族工具 && 通道 Active && 会话已附着 →
    /// Confirm 覆盖为 Allow——用户开启通道/批准加入的动作即知情同意。
    /// **只吃 Confirm，不碰 Deny**（评审 A1：无条件覆盖会越过显式永久拒绝）。
    pub fn short_circuit(
        &self,
        decision: AuthorizationDecision,
        tool_name: &str,
        conv_id: &str,
    ) -> AuthorizationDecision {
        if matches!(decision, AuthorizationDecision::Confirm { .. })
            && SCREEN_TOOLS.contains(&tool_name)
            && self.is_active_and_attached(conv_id)
        {
            AuthorizationDecision::Allow
        } else {
            decision
        }
    }

    /// 截图计数（capture 成功后调用；只计数不广播——§4.9 低频广播纪律）。
    pub fn note_screenshot(&self) {
        if let Some(a) = self.lock().as_mut() {
            a.screenshot_count += 1;
        }
    }

    pub fn snapshot(&self) -> ScreenChannelState {
        let g = self.lock();
        match g.as_ref() {
            None => ScreenChannelState::off(),
            Some(a) => {
                let mut attached: Vec<AttachedConv> = a
                    .attached
                    .iter()
                    .map(|(k, v)| AttachedConv {
                        conv_id: k.clone(),
                        agent_name: v.agent_name.clone(),
                        conv_title: v.conv_title.clone(),
                        purpose: v.purpose.clone(),
                    })
                    .collect();
                attached.sort_by(|x, y| x.conv_id.cmp(&y.conv_id));
                ScreenChannelState {
                    status: "active",
                    paused: false,
                    opened_at: Some(a.opened_at_unix),
                    hud_monitor: 0,
                    attached,
                    holder: None,
                    queue: Vec::new(),
                    human_active: false,
                    screenshot_count: a.screenshot_count,
                }
            }
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Active>> {
        // 同 ScreenState 纪律：临界区无 panic 源，中毒即上游已损坏，沿用语义。
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Default for ScreenChannel {
    fn default() -> Self {
        Self::new()
    }
}

static GLOBAL: OnceLock<Arc<ScreenChannel>> = OnceLock::new();

/// 进程级共享实例（tool_executor 短路 / 命令层 / request_screen_session 共用）。
pub fn global() -> Arc<ScreenChannel> {
    GLOBAL.get_or_init(|| Arc::new(ScreenChannel::new())).clone()
}

/// 生产便捷形态：读全局单例做短路（tool_executor 调用点）。
pub fn short_circuit(
    decision: AuthorizationDecision,
    tool_name: &str,
    conv_id: &str,
) -> AuthorizationDecision {
    global().short_circuit(decision, tool_name, conv_id)
}

/// 广播 `screen:channel-state`（全窗；主窗开关与未来 HUD 同源渲染）。
pub fn emit_state(app: &tauri::AppHandle) {
    use tauri::Emitter as _;
    let _ = app.emit("screen:channel-state", global().snapshot());
}

/// 广播 `screen:channel-closed`（终止归因；§4.9）。
pub fn emit_closed(app: &tauri::AppHandle, reason: &str) {
    use tauri::Emitter as _;
    let _ = app.emit("screen:channel-closed", serde_json::json!({ "reason": reason }));
}

/// 附着信息 best-effort 查库（查不到给诚实占位，不阻塞通道动作）。
pub async fn attach_info_from_db(
    pool: &sqlx::SqlitePool,
    agent_id: &str,
    conv_id: &str,
) -> AttachInfo {
    let conv = crate::db::repo::conversation::get_by_id(pool, conv_id).await.ok();
    let agent = crate::db::repo::agent::get_by_id(pool, agent_id).await.ok();
    AttachInfo {
        agent_name: agent
            .map(|a| a.name)
            .unwrap_or_else(|| "未知 agent".into()),
        conv_title: conv.map(|c| c.title).unwrap_or_default(),
        purpose: String::new(),
    }
}

// =========================================================================
// 单测（隔离实例；不碰 global——单例残留会串测试）
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn info(name: &str) -> AttachInfo {
        AttachInfo {
            agent_name: name.into(),
            conv_title: format!("会话-{name}"),
            purpose: String::new(),
        }
    }

    #[test]
    fn open_off_to_active_then_stop_clears() {
        let ch = ScreenChannel::new();
        assert!(!ch.is_active());
        assert!(ch.open("c1", info("a")));
        assert!(ch.is_active());
        assert!(ch.is_attached("c1"));
        // 已开再 open（他者加入）= 仅附着，不重复「开启」
        assert!(!ch.open("c2", info("b")));
        assert!(ch.is_attached("c2"));

        let cleared = ch.stop().expect("Active 状态 stop 应返回被清空名单");
        assert_eq!(cleared.len(), 2);
        assert!(!ch.is_active());
        assert!(!ch.is_attached("c1"));
        // Off 状态重复 stop 幂等
        assert!(ch.stop().is_none());
    }

    #[test]
    fn attach_if_active_respects_gate() {
        let ch = ScreenChannel::new();
        // 通道 Off：不附着（首用批准 ≠ 开通道）
        assert!(!ch.attach_if_active("c1", info("a")));
        assert!(!ch.is_attached("c1"));

        ch.open("c0", info("opener"));
        assert!(ch.attach_if_active("c1", info("a")));
        // 已附着幂等
        assert!(!ch.attach_if_active("c1", info("a")));
    }

    #[test]
    fn snapshot_shape_and_stable_order() {
        let ch = ScreenChannel::new();
        assert_eq!(ch.snapshot().status, "off");
        assert_eq!(ch.snapshot().attached.len(), 0);

        ch.open("c2", info("b"));
        ch.open("c1", info("a"));
        ch.note_screenshot();
        ch.note_screenshot();
        let s = ch.snapshot();
        assert_eq!(s.status, "active");
        assert!(s.opened_at.is_some());
        // conv_id 排序保证负载稳定
        assert_eq!(s.attached[0].conv_id, "c1");
        assert_eq!(s.attached[1].conv_id, "c2");
        assert_eq!(s.screenshot_count, 2);
        // 计数不随 stop 归零后残留——stop 后 snapshot 是 off 形状
        ch.stop();
        assert_eq!(ch.snapshot().screenshot_count, 0);
    }

    #[test]
    fn short_circuit_overrides_only_family_confirm_when_attached() {
        let ch = ScreenChannel::new();
        let confirm = || AuthorizationDecision::Confirm {
            request_id: "r".into(),
            tool_name: "capture_screen".into(),
            file_path: String::new(),
            arguments: String::new(),
            reason: "…".into(),
        };

        // 通道 Off：Confirm 原样
        assert!(matches!(
            ch.short_circuit(confirm(), "capture_screen", "c1"),
            AuthorizationDecision::Confirm { .. }
        ));

        ch.open("c1", info("a"));
        // 家族 + Active + 已附着 → Allow
        assert!(matches!(
            ch.short_circuit(confirm(), "capture_screen", "c1"),
            AuthorizationDecision::Allow
        ));
        assert!(matches!(
            ch.short_circuit(confirm(), "mouse_click", "c1"),
            AuthorizationDecision::Allow
        ));
        // 未附着会话（含家族工具）→ 不短路
        assert!(matches!(
            ch.short_circuit(confirm(), "capture_screen", "other"),
            AuthorizationDecision::Confirm { .. }
        ));
        // 非家族工具（如写文件）→ 不短路
        assert!(matches!(
            ch.short_circuit(confirm(), "write_file", "c1"),
            AuthorizationDecision::Confirm { .. }
        ));
        // request_screen_session 自身不短路（通道入口的 Confirm 是它的语义）
        assert!(matches!(
            ch.short_circuit(confirm(), "request_screen_session", "c1"),
            AuthorizationDecision::Confirm { .. }
        ));
        // Deny 永不被覆盖（评审 A1）
        let deny = AuthorizationDecision::Deny {
            reason: "显式拒绝".into(),
        };
        assert!(matches!(
            ch.short_circuit(deny, "capture_screen", "c1"),
            AuthorizationDecision::Deny { .. }
        ));
        // Allow 原样（本就放行，无短路意义）
        assert!(matches!(
            ch.short_circuit(AuthorizationDecision::Allow, "capture_screen", "c1"),
            AuthorizationDecision::Allow
        ));
    }
}
