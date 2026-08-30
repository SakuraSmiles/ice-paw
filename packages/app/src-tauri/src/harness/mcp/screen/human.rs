//! 人类优先仲裁（§4.5）：物理输入在场判定 + 抢占避让的数据源。
//!
//! Windows 桌面只有一个系统光标，真·多指针做不到（无内核驱动）；实现的是
//! **人类优先避让**——用户随时可夺回鼠标，agent 永远避让：
//! - **判别**：WH_MOUSE_LL + WH_KEYBOARD_LL 低级钩子（专线程 + 消息泵）。
//!   注入事件（SendInput）带 `LLMHF_INJECTED` / `LLKHF_INJECTED` 标志——回调
//!   只登记**非注入**事件的时间戳，即「物理输入 = 人类在场」。
//! - **回调纪律**：回调内只做一次时间戳写（微秒级），不做任何重活——系统会
//!   摘除超时钩子（~300ms），也绝不能在回调里 SendInput 同类事件（重入）。
//! - **消费**：仲裁全在工具协程侧——写 gate 见 [`super::channel`]（human_active
//!   → park）；原子序列每步后非阻塞检查 [`triggered`]（命中 → 安全收尾 +
//!   家族错误 `screen 用户抢占`，见 mod.rs 各工具）。
//! - **生命周期**：通道开启时装钩、关闭时卸（进程退出系统自动回收）。安装
//!   失败（安全软件）诚实降级——无时间戳 = 恒不活跃，功能不损只失避让。
//!
//! 测试缝：`set_fake_active`（thread-local 覆盖在场判定）与 `set_fake_preempt`
//! （thread-local 覆盖检查点谓词，供 Fake 后端按事件计数翻转测「序列中途
//! 抢占→安全收尾」）。工具/通道测试跑在 `#[tokio::test]` 默认 current_thread
//! runtime，与断言同线程——thread-local 即测试作用域，零并行污染（全局时间戳
//! 是进程级的，直接注入会串测试）。

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// 去抖窗口：最近该时长内有物理输入 = 人类在场（§4.5：2s）。
const HUMAN_QUIESCE_MS: u64 = 2000;

/// 窗口取值（cfg!(test) 缩放，act_shot 同纪律：生产真实值，测试快）。
/// channel 的 park 心跳臂也用它做自醒周期（见 [`super::channel`]）。
pub(super) fn quiesce_duration() -> Duration {
    if cfg!(test) {
        Duration::from_millis(2)
    } else {
        Duration::from_millis(HUMAN_QUIESCE_MS)
    }
}

/// 最近一次物理输入时刻（hook 线程写，查询侧读；None = 从未见过/钩子未装）。
static LAST_INPUT: Mutex<Option<Instant>> = Mutex::new(None);

/// 登记一次物理输入（hook 回调调用；也供真机冒烟/诊断手动注入）。
pub fn note_human_input_now() {
    let mut g = LAST_INPUT.lock().unwrap_or_else(|e| e.into_inner());
    *g = Some(Instant::now());
}

/// 人类在场（去抖窗口内有物理输入）。查询方：写 gate / 原子序列检查点 /
/// 通道 snapshot 的 human_active 字段。
pub fn active() -> bool {
    #[cfg(test)]
    {
        // 测试覆盖：thread-local 布尔直判（Some(true)=恒在场 / Some(false)=恒
        // 空闲 / None=解除覆盖走真实时间戳）。固定结果无时间抖动。
        if let Some(v) = FAKE_ACTIVE.with(|c| c.get()) {
            return v;
        }
    }
    let g = LAST_INPUT.lock().unwrap_or_else(|e| e.into_inner());
    g.is_some_and(|t| t.elapsed() < quiesce_duration())
}

/// 原子序列检查点的非阻塞查询（§4.5「每步插值后检查」）：当前即 [`active`]，
/// 独立命名留给未来差异化（如把检查降频/带滞回）——语义锚点。
pub fn triggered() -> bool {
    active()
}

/// 原子序列检查点谓词：**通道 Active 且**人类在场。
/// 仅通道 Active 时生效——Off 兼容路径的逐次 Confirm 就是那次操作的全部授权
/// （用户亲手点批准即人类在场，此时抢占判定会误杀刚被批准的操作）。
/// 通道侧入口见 [`super::channel::ScreenChannel::human_preempted`]。
pub fn preempt_now(channel_active: bool) -> bool {
    #[cfg(test)]
    {
        // 测试覆盖：thread-local 直判，绕过通道单例（进程级 global 通道被测试
        // 打开会串并行测试——检查点行为用本缝隔离验证）。
        if let Some(v) = FAKE_PREEMPT.with(|c| c.get()) {
            return v;
        }
    }
    channel_active && triggered()
}

/// 抢占家族错误（三段式：发生了什么 + 为什么 + 怎么办）。
/// 家族前缀 `screen 用户抢占` 稳定（doom_detect 冒号切分依赖首段）。
pub fn preempted_error(detail: &str) -> crate::error::AppError {
    crate::error::AppError::Validation(format!(
        "screen 用户抢占: 检测到用户正在使用鼠标/键盘，操作已中止{detail}——\
         用户是屏幕的唯一主人；稍后（用户停止操作约 {} 秒内）可重试",
        HUMAN_QUIESCE_MS / 1000
    ))
}

#[cfg(test)]
thread_local! {
    static FAKE_ACTIVE: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
    static FAKE_PREEMPT: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub mod test_support {
    //! 测试注入缝（thread-local，见模块文档）。
    use super::{FAKE_ACTIVE, FAKE_PREEMPT};

    /// 覆盖人类在场判定：Some(true/false) 固定结果，None 解除覆盖。
    pub fn set_fake_active(v: Option<bool>) {
        FAKE_ACTIVE.with(|c| c.set(v));
    }

    /// 覆盖检查点谓词 [`super::preempt_now`]：Some(true/false) 固定结果，
    /// None 解除覆盖。Fake 后端按事件计数翻转本缝可测「序列中途抢占→
    /// 安全收尾」路径，无需触碰进程级通道单例。
    pub fn set_fake_preempt(v: Option<bool>) {
        FAKE_PREEMPT.with(|c| c.set(v));
    }
}

// =========================================================================
// LL 钩子线程（仅 Windows；装/卸由通道生命周期驱动）
// =========================================================================

#[cfg(windows)]
mod hooks {
    use std::sync::mpsc;
    use std::sync::Mutex;

    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, GetMessageW, KBDLLHOOKSTRUCT, LLKHF_INJECTED, LLMHF_INJECTED, MSG,
        MSLLHOOKSTRUCT, PostThreadMessageW, SetWindowsHookExW, UnhookWindowsHookEx,
        WH_KEYBOARD_LL, WH_MOUSE_LL, WM_QUIT,
    };

    /// 本地常量（VK 码表同纪律：不 import windows-sys 常量，防跨版本漂移）。
    const HC_ACTION: i32 = 0;

    struct HookThread {
        thread_id: u32,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    static INSTALLED: Mutex<Option<HookThread>> = Mutex::new(None);

    unsafe extern "system" fn mouse_proc(code: i32, wparam: usize, lparam: isize) -> isize {
        if code == HC_ACTION {
            let info = &*(lparam as *const MSLLHOOKSTRUCT);
            // 只登记物理输入：注入事件（SendInput/其它自动化）带 INJECTED 标志。
            if info.flags & LLMHF_INJECTED == 0 {
                super::note_human_input_now();
            }
        }
        CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
    }

    unsafe extern "system" fn kbd_proc(code: i32, wparam: usize, lparam: isize) -> isize {
        if code == HC_ACTION {
            let info = &*(lparam as *const KBDLLHOOKSTRUCT);
            if info.flags & LLKHF_INJECTED == 0 {
                super::note_human_input_now();
            }
        }
        CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
    }

    /// 安装 LL 钩子（幂等）。返回 false = 安装失败（诚实降级：无时间戳=恒不活跃）。
    /// 专线程跑消息泵——LL 钩子回调只在装钩线程有消息循环时被调用。
    pub fn install() -> bool {
        let mut g = INSTALLED.lock().unwrap_or_else(|e| e.into_inner());
        if g.is_some() {
            return true;
        }
        let (tx, rx) = mpsc::channel::<Result<u32, String>>();
        let handle = std::thread::Builder::new()
            .name("screen-human-hook".into())
            .spawn(move || unsafe {
                let mouse = SetWindowsHookExW(
                    WH_MOUSE_LL,
                    Some(mouse_proc),
                    std::ptr::null_mut(),
                    0,
                );
                let kbd = SetWindowsHookExW(
                    WH_KEYBOARD_LL,
                    Some(kbd_proc),
                    std::ptr::null_mut(),
                    0,
                );
                if mouse.is_null() || kbd.is_null() {
                    if !mouse.is_null() {
                        UnhookWindowsHookEx(mouse);
                    }
                    if !kbd.is_null() {
                        UnhookWindowsHookEx(kbd);
                    }
                    let _ = tx.send(Err("SetWindowsHookExW 失败（安全软件拦截？）".into()));
                    return;
                }
                let _ = tx.send(Ok(GetCurrentThreadId()));
                // 消息泵：GetMessageW 在 WM_QUIT 时返回 0 退出循环。
                // MSG 全字段为数值/指针，zeroed 是合法初值（无 Default）。
                let mut msg: MSG = std::mem::zeroed();
                while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {}
                UnhookWindowsHookEx(mouse);
                UnhookWindowsHookEx(kbd);
            });
        match (handle, rx.recv()) {
            (Ok(handle), Ok(Ok(thread_id))) => {
                *g = Some(HookThread {
                    thread_id,
                    handle: Some(handle),
                });
                true
            }
            (Ok(handle), Ok(Err(e))) => {
                tracing::warn!(
                    target: "ice_paw.screen_channel",
                    error = %e,
                    "人类输入钩子安装失败——抢占避让降级为不可用（屏幕操作不受影响）"
                );
                let _ = handle.join();
                false
            }
            (Ok(handle), Err(_)) => {
                // 线程 panic 于 send 之前：join 拿回 panic 打印
                let _ = handle.join();
                false
            }
            (Err(e), _) => {
                tracing::warn!(target: "ice_paw.screen_channel", error = %e, "人类输入钩子线程创建失败");
                false
            }
        }
    }

    /// 卸载（幂等）。向钩子线程投递 WM_QUIT 并等它摘钩退出。
    pub fn uninstall() {
        let mut g = INSTALLED.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(mut h) = g.take() {
            unsafe {
                PostThreadMessageW(h.thread_id, WM_QUIT, 0, 0);
            }
            if let Some(handle) = h.handle.take() {
                let _ = handle.join();
            }
        }
    }
}

#[cfg(windows)]
pub fn install() -> bool {
    if cfg!(test) {
        // 测试构建不装真钩子：LL 钩子是进程级的，cargo test 并行跑会装出真
        // 时间戳（开发机上的人体鼠标运动会串测试）。active() 走真实空时间戳
        // = 恒 false；human.rs 自身测试用 note+窗口衰减覆盖真实路径。
        return false;
    }
    hooks::install()
}

#[cfg(not(windows))]
pub fn install() -> bool {
    // 非 Windows：无钩子能力，恒不活跃（诚实降级，同 Unsupported 后端）
    false
}

#[cfg(windows)]
pub fn uninstall() {
    if cfg!(test) {
        return;
    }
    hooks::uninstall();
}

#[cfg(not(windows))]
pub fn uninstall() {}

// =========================================================================
// 单测（thread-local 覆盖；真实时间戳路径用 note+elapsed 边界验证）
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_without_input_is_false_and_decays() {
        // 全局时间戳为 None（或很久以前）→ 不在场
        test_support::set_fake_active(None);
        assert!(!active());
        // note 后在窗口内 → 在场；窗口外 → 复位（cfg test 窗口 2ms，睡过即衰减）
        note_human_input_now();
        assert!(active());
        std::thread::sleep(Duration::from_millis(6));
        assert!(!active(), "去抖窗口过后应复位");
        test_support::set_fake_active(None);
    }

    #[test]
    fn fake_active_overrides_for_tests() {
        test_support::set_fake_active(Some(true));
        assert!(active());
        assert!(triggered());
        test_support::set_fake_active(Some(false));
        assert!(!active());
        test_support::set_fake_active(None);
    }

    #[test]
    fn preempt_now_requires_both_channel_and_human() {
        test_support::set_fake_active(Some(true));
        assert!(!preempt_now(false), "通道 Off（兼容路径）不抢占");
        assert!(preempt_now(true));
        test_support::set_fake_active(Some(false));
        assert!(!preempt_now(true), "人类闲置不抢占");
        test_support::set_fake_active(None);
    }

    #[test]
    fn fake_preempt_overrides_for_tests() {
        test_support::set_fake_preempt(Some(true));
        assert!(preempt_now(false), "检查点测试缝应绕过通道判定");
        test_support::set_fake_preempt(Some(false));
        assert!(!preempt_now(true));
        test_support::set_fake_preempt(None);
        // 解除覆盖后回落真实谓词
        test_support::set_fake_active(Some(true));
        assert!(preempt_now(true));
        test_support::set_fake_active(None);
    }

    #[test]
    fn preempted_error_family_prefix() {
        let msg = match preempted_error("并已释放按住的鼠标按钮") {
            crate::error::AppError::Validation(m) => m,
            other => format!("{other:?}"),
        };
        assert!(msg.starts_with("screen 用户抢占"), "家族前缀稳定: {msg}");
    }
}
