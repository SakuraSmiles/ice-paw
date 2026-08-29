//! [`ScreenChannel`] —— 屏幕共享通道：授权与可见性的单位（批次④ 步骤 1+3）。
//!
//! 通道不是物理管道（GDI 截文本就无状态、多会话各自截屏零冲突），它提供三件
//! 东西：**一份授权**（上收自逐工具 Confirm——见 [`short_circuit`]）、
//! **一个可见运行态**（HUD，步骤 2）、**一套写者仲裁**（单鼠标资源分配，本步）。
//!
//! 生命周期：进程级单例（[`global`]，先例 `ScreenState::global()`）、不持久化、
//! 重启即 Off。开/关由用户动作触发（聊天头开关 / `request_screen_session` 批准卡
//! / 终止键），通道生命周期事件走 tracing（`ice_paw.screen_channel`）不进
//! session_events——开/关无 conv/turn 容器，伪造 turn_id 会毒化 reconcile 的
//! turn 锚点分组（设计评审 A4，docs/computer-use-roadmap.md §4.11）。
//!
//! 步骤 3 落地：**单写者仲裁 + 暂停 + gate**（§4.3/§4.4）。读写分家——读（截屏）
//! 自由并发，写（鼠标/键盘注入）同一时刻只允许一个会话持有令牌；暂停时读写
//! 全部挂起。gate 的 park 是**取消感知**的（评审 B1 硬要求：暂停后「停止生成」
//! 必须能打断挂起，否则会话在 ChatState 注册表吊死）；排队是**无固定超时**的
//! park（评审 B3：超时报错会让排队者被 doom_detect 按签名连败终止）。回合结束
//! 归还挂 [`crate::harness::loop::emitter`] 的 `on_loop_exit`（RAII 全退出路径
//! 必经，评审 B6）。
//!
//! 线程模型：`std::sync::Mutex` 包整体，临界区只有 clone/insert（微秒级），
//! 持锁期间无 await——async 上下文安全（同 `ScreenState` 纪律）。唤醒用
//! `tokio::sync::watch` 版本号广播：每次状态突变 `bump()`，park 者醒来重查。
//! 锁序单向 `channel → chat_state`（活性回收在 channel 临界区内查 ChatState，
//! 无反向路径，无死锁面）。

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};

use serde::Serialize;

use crate::error::{AppError, AppResult};
use crate::harness::authority::AuthorizationDecision;
use crate::infra::cancel::CancellationToken;

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
    /// 暂停（§4.4：读写全部挂起，通道/授权保持）。
    paused: bool,
    /// 写者令牌（§4.3：空闲先到先得；持有粒度=回合，结束归还）。
    token: WriteToken,
    /// 等待写令牌的会话（FIFO；用户手动切换的出口）。
    queue: VecDeque<String>,
}

/// 写者令牌状态。
#[derive(Debug, Clone, PartialEq)]
enum WriteToken {
    Free,
    Held(String),
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
            paused: false,
            token: WriteToken::Free,
            queue: VecDeque::new(),
        }
    }

    /// 授予写令牌并清掉自己的陈旧排队位。
    /// **不变式 `Held(x) ⇒ x ∉ queue`**：活性回收路径（曾入队→持有者死亡→回收
    /// 再授予）不清理会留下「持有者排在等自己」的陈旧位——释放时会把自己
    /// 从队头再授予一次。所有 token 写入点统一走本方法。
    fn grant_token_to(&mut self, conv_id: &str) {
        self.queue.retain(|c| c != conv_id);
        self.token = WriteToken::Held(conv_id.to_string());
    }
}

/// 屏幕共享通道（全局单例见 [`global`]；测试用 `ScreenChannel::new()` 隔离）。
pub struct ScreenChannel {
    inner: Mutex<Option<Active>>,
    /// 状态版本号广播：gate park 者订阅，任何突变 `bump()` 唤醒重查。
    /// `send_modify` 永不失败（无接收者也不丢版本），唤醒语义只依赖版本变化。
    version: tokio::sync::watch::Sender<u64>,
    /// 写者活性查询源（持有者泄漏回收 §4.3）：lib.rs 初始化时注入 ChatState。
    /// 未注入（测试/启动早期）时保守视为「活着」——不回收，宁可用户手动切换。
    liveness: Mutex<Option<crate::harness::chat_state::ChatState>>,
}

impl ScreenChannel {
    pub fn new() -> Self {
        let (tx, _rx) = tokio::sync::watch::channel(0u64);
        // _rx 故意 drop：watch Sender 无接收者也能 send_modify，gate 侧按需 subscribe。
        drop(_rx);
        Self {
            inner: Mutex::new(None),
            version: tx,
            liveness: Mutex::new(None),
        }
    }

    /// 注入活性查询源（lib.rs 启动时一次；重复注入以后者为准——测试重排用）。
    pub fn set_liveness(&self, cs: crate::harness::chat_state::ChatState) {
        *self.liveness.lock().unwrap_or_else(|e| e.into_inner()) = Some(cs);
    }

    fn holder_alive(&self, conv_id: &str) -> bool {
        self.liveness
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_none_or(|cs| cs.is_streaming(conv_id))
    }

    /// 状态突变后的唤醒广播（在临界区内调用安全——watch send_modify 无锁竞争面）。
    fn bump(&self) {
        self.version.send_modify(|v| *v += 1);
    }

    /// 开启通道（已开则仅附着本会话——§4.1 request_screen_session 批准语义）。
    /// 返回 true = 发生了 Off→Active 转换（供 tracing/事件决策）。
    pub fn open(&self, conv_id: &str, info: AttachInfo) -> bool {
        let mut g = self.lock();
        let newly = g.is_none();
        let active = g.get_or_insert_with(Active::new);
        active.attached.insert(conv_id.to_string(), info);
        drop(g);
        self.bump();
        newly
    }

    /// 仅在通道已 Active 时附着本会话（首用批准路径：批准加入 ≠ 开启通道）。
    /// 返回 true = 实际新附着（Off / 已附着返回 false，幂等）。
    pub fn attach_if_active(&self, conv_id: &str, info: AttachInfo) -> bool {
        let mut g = self.lock();
        let attached = match g.as_mut() {
            Some(active) => active.attached.insert(conv_id.to_string(), info).is_none(),
            None => false,
        };
        drop(g);
        self.bump();
        attached
    }

    /// 关闭通道。返回被清空的附着名单（Off 状态调用返回 None）。
    /// ScreenState 的坐标基准不随通道清空（坐标与通道生命周期解耦，§4.4）。
    /// park 中的 gate / 排队者被 bump 唤醒后重查发现 Off → 家族错误（§4.4 终止）。
    pub fn stop(&self) -> Option<Vec<(String, AttachInfo)>> {
        let mut g = self.lock();
        let cleared = g.take().map(|a| a.attached.into_iter().collect());
        drop(g);
        self.bump();
        if cleared.is_some() {
            tracing::info!(target: "ice_paw.screen_channel", "通道关闭（令牌/队列随状态清空）");
        }
        cleared
    }

    /// 暂停（读写全部挂起；通道/授权/附着保持——播放器语义 §4.4）。
    pub fn pause(&self) {
        let mut g = self.lock();
        if let Some(a) = g.as_mut() {
            a.paused = true;
        }
        drop(g);
        self.bump();
    }

    /// 恢复（park 中的读写 gate 被唤醒继续）。
    pub fn resume(&self) {
        let mut g = self.lock();
        if let Some(a) = g.as_mut() {
            a.paused = false;
        }
        drop(g);
        self.bump();
    }

    /// 读 gate（§4.3 读写分家：截图自由并发，只受通道状态约束）。
    /// Off 首入直接过（§4.1 入口 3 向后兼容：授权回落逐次 Confirm）；
    /// 暂停 → 取消感知的 park（评审 B1）；域内被 Off → 家族错误。
    pub async fn gate_read(&self, cancel: Option<&CancellationToken>) -> AppResult<()> {
        self.gate_impl("", false, cancel).await
    }

    /// 写 gate（单写者令牌）：Free→授予；Held(本会话)→过；Held(他者)→排队 park。
    /// 排队无固定超时（评审 B3：超时报错会被 doom_detect 按签名连败终止）。
    pub async fn gate_write(
        &self,
        conv_id: &str,
        cancel: Option<&CancellationToken>,
    ) -> AppResult<()> {
        self.gate_impl(conv_id, true, cancel).await
    }

    /// gate 统一体：`write=false` 为读（conv_id 不参与）；`write=true` 为写。
    /// park 形态（评审 B1 硬要求）：select { watch 唤醒, 对话取消 }——
    /// 用户暂停后点「停止生成」必须能打断挂起，否则 ChatState 注册表吊死。
    async fn gate_impl(
        &self,
        conv_id: &str,
        write: bool,
        cancel: Option<&CancellationToken>,
    ) -> AppResult<()> {
        // subscribe 必须先于状态检查：检查时已见目标状态则直接过，
        // 检查后突变则 changed() 立即就绪重查——无丢唤醒窗口。
        let mut rx = self.version.subscribe();
        let mut entered = false; // 曾见 Active = 已进入通道仲裁域
        loop {
            {
                let mut g = self.lock();
                let Some(a) = g.as_mut() else {
                    if entered {
                        return Err(AppError::Validation(
                            "screen 通道已关闭: 用户结束了屏幕共享，本操作未执行——\
                             如需继续请让用户重新开启屏幕共享（聊天头开关）"
                                .into(),
                        ));
                    }
                    return Ok(()); // Off 兼容路径：不进通道域，逐次 Confirm 兜授权
                };
                entered = true;
                if a.paused {
                    // park（读写都挂起，§4.4）
                } else if !write {
                    return Ok(()); // 读：通道 Active 且未暂停即过
                } else {
                    match a.token.clone() {
                        WriteToken::Free => {
                            a.grant_token_to(conv_id);
                            return Ok(());
                        }
                        WriteToken::Held(ref h) if h == conv_id => return Ok(()),
                        WriteToken::Held(h) => {
                            // 活性回收（§4.3）：持有者回合已结束而令牌仍 Held = 泄漏。
                            // 只读查询 ChatState，不触碰持有者会话（用户拍板：无跨会话操作）。
                            if !self.holder_alive(&h) {
                                tracing::warn!(
                                    target: "ice_paw.screen_channel",
                                    holder = %h, waiter = %conv_id,
                                    "写者令牌泄漏回收（持有者会话已不在流式注册表）"
                                );
                                a.token = WriteToken::Free;
                                continue; // 重走 Free 分支授予本会话
                            }
                            // 排队：去重入队，park 等授予（用户手动 grant / 持有者归还）
                            if !a.queue.contains(&conv_id.to_string()) {
                                a.queue.push_back(conv_id.to_string());
                                tracing::info!(
                                    target: "ice_paw.screen_channel",
                                    holder = %h, waiter = %conv_id,
                                    queue_len = a.queue.len(),
                                    "写操作排队等待令牌"
                                );
                            }
                        }
                    }
                }
            }
            // park：等状态突变唤醒，或对话取消打断（B1）。
            // wait_cancel_safe(None) 永不完成——select 退化为纯 watch 等待。
            tokio::select! {
                changed = rx.changed() => {
                    let _ = changed;
                    continue;
                }
                _ = wait_cancel_safe(cancel) => {
                    // 取消：摘除自己的排队（死会话占队列，B6 同款问题）再返回
                    let mut g = self.lock();
                    if let Some(a) = g.as_mut() {
                        a.queue.retain(|c| c != conv_id);
                    }
                    drop(g);
                    return Err(AppError::Validation(
                        "screen 通道已暂停: 等待屏幕共享恢复期间对话被用户取消，本操作未执行"
                            .into(),
                    ));
                }
            }
        }
    }

    /// 归还写令牌 + 摘除本会话排队（回合结束/会话退出钩子挂 `on_loop_exit`）。
    /// 归属检查（评审 B6）：仅 `Held(本会话)` 才清——防与手动授予竞态互踩。
    /// 归还时按队列顺序授予队头（防后来者插队），队列空则回 Free；
    /// 被授予者出队（`Held(x) ⇒ x ∉ queue` 不变式）。
    pub fn release_write(&self, conv_id: &str) {
        let mut g = self.lock();
        let mut changed = false;
        if let Some(a) = g.as_mut() {
            if a.token == WriteToken::Held(conv_id.to_string()) {
                a.token = a
                    .queue
                    .pop_front()
                    .map(WriteToken::Held)
                    .unwrap_or(WriteToken::Free);
                changed = true;
            }
            let before = a.queue.len();
            a.queue.retain(|c| c != conv_id);
            changed |= a.queue.len() != before;
        }
        drop(g);
        if changed {
            self.bump();
        }
    }

    /// 用户手动切换令牌（HUD 队列「授予」）：目标会话立即持有；原持有者（若有）
    /// 作为普通排队者入队尾（评审 B9：不入队即丢失唤醒），不中止其回合。
    pub fn grant(&self, conv_id: &str) {
        let mut g = self.lock();
        let Some(a) = g.as_mut() else {
            return;
        };
        let prev = a.token.clone();
        if prev == WriteToken::Held(conv_id.to_string()) {
            return; // 已是持有者，幂等
        }
        a.queue.retain(|c| c != conv_id);
        if let WriteToken::Held(prev_holder) = prev {
            a.queue.push_back(prev_holder);
        }
        a.token = WriteToken::Held(conv_id.to_string());
        drop(g);
        self.bump();
    }

    /// 写操作的排队情报注记（§4.3「队列情报对模型可见」）：park 中的模型无法
    /// 自行放弃，「排不到就友善终止/先读/告知用户」的决策点只能发生在下一次
    /// 思考——靠结果里的这份快照支撑。无争用（队列空）返回 None 静默。
    pub fn contention_note(&self) -> Option<String> {
        let g = self.lock();
        let a = g.as_ref()?;
        if a.queue.is_empty() {
            return None;
        }
        let waiting: Vec<String> = a.queue.iter().cloned().collect();
        drop(g);
        Some(format!(
            "另有会话正在排队等待屏幕操作权（{}）——本会话回合结束（写权自动归还）前\
             它们无法操作屏幕；若不再需要持续操作，可尽早收束回合",
            waiting.join("、")
        ))
    }

    /// 摘除附着（会话主动退出/清理）：连带归还令牌与排队位。
    pub fn detach(&self, conv_id: &str) {
        let was_attached = {
            let mut g = self.lock();
            g.as_mut().is_some_and(|a| a.attached.remove(conv_id).is_some())
        };
        if was_attached {
            tracing::info!(target: "ice_paw.screen_channel", conv = %conv_id, "会话脱离通道");
            self.release_write(conv_id);
        }
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
                    paused: a.paused,
                    opened_at: Some(a.opened_at_unix),
                    hud_monitor: 0,
                    attached,
                    holder: match &a.token {
                        WriteToken::Free => None,
                        WriteToken::Held(h) => Some(h.clone()),
                    },
                    queue: a.queue.iter().cloned().collect(),
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

/// gate park 的取消臂：None 时挂起永不完成——select 退化为纯 watch 等待
/// （同步路径 gate 不接受取消打断，与 keyboard.rs wait 的 `if let Some` 同款形态）。
async fn wait_cancel_safe(cancel: Option<&CancellationToken>) {
    match cancel {
        Some(t) => crate::harness::tool_executor::wait_for_cancel(t).await,
        None => std::future::pending::<()>().await,
    }
}

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

    // ---------------- 步骤 3：gate / 令牌 / 排队 FSM（评审 B14 无头先行） ----------------

    use std::time::Duration;

    /// 轮询等待 snapshot 满足谓词（park 中的对端任务需要一拍调度才能入队）。
    async fn until<F: Fn(&ScreenChannelState) -> bool>(ch: &ScreenChannel, pred: F) {
        for _ in 0..500 {
            if pred(&ch.snapshot()) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        panic!("snapshot 条件 1s 内未达成: {:?}", ch.snapshot());
    }

    fn err_text(e: AppError) -> String {
        match e {
            AppError::Validation(m) => m,
            other => format!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn gate_off_first_entry_passes_for_compat() {
        let ch = ScreenChannel::new();
        // Off 首入 = 向后兼容路径（§4.1 入口 3）：不进通道域，授权回落逐次 Confirm
        ch.gate_read(None).await.expect("Off 首入读应过（兼容）");
        ch.gate_write("c1", None).await.expect("Off 首入写应过（兼容）");
        // 不产生任何通道状态（无令牌、无排队）
        let s = ch.snapshot();
        assert_eq!(s.status, "off");
        assert_eq!(s.holder, None);
        assert!(s.queue.is_empty());
    }

    #[tokio::test]
    async fn gate_write_free_grants_reentrant_read_concurrent() {
        let ch = ScreenChannel::new();
        ch.open("c1", info("a"));
        // 读：Active 即过，不取令牌
        ch.gate_read(None).await.expect("读 gate 应直接过");
        assert_eq!(ch.snapshot().holder, None);
        // 写：Free → 立即持有
        ch.gate_write("c1", None).await.expect("Free 应立即授予");
        assert_eq!(ch.snapshot().holder.as_deref(), Some("c1"));
        // 同会话重入（一回合多次工具调用）
        ch.gate_write("c1", None).await.expect("持有者重入应过");
        assert_eq!(ch.snapshot().queue.len(), 0);
    }

    #[tokio::test]
    async fn contention_queues_fifo_release_wakes_next_and_pops_queue() {
        let ch = std::sync::Arc::new(ScreenChannel::new());
        ch.open("c1", info("a"));
        ch.open("c2", info("b"));
        ch.open("c3", info("c"));
        ch.gate_write("c1", None).await.expect("先到先得");

        let h2 = {
            let ch = ch.clone();
            tokio::spawn(async move { ch.gate_write("c2", None).await })
        };
        until(&ch, |s| s.queue.contains(&"c2".to_string())).await;
        let h3 = {
            let ch = ch.clone();
            tokio::spawn(async move { ch.gate_write("c3", None).await })
        };
        until(&ch, |s| s.queue == vec!["c2".to_string(), "c3".to_string()]).await;

        // 归还：队头 c2 得令牌并出队（holder ∉ queue 不变式）
        ch.release_write("c1");
        h2.await.expect("c2 任务未 panic").expect("c2 应获授予");
        until(&ch, |s| s.holder.as_deref() == Some("c2") && s.queue == vec!["c3".to_string()]).await;

        ch.release_write("c2");
        h3.await.expect("c3 任务未 panic").expect("c3 应获授予");
        assert_eq!(ch.snapshot().holder.as_deref(), Some("c3"));
        assert_eq!(ch.snapshot().queue.len(), 0);
        // 全部归还 → Free
        ch.release_write("c3");
        assert_eq!(ch.snapshot().holder, None);
    }

    #[tokio::test]
    async fn cancel_interrupts_park_and_dequeues() {
        let ch = std::sync::Arc::new(ScreenChannel::new());
        ch.open("c1", info("a"));
        ch.open("c2", info("b"));
        ch.gate_write("c1", None).await.expect("先到先得");

        let token = CancellationToken::new();
        let h = {
            let ch = ch.clone();
            let t = token.clone();
            tokio::spawn(async move { ch.gate_write("c2", Some(&t)).await })
        };
        until(&ch, |s| s.queue.contains(&"c2".to_string())).await;

        token.cancel();
        let err = h.await.expect("c2 任务未 panic").expect_err("取消应中断 park");
        assert!(err_text(err).starts_with("screen 通道已暂停"), "取消错误家族前缀漂移");
        // 摘除自己的排队位，不惊动持有者
        until(&ch, |s| !s.queue.contains(&"c2".to_string())).await;
        assert_eq!(ch.snapshot().holder.as_deref(), Some("c1"));
    }

    #[tokio::test]
    async fn stop_during_park_wakes_with_closed_error() {
        let ch = std::sync::Arc::new(ScreenChannel::new());
        ch.open("c1", info("a"));
        ch.open("c2", info("b"));
        ch.gate_write("c1", None).await.expect("先到先得");

        let h = {
            let ch = ch.clone();
            tokio::spawn(async move { ch.gate_write("c2", None).await })
        };
        until(&ch, |s| s.queue.contains(&"c2".to_string())).await;

        ch.stop();
        let err = h.await.expect("c2 任务未 panic").expect_err("stop 应唤醒 park 并 Err");
        assert!(err_text(err).starts_with("screen 通道已关闭"), "关闭错误家族前缀漂移");
    }

    #[tokio::test]
    async fn pause_parks_read_write_resume_unblocks() {
        let ch = std::sync::Arc::new(ScreenChannel::new());
        ch.open("c1", info("a"));
        ch.pause();

        let h_read = {
            let ch = ch.clone();
            tokio::spawn(async move { ch.gate_read(None).await })
        };
        let h_write = {
            let ch = ch.clone();
            tokio::spawn(async move { ch.gate_write("c1", None).await })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        // 暂停期间读写都挂起（不完成也不报错）
        assert!(!h_read.is_finished());
        assert!(!h_write.is_finished());
        assert!(ch.snapshot().paused);

        ch.resume();
        h_read.await.expect("读任务未 panic").expect("恢复后读应过");
        h_write.await.expect("写任务未 panic").expect("恢复后写应过");
        assert!(!ch.snapshot().paused);
    }

    #[tokio::test]
    async fn grant_switches_holder_requeues_previous_at_tail() {
        let ch = std::sync::Arc::new(ScreenChannel::new());
        ch.open("c1", info("a"));
        ch.open("c2", info("b"));
        ch.open("c3", info("c"));
        ch.gate_write("c1", None).await.expect("先到先得");

        let h3 = {
            let ch = ch.clone();
            tokio::spawn(async move { ch.gate_write("c3", None).await })
        };
        until(&ch, |s| s.queue.contains(&"c3".to_string())).await;

        // 用户手动切给 c3：c3 立即持有，原持有者 c1 入队尾（评审 B9——不入队即丢唤醒）
        ch.grant("c3");
        h3.await.expect("c3 任务未 panic").expect("手动授予应放行 c3");
        until(&ch, |s| s.holder.as_deref() == Some("c3") && s.queue == vec!["c1".to_string()]).await;

        // 已是持有者再 grant 幂等
        ch.grant("c3");
        assert_eq!(ch.snapshot().holder.as_deref(), Some("c3"));

        // c1 重新获得（c3 归还 → 队头 c1）
        ch.release_write("c3");
        until(&ch, |s| s.holder.as_deref() == Some("c1") && s.queue.is_empty()).await;
    }

    #[tokio::test]
    async fn release_write_ownership_check() {
        let ch = ScreenChannel::new();
        ch.open("c1", info("a"));
        ch.open("c2", info("b"));
        ch.gate_write("c1", None).await.expect("先到先得");

        // 非持有者归还无效（评审 B6：防与手动授予竞态互踩）
        ch.release_write("c2");
        assert_eq!(ch.snapshot().holder.as_deref(), Some("c1"));

        ch.release_write("c1");
        assert_eq!(ch.snapshot().holder, None);
    }

    #[tokio::test]
    async fn liveness_reclaim_dead_holder_and_conservative_default() {
        let ch = std::sync::Arc::new(ScreenChannel::new());
        ch.open("c1", info("a"));
        ch.open("c2", info("b"));
        ch.gate_write("c1", None).await.expect("先到先得");

        // 未注入 liveness（启动早期/测试默认）：保守不回收，c2 排队
        let h = {
            let ch = ch.clone();
            tokio::spawn(async move { ch.gate_write("c2", None).await })
        };
        until(&ch, |s| s.queue.contains(&"c2".to_string())).await;

        // 注入后 c1 仍活着（注册表中）→ 不回收：c2 依旧排队、令牌不动。
        // 无 bump 则 park 者不醒，断言确定性成立。
        let cs = crate::harness::chat_state::ChatState::new();
        cs.register("c1", CancellationToken::new());
        ch.set_liveness(cs);
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(ch.snapshot().holder.as_deref(), Some("c1"));
        assert!(ch.snapshot().queue.contains(&"c2".into()));
        h.abort();

        // c1 回合结束（不在注册表）而令牌仍 Held = 泄漏 → c2 直接回收获取。
        // abort 留下的陈旧排队位由授予点清理（`Held(x) ⇒ x ∉ queue` 不变式）。
        let cs2 = crate::harness::chat_state::ChatState::new();
        ch.set_liveness(cs2);
        ch.gate_write("c2", None).await.expect("死持有者应被回收");
        assert_eq!(ch.snapshot().holder.as_deref(), Some("c2"));
        // 回收授予清掉了陈旧排队位（不变式）
        assert_eq!(ch.snapshot().queue.len(), 0);
    }

    #[tokio::test]
    async fn detach_releases_token_and_queue_slot() {
        let ch = std::sync::Arc::new(ScreenChannel::new());
        ch.open("c1", info("a"));
        ch.open("c2", info("b"));
        ch.gate_write("c1", None).await.expect("先到先得");

        let h = {
            let ch = ch.clone();
            tokio::spawn(async move { ch.gate_write("c2", None).await })
        };
        until(&ch, |s| s.queue.contains(&"c2".to_string())).await;

        // c2 主动脱离：排队位被摘（先 abort——park 任务被 detach 的 bump 唤醒后
        // 会重查并重新排队，摘位测试须用死任务才是确定性的）
        h.abort();
        ch.detach("c2");
        until(&ch, |s| !s.queue.contains(&"c2".to_string())).await;
        assert!(!ch.is_attached("c2"));

        // c1 脱离：令牌归还
        ch.detach("c1");
        assert_eq!(ch.snapshot().holder, None);
        // 未附着会话 detach 幂等
        ch.detach("other");
        assert!(ch.is_active());
    }

    #[tokio::test]
    async fn snapshot_reflects_pause_holder_queue() {
        let ch = ScreenChannel::new();
        ch.open("c1", info("a"));
        ch.gate_write("c1", None).await.expect("先到先得");
        ch.pause();
        let s = ch.snapshot();
        assert!(s.paused);
        assert_eq!(s.holder.as_deref(), Some("c1"));
        // stop 清空后 off 形态不带残留
        ch.stop();
        let s = ch.snapshot();
        assert_eq!(s.status, "off");
        assert!(!s.paused);
        assert_eq!(s.holder, None);
        assert!(s.queue.is_empty());
    }
}
