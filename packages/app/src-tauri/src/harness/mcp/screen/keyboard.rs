//! computer use 操作工具 —— 键盘三件 + 节奏件 wait（阶段二第二刀）。
//!
//! - `type_text`：`KEYEVENTF_UNICODE` 逐字符注入——**绕过键盘布局**（中文/
//!   Emoji/任何 Unicode 直进，不依赖焦点窗口的输入语言）；BMP 外字符由
//!   工具层拆 UTF-16 代理对逐单元发。
//! - `press_key`：组合键字符串（如 `ctrl+shift+t`、`win+d`）→ 修饰键按下 →
//!   主键点按 → 修饰键逆序释放。VK 码表在 [`vk_for`] 本地定义（不 import
//!   windows-sys 常量——模块归属跨版本漂过，同 WHEEL_DELTA 教训）。
//! - `wait`：动作与截图之间的节奏件——select 取消令牌，用户点「停止生成」
//!   立即返回，不傻等满额。
//!
//! 键盘无坐标——作用于**当前焦点窗口**；模型流程 = capture → click 定位 →
//! type。type/press 全部 Confirm 级（模拟输入真实作用于用户机器）。
//!
//! **act-and-look**：type/press 成功后走 [`super::action_shot`] 附「操作效果」
//! 图（同区域重抓，即刻成为新坐标基准）——输入后是否落对位置，模型从附图
//! 直接判断，无需再 capture 一轮。**wait 刻意保持纯文本**：它是 Always 级
//! （无外部作用才免审批），静默附图会让画面在用户不知情下离开本机——授权
//! 治理优先于节奏便利（见 mod.rs 模块文档）。

use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;

use crate::error::{AppError, AppResult};

use super::backend::ScreenBackend;
use super::state::ScreenState;
use crate::harness::mcp::client::{McpClient, ToolContext, ToolOutput};
use crate::harness::mcp::types::AuthorizationLevel;

#[cfg(windows)]
use super::backend::GdiBackend;
#[cfg(not(windows))]
use super::backend::UnsupportedBackend;

/// 相邻字符注入间隔：给目标应用消化输入的余量（部分老应用逐事件处理会丢字）。
const TYPE_GAP_MS: u64 = 10;
/// 组合键内修饰键→主键→释放的间隔。
const KEY_GAP_MS: u64 = 10;
/// 多次点按之间的间隔。
const PRESS_GAP_MS: u64 = 60;
/// 单次 type_text 的字符上限（超长拆多次调用——一次塞一屏文本既慢又难重试）。
const MAX_TYPE_CHARS: usize = 2000;
/// wait 的合法区间（下限防 0 空转、上限防模型把回合卡半分钟以上）。
const WAIT_MIN_MS: u64 = 50;
const WAIT_MAX_MS: u64 = 10_000;

// =========================================================================
// VK 码表（本地定义——windows-sys 常量模块归属跨版本漂过）
// =========================================================================

/// 键名（小写）→ 虚拟键码。单字符支路：字母 a-z（0x41..）与数字 0-9（0x30..）。
fn vk_for(name: &str) -> Option<u16> {
    let n = name.to_ascii_lowercase();
    if n.len() == 1 {
        let c = n.as_bytes()[0];
        return if c.is_ascii_alphabetic() {
            Some(0x41 + (c - b'a') as u16)
        } else if c.is_ascii_digit() {
            Some(0x30 + (c - b'0') as u16)
        } else {
            None // 字面 '+' 是分隔符撞车位 → 用命名键 plus
        };
    }
    match n.as_str() {
        "ctrl" | "control" => Some(0x11),
        "shift" => Some(0x10),
        "alt" => Some(0x12),
        "win" | "meta" | "cmd" | "super" => Some(0x5B),
        "enter" | "return" => Some(0x0D),
        "esc" | "escape" => Some(0x1B),
        "tab" => Some(0x09),
        "space" => Some(0x20),
        "backspace" => Some(0x08),
        "delete" | "del" => Some(0x2E),
        "insert" => Some(0x2D),
        "left" | "arrowleft" => Some(0x25),
        "up" | "arrowup" => Some(0x26),
        "right" | "arrowright" => Some(0x27),
        "down" | "arrowdown" => Some(0x28),
        "home" => Some(0x24),
        "end" => Some(0x23),
        "pageup" => Some(0x21),
        "pagedown" => Some(0x22),
        "printscreen" => Some(0x2C),
        "capslock" => Some(0x14),
        "numlock" => Some(0x90),
        "scrolllock" => Some(0x91),
        "plus" | "equal" | "equals" => Some(0xBB),
        "minus" => Some(0xBD),
        "comma" => Some(0xBC),
        "period" | "dot" => Some(0xBE),
        _ => {
            // 功能键 f1..f12（0x70..=0x7B；f13+ 非标准，不认）
            if let Some(num) = n.strip_prefix('f') {
                let num: u16 = num.parse().ok()?;
                if (1..=12).contains(&num) {
                    return Some(0x70 + num - 1);
                }
            }
            None
        }
    }
}

/// 组合键解析：`"ctrl+shift+t"` → (`[ctrl, shift]`, `t`)。
/// 空段（`"ctrl+"`）或未识别键名 → 家族错误（错误文案自带词表，模型可自纠）。
fn parse_combo(combo: &str) -> AppResult<(Vec<u16>, u16)> {
    let parts: Vec<&str> = combo.split('+').map(str::trim).collect();
    if parts.iter().any(|p| p.is_empty()) {
        return Err(AppError::Validation(
            "screen 按键无效: 组合键里有空段（形如 \"ctrl+\"）——\
             格式为 \"ctrl+shift+t\"，修饰键在前、主键在末尾".into(),
        ));
    }
    let mut vks = Vec::with_capacity(parts.len());
    for p in &parts {
        match vk_for(p) {
            Some(vk) => vks.push(vk),
            None => {
                return Err(AppError::Validation(format!(
                    "screen 按键无效: 无法识别的键名「{p}」——支持的键：\
                     单字母 a-z / 数字 0-9 / 修饰键 ctrl·shift·alt·win / \
                     命名键 enter·esc·tab·space·backspace·delete·insert·\
                     up·down·left·right·home·end·pageup·pagedown·\
                     printscreen·capslock·numlock·plus·minus·功能键 f1-f12"
                )));
            }
        }
    }
    let (mods, main) = vks.split_at(vks.len() - 1);
    Ok((mods.to_vec(), main[0]))
}

/// wait 时长钳制（纯函数，独立可测）。
fn clamp_wait(ms: u64) -> u64 {
    ms.clamp(WAIT_MIN_MS, WAIT_MAX_MS)
}

/// 生产后端（type/press 用；wait 不碰后端）。
fn builtin_backend() -> Arc<dyn ScreenBackend> {
    #[cfg(windows)]
    {
        Arc::new(GdiBackend)
    }
    #[cfg(not(windows))]
    {
        Arc::new(UnsupportedBackend)
    }
}

/// 工具持有面（backend + 坐标基准状态；wait 无字段）。
macro_rules! screen_key_tool {
    ($name:ident) => {
        pub struct $name {
            backend: Arc<dyn ScreenBackend>,
            state: Arc<ScreenState>,
        }

        impl $name {
            /// 注入式构造（测试用 Fake 后端 + 隔离状态）。
            pub fn new(backend: Arc<dyn ScreenBackend>, state: Arc<ScreenState>) -> Self {
                Self { backend, state }
            }

            /// 生产构造（与看屏/鼠标工具共用进程级 ScreenState——同一坐标基准）。
            pub fn builtin() -> Self {
                Self {
                    backend: builtin_backend(),
                    state: super::state::global(),
                }
            }
        }
    };
}

screen_key_tool!(TypeTextTool);
screen_key_tool!(PressKeyTool);

// =========================================================================
// type_text
// =========================================================================

#[derive(Deserialize)]
struct TypeTextArgs {
    text: String,
}

#[async_trait]
impl McpClient for TypeTextTool {
    fn name(&self) -> &str {
        "type_text"
    }

    fn description(&self) -> &str {
        "Type text into whatever currently has keyboard focus (use mouse_click to focus an \
         input first). Works with any Unicode text (CJK, emoji) regardless of the target \
         window's keyboard layout. This sends real keystrokes to the user's machine — it \
         does not read or modify files directly. Max 2000 chars per call; split longer \
         content into multiple calls. The result includes a post-action screenshot showing \
         what the focused area looks like now — read it to confirm the text landed where \
         expected, no re-capture needed."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["text"],
            "properties": {
                "text": { "type": "string", "description": "Text to type at the current focus." }
            }
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::Confirm
    }

    fn auth_reason(&self) -> Option<String> {
        Some("将模拟键盘输入并回传操作后的屏幕截图给当前模型服务商（内容会真实输入到你机器上当前聚焦的窗口）".into())
    }

    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(
            "type_text 必须通过 execute_with_output 调用（需要 conv_id 记录输入日志 + 回传附图）".into(),
        ))
    }

    async fn execute_with_output(&self, args: &str, ctx: &ToolContext) -> AppResult<ToolOutput> {
        let p: TypeTextArgs =
            serde_json::from_str(args)
                .map_err(|e| AppError::Validation(format!("type_text 参数解析失败: {e}")))?;
        let chars = p.text.chars().count();
        if chars == 0 {
            return Err(AppError::Validation(
                "screen 输入参数无效: text 为空——省略输入内容没有意义；\
                 若想清空输入框，用 press_key 组合 ctrl+a 后 delete".into(),
            ));
        }
        if chars > MAX_TYPE_CHARS {
            return Err(AppError::Validation(format!(
                "screen 输入参数无效: text 有 {chars} 字符，超过单次 {MAX_TYPE_CHARS} 上限——\
                 拆成多次 type_text 调用（也便于失败重试）"
            )));
        }

        // 逐 UTF-16 单元 down+up（BMP 外字符 = 代理对两单元）。
        let mut units = 0usize;
        for unit in p.text.encode_utf16() {
            self.backend.key_unicode(unit, true)?;
            self.backend.key_unicode(unit, false)?;
            units += 1;
            tokio::time::sleep(Duration::from_millis(TYPE_GAP_MS)).await;
        }

        tracing::info!(
            target: "ice_paw.screen",
            conv = %ctx.conv_id, chars, units,
            preview = %&p.text.chars().take(30).collect::<String>(),
            "type_text 成功"
        );
        let mut echo = serde_json::Map::new();
        echo.insert("action".into(), "type_text".into());
        echo.insert("chars".into(), serde_json::json!(chars));
        echo.insert("utf16_units".into(), serde_json::json!(units));
        let shot = super::action_shot(&self.backend, &self.state, ctx).await;
        Ok(super::finish_action_output(echo, shot))
    }
}

// =========================================================================
// press_key
// =========================================================================

#[derive(Deserialize)]
struct PressKeyArgs {
    combo: String,
    #[serde(default)]
    presses: Option<u32>,
}

#[async_trait]
impl McpClient for PressKeyTool {
    fn name(&self) -> &str {
        "press_key"
    }

    fn description(&self) -> &str {
        "Press a key or key combination, e.g. \"enter\", \"esc\", \"ctrl+a\", \"ctrl+shift+t\", \
         \"win+d\", \"alt+f4\", \"tab\", \"f5\". Format: modifiers (ctrl/shift/alt/win) joined \
         with '+', main key last. Named keys: enter, esc, tab, space, backspace, delete, \
         insert, up, down, left, right, home, end, pageup, pagedown, printscreen, capslock, \
         numlock, plus, minus, f1-f12, letters a-z, digits 0-9. Acts on the focused window \
         on the user's real machine. The result includes a post-action screenshot — read it \
         to see what the shortcut did, no re-capture needed."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["combo"],
            "properties": {
                "combo": { "type": "string", "description": "Key combination, e.g. \"ctrl+s\"." },
                "presses": { "type": "integer", "description": "Times to press (default 1, max 10), e.g. 3 for triple-tab." }
            }
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::Confirm
    }

    fn auth_reason(&self) -> Option<String> {
        Some("将模拟按键并回传操作后的屏幕截图给当前模型服务商（快捷键会真实作用于你机器上聚焦的应用）".into())
    }

    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(
            "press_key 必须通过 execute_with_output 调用（需要 conv_id 记录输入日志 + 回传附图）".into(),
        ))
    }

    async fn execute_with_output(&self, args: &str, ctx: &ToolContext) -> AppResult<ToolOutput> {
        let p: PressKeyArgs =
            serde_json::from_str(args)
                .map_err(|e| AppError::Validation(format!("press_key 参数解析失败: {e}")))?;
        let presses = p.presses.unwrap_or(1).clamp(1, 10);
        let (mods, main) = parse_combo(&p.combo)?;

        for i in 0..presses {
            if i > 0 {
                tokio::time::sleep(Duration::from_millis(PRESS_GAP_MS)).await;
            }
            for vk in &mods {
                self.backend.key_vk(*vk, true)?;
            }
            if !mods.is_empty() {
                tokio::time::sleep(Duration::from_millis(KEY_GAP_MS)).await;
            }
            self.backend.key_vk(main, true)?;
            self.backend.key_vk(main, false)?;
            if !mods.is_empty() {
                tokio::time::sleep(Duration::from_millis(KEY_GAP_MS)).await;
                for vk in mods.iter().rev() {
                    self.backend.key_vk(*vk, false)?;
                }
            }
        }

        tracing::info!(
            target: "ice_paw.screen",
            conv = %ctx.conv_id, combo = %p.combo, presses,
            "press_key 成功"
        );
        let mut echo = serde_json::Map::new();
        echo.insert("action".into(), "press_key".into());
        echo.insert("combo".into(), serde_json::json!(p.combo));
        echo.insert("presses".into(), serde_json::json!(presses));
        let shot = super::action_shot(&self.backend, &self.state, ctx).await;
        Ok(super::finish_action_output(echo, shot))
    }
}

// =========================================================================
// wait（节奏件——Always 级：无外部作用，只是暂停）
// =========================================================================

#[derive(Deserialize)]
struct WaitArgs {
    ms: u64,
}

#[derive(Default)]
pub struct WaitTool;

impl WaitTool {
    pub fn builtin() -> Self {
        Self
    }
}

#[async_trait]
impl McpClient for WaitTool {
    fn name(&self) -> &str {
        "wait"
    }

    fn description(&self) -> &str {
        "Pause for a moment (50-10000 ms) before the next action — useful after clicking \
         something that takes time to load (menus, dialogs, navigation) so the next \
         capture_screen shows the settled state. Interrupted immediately if the user stops \
         the turn. No effect on the user's machine beyond time passing."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["ms"],
            "properties": {
                "ms": { "type": "integer", "description": "Milliseconds to wait (50-10000; clamped to range)." }
            }
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::Always
    }

    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(
            "wait 必须通过 execute_with_context 调用（需要取消令牌实现立即中断）".into(),
        ))
    }

    async fn execute_with_context(&self, args: &str, ctx: &ToolContext) -> AppResult<String> {
        let p: WaitArgs =
            serde_json::from_str(args)
                .map_err(|e| AppError::Validation(format!("wait 参数解析失败: {e}")))?;
        let requested = p.ms;
        let actual = clamp_wait(requested);
        let slept = tokio::time::sleep(Duration::from_millis(actual));

        // 用户点「停止生成」→ 立即返回（对齐 proposal_tool 的取消语义：Ok + 状态位）。
        if let Some(cancel) = ctx.cancel.as_ref() {
            tokio::select! {
                biased;
                _ = crate::harness::tool_executor::wait_for_cancel(cancel) => {
                    tracing::info!(
                        target: "ice_paw.screen",
                        conv = %ctx.conv_id, actual,
                        "wait 因对话取消而中断"
                    );
                    return Ok(serde_json::json!({
                        "action": "wait",
                        "status": "cancelled",
                        "requested_ms": requested,
                        "note": "Wait interrupted: the turn was cancelled."
                    })
                    .to_string());
                }
                _ = slept => {}
            }
        } else {
            slept.await;
        }

        Ok(serde_json::json!({
            "action": "wait",
            "status": "done",
            "requested_ms": requested,
            "waited_ms": actual,
            "note": if actual != requested { "Clamped to the 50-10000 ms range." } else { "Waited." }
        })
        .to_string())
    }
}

// =========================================================================
// 单测（Fake 键盘后端——记录 VK/Unicode 事件序列）
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::mcp::screen::coords::{PhysRect, VirtualScreenLayout};
    use crate::harness::mcp::screen::{MouseButton, RgbaFrame, WindowInfo};
    use std::sync::Mutex;

    struct FakeKeyboardBackend {
        vks: Mutex<Vec<(u16, bool)>>,
        units: Mutex<Vec<(u16, bool)>>,
    }

    impl FakeKeyboardBackend {
        fn new() -> Self {
            Self {
                vks: Mutex::new(Vec::new()),
                units: Mutex::new(Vec::new()),
            }
        }
    }

    impl ScreenBackend for FakeKeyboardBackend {
        fn name(&self) -> &'static str {
            "fake-keyboard"
        }
        fn virtual_screen(&self) -> AppResult<VirtualScreenLayout> {
            Ok(VirtualScreenLayout {
                origin_x: 0,
                origin_y: 0,
                width: 1920,
                height: 1080,
            })
        }
        fn monitors(&self) -> AppResult<Vec<PhysRect>> {
            Ok(vec![PhysRect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            }])
        }
        fn capture(&self, region: PhysRect) -> AppResult<RgbaFrame> {
            Ok(RgbaFrame {
                width: region.width,
                height: region.height,
                rgba: vec![0; region.width as usize * region.height as usize * 4],
            })
        }
        fn windows(&self) -> AppResult<Vec<WindowInfo>> {
            Ok(vec![])
        }
        fn capture_window(&self, _hwnd: i64) -> AppResult<(RgbaFrame, PhysRect)> {
            Err(AppError::Internal("fake 不支持窗口捕获".into()))
        }
        fn foreground_window(&self) -> Option<i64> {
            None
        }
        fn mouse_move_abs(&self, _abs_x: i32, _abs_y: i32) -> AppResult<()> {
            Ok(())
        }
        fn mouse_button(&self, _button: MouseButton, _down: bool) -> AppResult<()> {
            Ok(())
        }
        fn mouse_scroll(&self, _dx_notches: i32, _dy_notches: i32) -> AppResult<()> {
            Ok(())
        }
        fn key_vk(&self, vk: u16, down: bool) -> AppResult<()> {
            self.vks.lock().unwrap().push((vk, down));
            Ok(())
        }
        fn key_unicode(&self, unit: u16, down: bool) -> AppResult<()> {
            self.units.lock().unwrap().push((unit, down));
            Ok(())
        }
    }

    async fn make_ctx(conv: &str) -> ToolContext {
        ToolContext {
            conv_id: conv.into(),
            agent_id: "a1".into(),
            project_id: None,
            workspace: None,
            pool: sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap(),
            api_key: None,
            app_handle: None,
            proposal_registry: None,
            turn_id: None,
            cancel: None,
        }
    }

    #[tokio::test]
    async fn type_text_sends_utf16_units_down_up() {
        let backend = Arc::new(FakeKeyboardBackend::new());
        let tool = TypeTextTool::new(backend.clone(), Arc::new(ScreenState::new()));
        let ctx = make_ctx("t1").await;

        // "hi你😀"：h(0x68) i(0x69) 你(0x4F60) 😀=代理对(0xD83D,0xDE00)
        let out = tool
            .execute_with_output(r#"{"text":"hi你😀"}"#, &ctx)
            .await
            .unwrap();
        let units = backend.units.lock().unwrap().clone();
        let expected: Vec<(u16, bool)> = [0x68u16, 0x69, 0x4F60, 0xD83D, 0xDE00]
            .iter()
            .flat_map(|u| [(*u, true), (*u, false)])
            .collect();
        assert_eq!(units, expected);
        assert!(backend.vks.lock().unwrap().is_empty());
        // act-and-look：输入后附操作效果图（无基准 → 整桌面回落）
        assert!(out.image_png.is_some(), "type 后应附操作效果图");
        let v: serde_json::Value = serde_json::from_str(&out.text).unwrap();
        assert_eq!(v["utf16_units"].as_u64().unwrap(), 5);
        assert!(v["note"].as_str().unwrap().contains("most recent image"));

        // 空文本 → 家族错误（指路 ctrl+a+delete 而非傻输入）
        let err = tool.execute_with_output(r#"{"text":""}"#, &ctx).await.unwrap_err();
        assert!(err.to_string().contains("screen 输入参数无效"), "实际: {err}");
    }

    #[tokio::test]
    async fn press_key_combo_orders_modifiers_main_reverse_release() {
        let backend = Arc::new(FakeKeyboardBackend::new());
        let tool = PressKeyTool::new(backend.clone(), Arc::new(ScreenState::new()));
        let ctx = make_ctx("k1").await;

        // ctrl(0x11)+shift(0x10)+t(0x54)
        tool.execute_with_output(r#"{"combo":"ctrl+shift+t"}"#, &ctx)
            .await
            .unwrap();
        assert_eq!(
            backend.vks.lock().unwrap().clone(),
            vec![
                (0x11, true),
                (0x10, true),
                (0x54, true),
                (0x54, false),
                (0x10, false),
                (0x11, false),
            ]
        );

        // 单键 + 别名 + 大小写不敏感
        backend.vks.lock().unwrap().clear();
        tool.execute_with_output(r#"{"combo":"ENTER"}"#, &ctx).await.unwrap();
        assert_eq!(
            backend.vks.lock().unwrap().clone(),
            vec![(0x0D, true), (0x0D, false)]
        );

        backend.vks.lock().unwrap().clear();
        tool.execute_with_output(r#"{"combo":"F5"}"#, &ctx).await.unwrap();
        assert_eq!(
            backend.vks.lock().unwrap().clone(),
            vec![(0x74, true), (0x74, false)]
        );

        // presses=2：两轮完整序列
        backend.vks.lock().unwrap().clear();
        tool.execute_with_output(r#"{"combo":"tab","presses":2}"#, &ctx)
            .await
            .unwrap();
        assert_eq!(
            backend.vks.lock().unwrap().clone(),
            vec![(0x09, true), (0x09, false), (0x09, true), (0x09, false)]
        );
    }

    #[tokio::test]
    async fn press_key_rejects_unknown_and_empty_segments() {
        let tool = PressKeyTool::new(
            Arc::new(FakeKeyboardBackend::new()),
            Arc::new(ScreenState::new()),
        );
        let ctx = make_ctx("k2").await;

        let err = tool
            .execute_with_output(r#"{"combo":"ctrl+nonsense"}"#, &ctx)
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("screen 按键无效"), "实际: {msg}");
        assert!(msg.contains("nonsense"), "应点名坏键名: {msg}");

        // 空段（"ctrl+"）
        let err = tool.execute_with_output(r#"{"combo":"ctrl+"}"#, &ctx).await.unwrap_err();
        assert!(err.to_string().contains("空段"), "实际: {err}");

        // f13 越界不认
        let err = tool.execute_with_output(r#"{"combo":"f13"}"#, &ctx).await.unwrap_err();
        assert!(err.to_string().contains("screen 按键无效"), "实际: {err}");
    }

    #[tokio::test]
    async fn wait_completes_and_reports_clamp() {
        let ctx = make_ctx("w1").await;
        let start = std::time::Instant::now();
        let out = WaitTool
            .execute_with_context(r#"{"ms":120}"#, &ctx)
            .await
            .unwrap();
        assert!(start.elapsed() >= Duration::from_millis(120));
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["status"], "done");
        assert_eq!(v["waited_ms"].as_u64().unwrap(), 120);

        // 钳制纯函数
        assert_eq!(clamp_wait(1), 50);
        assert_eq!(clamp_wait(5_000_000), 10_000);
        assert_eq!(clamp_wait(500), 500);
    }

    #[tokio::test]
    async fn wait_interrupts_on_cancel() {
        let cancel = crate::infra::cancel::CancellationToken::new();
        let ctx = ToolContext {
            cancel: Some(cancel.clone()),
            ..make_ctx("w2").await
        };
        // 50ms 后触发取消——wait 要求 10s，被取消应远早于 1s 返回
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            cancel.cancel();
        });
        let start = std::time::Instant::now();
        let out = WaitTool
            .execute_with_context(r#"{"ms":10000}"#, &ctx)
            .await
            .unwrap();
        assert!(start.elapsed() < Duration::from_millis(1000), "取消应立即中断");
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["status"], "cancelled");
    }

    /// VK 表钉板（防手滑改错码位）。
    #[test]
    fn vk_table_pins() {
        assert_eq!(vk_for("a"), Some(0x41));
        assert_eq!(vk_for("Z"), Some(0x5A));
        assert_eq!(vk_for("0"), Some(0x30));
        assert_eq!(vk_for("9"), Some(0x39));
        assert_eq!(vk_for("ctrl"), Some(0x11));
        assert_eq!(vk_for("shift"), Some(0x10));
        assert_eq!(vk_for("alt"), Some(0x12));
        assert_eq!(vk_for("win"), Some(0x5B));
        assert_eq!(vk_for("enter"), Some(0x0D));
        assert_eq!(vk_for("esc"), Some(0x1B));
        assert_eq!(vk_for("space"), Some(0x20));
        assert_eq!(vk_for("backspace"), Some(0x08));
        assert_eq!(vk_for("delete"), Some(0x2E));
        assert_eq!(vk_for("left"), Some(0x25));
        assert_eq!(vk_for("down"), Some(0x28));
        assert_eq!(vk_for("home"), Some(0x24));
        assert_eq!(vk_for("end"), Some(0x23));
        assert_eq!(vk_for("pageup"), Some(0x21));
        assert_eq!(vk_for("pagedown"), Some(0x22));
        assert_eq!(vk_for("f1"), Some(0x70));
        assert_eq!(vk_for("f12"), Some(0x7B));
        assert_eq!(vk_for("plus"), Some(0xBB));
        assert_eq!(vk_for("printscreen"), Some(0x2C));
        assert_eq!(vk_for("f13"), None);
        assert_eq!(vk_for("++"), None);
        assert_eq!(vk_for("é"), None);
    }
}
