//! computer use 操作工具 —— 鼠标四件（阶段二第一刀）。
//!
//! 坐标契约的输入侧落地：模型传的 x/y = 本会话**最近一次截图的图片像素空间**，
//! 工具层三步换算（图片 → 物理 → SendInput 绝对），后端零数学：
//!
//! ```text
//! resolve_point: state.get(conv) → 布局 revalidate → img_to_phys → phys_to_absolute
//! ```
//!
//! **revalidate（防过期坐标伤人）**：截图后显示器布局变了（插拔/改分辨率），
//! 旧坐标映射到的物理位置可能完全错位——对比 `virtual_screen()` 与 meta 里的
//! 布局快照，不一致拒绝执行，报单一家族前缀 `screen 坐标过期`（doom_detect
//! 冒号切分依赖首段稳定）。
//!
//! **授权**：全部 `Confirm` 级——输入模拟真实作用于用户机器上的应用。
//!
//! SendInput 是微秒级 syscall，不走 `spawn_blocking`（与几十 ms 的 BitBlt 不同）；
//! 拖拽的步进间隔用 tokio sleep，天然可被取消令牌打断（工具执行层的超时罩着）。

use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;

use crate::error::{AppError, AppResult};

use super::backend::{MouseButton, ScreenBackend};
use super::coords::{self, CaptureMeta};
use super::state::ScreenState;
use crate::harness::mcp::client::{McpClient, ToolContext};
use crate::harness::mcp::types::AuthorizationLevel;

#[cfg(windows)]
use super::backend::GdiBackend;
#[cfg(not(windows))]
use super::backend::UnsupportedBackend;

/// 移动后的落点稳定等待：真实点击需要目标窗口响应 hover/重绘后再落键。
const SETTLE_MS: u64 = 40;
/// 拖拽步进数与步进间隔：10 步 × 12ms ≈ 120ms 拖完——足够目标应用跟进
/// 拖拽悬停（drag-over 高亮），又不至于慢到影响回合节奏。
const DRAG_STEPS: u32 = 10;
const DRAG_STEP_MS: u64 = 12;
/// 双击两段按压的间隔（略短于系统双击时阈值，稳定被识别为 double）。
const DOUBLE_GAP_MS: u64 = 60;

/// 生产后端（4 件工具共用；测试用注入构造）。
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

// =========================================================================
// 共享：坐标解析 + revalidate
// =========================================================================

/// 图片像素 → (物理像素, SendInput 绝对坐标)，附 meta 供摘要回显。
///
/// 家族错误两条：`screen 坐标基准缺失`（本会话没截过图）、
/// `screen 坐标过期`（截图后布局变了）——都指路先重新截图。
fn resolve_point(
    backend: &dyn ScreenBackend,
    state: &ScreenState,
    ctx: &ToolContext,
    x: i64,
    y: i64,
) -> AppResult<(CaptureMeta, i32, i32, i32, i32)> {
    let meta = state.get(&ctx.conv_id).ok_or_else(|| {
        AppError::Validation(
            "screen 坐标基准缺失: 本会话还没有截图，坐标无从换算——\
             先调用 capture_screen（或 capture_window）建立坐标基准，\
             再对图中的位置操作".into(),
        )
    })?;
    let cur = backend.virtual_screen()?;
    if cur != meta.layout {
        return Err(AppError::Validation(format!(
            "screen 坐标过期: 截图后显示器布局已变化（{:?} → {:?}），\
             旧截图上的坐标可能已指向错误位置——请重新截图后再操作",
            meta.layout, cur
        )));
    }
    let (px, py) = meta.img_to_phys(x, y);
    let (ax, ay) = coords::phys_to_absolute(&meta.layout, px, py);
    Ok((meta, px, py, ax, ay))
}

/// 按钮参数（serde 面）→ 后端枚举。
#[derive(Debug, Deserialize, Default, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum ButtonArg {
    #[default]
    Left,
    Right,
    Middle,
}

impl From<ButtonArg> for MouseButton {
    fn from(b: ButtonArg) -> Self {
        match b {
            ButtonArg::Left => MouseButton::Left,
            ButtonArg::Right => MouseButton::Right,
            ButtonArg::Middle => MouseButton::Middle,
        }
    }
}

impl ButtonArg {
    fn as_str(self) -> &'static str {
        match self {
            ButtonArg::Left => "left",
            ButtonArg::Right => "right",
            ButtonArg::Middle => "middle",
        }
    }
}

/// 工具共用的持有面（backend + 坐标基准状态）。
macro_rules! screen_input_tool {
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

            /// 生产构造（与看屏工具共用进程级 ScreenState——同一坐标基准）。
            pub fn builtin() -> Self {
                Self {
                    backend: builtin_backend(),
                    state: super::state::global(),
                }
            }
        }
    };
}

screen_input_tool!(MouseMoveTool);
screen_input_tool!(MouseClickTool);
screen_input_tool!(MouseDragTool);
screen_input_tool!(MouseScrollTool);

/// 结果摘要的公共回显段（坐标契约提醒模型下一次仍用图片像素）。
/// 返回 Map 供调用方 extend 组装（json! 宏不支持展开语法）。
fn echo_point(
    meta: &CaptureMeta,
    img_x: i64,
    img_y: i64,
    px: i32,
    py: i32,
) -> serde_json::Map<String, serde_json::Value> {
    let mut m = serde_json::Map::new();
    m.insert("image_xy".into(), serde_json::json!([img_x, img_y]));
    m.insert("physical_xy".into(), serde_json::json!([px, py]));
    m.insert(
        "image_size".into(),
        serde_json::json!({ "width": meta.sent_width, "height": meta.sent_height }),
    );
    m
}

// =========================================================================
// mouse_move
// =========================================================================

#[derive(Deserialize)]
struct MouseMoveArgs {
    x: i64,
    y: i64,
}

#[async_trait]
impl McpClient for MouseMoveTool {
    fn name(&self) -> &str {
        "mouse_move"
    }

    fn description(&self) -> &str {
        "Move the mouse cursor to a point on the screen. Coordinates (x, y) are in the pixel \
         space of the MOST RECENT captured image (the coordinate contract declared by \
         capture_screen/capture_window). Fails with 'coordinate base missing' if no screenshot \
         was taken yet in this conversation, and 'coordinates stale' if the display layout \
         changed since — re-capture and retry. Moving alone clicks nothing."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["x", "y"],
            "properties": {
                "x": { "type": "integer", "description": "X in the most recent image's pixels." },
                "y": { "type": "integer", "description": "Y in the most recent image's pixels." }
            }
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::Confirm
    }

    fn auth_reason(&self) -> Option<String> {
        Some("将移动鼠标指针（会真实作用于你机器上的当前桌面）".into())
    }

    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(
            "mouse_move 必须通过 execute_with_context 调用（需要 conv_id 定位坐标基准）".into(),
        ))
    }

    async fn execute_with_context(&self, args: &str, ctx: &ToolContext) -> AppResult<String> {
        let p: MouseMoveArgs = serde_json::from_str(args).map_err(|e| {
            AppError::Validation(format!("mouse_move 参数解析失败: {e}"))
        })?;
        let (meta, px, py, ax, ay) =
            resolve_point(self.backend.as_ref(), &self.state, ctx, p.x, p.y)?;
        self.backend.mouse_move_abs(ax, ay)?;
        tracing::info!(
            target: "ice_paw.screen",
            conv = %ctx.conv_id, image = ?(p.x, p.y), phys = ?(px, py),
            "mouse_move 成功"
        );
        let mut out = echo_point(&meta, p.x, p.y, px, py);
        out.insert("action".into(), "mouse_move".into());
        out.insert(
            "note".into(),
            "Cursor moved (nothing clicked). Coordinates for all screen tools stay in the most \
             recent image's pixel space."
                .into(),
        );
        Ok(serde_json::Value::Object(out).to_string())
    }
}

// =========================================================================
// mouse_click
// =========================================================================

#[derive(Deserialize)]
struct MouseClickArgs {
    /// 点击位置（图片像素空间）；省略 = 在当前指针位置点击（scroll 后连点等场景）。
    #[serde(default)]
    x: Option<i64>,
    #[serde(default)]
    y: Option<i64>,
    #[serde(default)]
    button: ButtonArg,
    #[serde(default)]
    double: bool,
}

#[async_trait]
impl McpClient for MouseClickTool {
    fn name(&self) -> &str {
        "mouse_click"
    }

    fn description(&self) -> &str {
        "Click the mouse. Give (x, y) in the most recent image's pixel space to move there \
         first, or omit both to click at the current cursor position. button: left (default) / \
         right (context menu) / middle. double: true for a double-click. The click acts on \
         whatever is under the point on the user's real screen — prefer precise coordinates \
         from a fresh capture."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "x": { "type": "integer", "description": "X in the most recent image's pixels. Omit to click at the current position." },
                "y": { "type": "integer", "description": "Y in the most recent image's pixels. Omit to click at the current position." },
                "button": { "type": "string", "enum": ["left", "right", "middle"], "description": "Mouse button (default left)." },
                "double": { "type": "boolean", "description": "True = double-click (default false)." }
            }
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::Confirm
    }

    fn auth_reason(&self) -> Option<String> {
        Some("将模拟鼠标点击——会真实作用于屏幕上当前打开的应用".into())
    }

    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(
            "mouse_click 必须通过 execute_with_context 调用（需要 conv_id 定位坐标基准）".into(),
        ))
    }

    async fn execute_with_context(&self, args: &str, ctx: &ToolContext) -> AppResult<String> {
        let p: MouseClickArgs = serde_json::from_str(args).map_err(|e| {
            AppError::Validation(format!("mouse_click 参数解析失败: {e}"))
        })?;

        let button: MouseButton = p.button.into();
        let mut echo = serde_json::Map::new();
        echo.insert("action".into(), "mouse_click".into());

        // 有点 → 解析换算 + 先移动；无点 → 原位点击（无需坐标基准）。
        match (p.x, p.y) {
            (Some(x), Some(y)) => {
                let (meta, px, py, ax, ay) =
                    resolve_point(self.backend.as_ref(), &self.state, ctx, x, y)?;
                self.backend.mouse_move_abs(ax, ay)?;
                tokio::time::sleep(Duration::from_millis(SETTLE_MS)).await;
                echo.extend(echo_point(&meta, x, y, px, py));
            }
            (None, None) => {
                echo.insert(
                    "position".into(),
                    "current_cursor".into(),
                );
            }
            // 半给坐标是最常见的模型笔误——直接拦下而不是悄悄用半个。
            _ => {
                return Err(AppError::Validation(
                    "mouse_click 参数不完整: x 与 y 必须同时给出（或同时省略表示原位点击）".into(),
                ));
            }
        }

        let rounds = if p.double { 2 } else { 1 };
        for r in 0..rounds {
            if r > 0 {
                tokio::time::sleep(Duration::from_millis(DOUBLE_GAP_MS)).await;
            }
            self.backend.mouse_button(button, true)?;
            self.backend.mouse_button(button, false)?;
        }

        tracing::info!(
            target: "ice_paw.screen",
            conv = %ctx.conv_id, button = ?p.button, double = p.double,
            image = ?(p.x, p.y),
            "mouse_click 成功"
        );
        echo.insert("button".into(), p.button.as_str().into());
        if p.double {
            echo.insert("double".into(), serde_json::json!(true));
        }
        echo.insert(
            "note".into(),
            serde_json::json!(format!(
                "Clicked. The effect depends on what was under the point — capture again to see \
                 the result.{}",
                if p.double { " (double-click)" } else { "" }
            )),
        );
        Ok(serde_json::Value::Object(echo).to_string())
    }
}

// =========================================================================
// mouse_drag
// =========================================================================

#[derive(Deserialize)]
struct MouseDragArgs {
    from_x: i64,
    from_y: i64,
    to_x: i64,
    to_y: i64,
    #[serde(default)]
    button: ButtonArg,
}

#[async_trait]
impl McpClient for MouseDragTool {
    fn name(&self) -> &str {
        "mouse_drag"
    }

    fn description(&self) -> &str {
        "Drag with the mouse: press at (from_x, from_y), move to (to_x, to_y) in smooth steps, \
         release. All four coordinates are in the most recent image's pixel space. Used for \
         moving sliders, repositioning windows, selecting text ranges. The path is \
         interpolated (~10 steps) so drag-over targets can react."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["from_x", "from_y", "to_x", "to_y"],
            "properties": {
                "from_x": { "type": "integer", "description": "Start X in the most recent image's pixels." },
                "from_y": { "type": "integer", "description": "Start Y in the most recent image's pixels." },
                "to_x": { "type": "integer", "description": "End X in the most recent image's pixels." },
                "to_y": { "type": "integer", "description": "End Y in the most recent image's pixels." },
                "button": { "type": "string", "enum": ["left", "right", "middle"], "description": "Button held during the drag (default left)." }
            }
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::Confirm
    }

    fn auth_reason(&self) -> Option<String> {
        Some("将模拟鼠标拖拽——按下、移动、释放会真实作用于屏幕上的应用".into())
    }

    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(
            "mouse_drag 必须通过 execute_with_context 调用（需要 conv_id 定位坐标基准）".into(),
        ))
    }

    async fn execute_with_context(&self, args: &str, ctx: &ToolContext) -> AppResult<String> {
        let p: MouseDragArgs = serde_json::from_str(args).map_err(|e| {
            AppError::Validation(format!("mouse_drag 参数解析失败: {e}"))
        })?;

        // 两端各自换算（同一次 resolve 共享同一 meta + 同一布局校验）。
        let (meta, fx, fy, fax, fay) =
            resolve_point(self.backend.as_ref(), &self.state, ctx, p.from_x, p.from_y)?;
        let (_, tx, ty, tax, tay) =
            resolve_point(self.backend.as_ref(), &self.state, ctx, p.to_x, p.to_y)?;

        let button: MouseButton = p.button.into();
        self.backend.mouse_move_abs(fax, fay)?;
        tokio::time::sleep(Duration::from_millis(SETTLE_MS)).await;
        self.backend.mouse_button(button, true)?;
        // 线性插值步进（绝对坐标空间线性 = 物理像素空间线性，仿射等价）。
        for i in 1..=DRAG_STEPS {
            let t = i as f64 / DRAG_STEPS as f64;
            let ax = (fax as f64 + (tax - fax) as f64 * t).round() as i32;
            let ay = (fay as f64 + (tay - fay) as f64 * t).round() as i32;
            self.backend.mouse_move_abs(ax, ay)?;
            tokio::time::sleep(Duration::from_millis(DRAG_STEP_MS)).await;
        }
        tokio::time::sleep(Duration::from_millis(SETTLE_MS)).await;
        self.backend.mouse_button(button, false)?;

        tracing::info!(
            target: "ice_paw.screen",
            conv = %ctx.conv_id, button = ?p.button,
            from_image = ?(p.from_x, p.from_y), to_image = ?(p.to_x, p.to_y),
            from_phys = ?(fx, fy), to_phys = ?(tx, ty),
            "mouse_drag 成功"
        );
        let mut out = serde_json::json!({
            "action": "mouse_drag",
            "button": p.button.as_str(),
            "from": { "image_xy": [p.from_x, p.from_y], "physical_xy": [fx, fy] },
            "to": { "image_xy": [p.to_x, p.to_y], "physical_xy": [tx, ty] },
            "note": "Dragged. Capture again to see the result."
        });
        out.as_object_mut()
            .unwrap()
            .extend(echo_point(&meta, p.from_x, p.from_y, fx, fy));
        Ok(out.to_string())
    }
}

// =========================================================================
// mouse_scroll
// =========================================================================

#[derive(Deserialize)]
struct MouseScrollArgs {
    /// 垂直刻数（正=向上，负=向下；1 刻 ≈ 3 行）。
    #[serde(default)]
    dy: i32,
    /// 水平刻数（正=向右，负=向左）。
    #[serde(default)]
    dx: i32,
    /// 可选滚动位置（图片像素空间）；省略 = 在当前指针位置滚。
    #[serde(default)]
    x: Option<i64>,
    #[serde(default)]
    y: Option<i64>,
}

#[async_trait]
impl McpClient for MouseScrollTool {
    fn name(&self) -> &str {
        "mouse_scroll"
    }

    fn description(&self) -> &str {
        "Scroll the mouse wheel at a point (x, y in the most recent image's pixels) or at the \
         current cursor position (omit x/y). dy = vertical notches (positive up, negative \
         down; 1 notch ≈ 3 lines), dx = horizontal notches (positive right). Scrolling acts \
         on whatever is under the point — hover state matters, prefer explicit x/y from a \
         fresh capture."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "dy": { "type": "integer", "description": "Vertical notches: positive scrolls up, negative down. Omit for none." },
                "dx": { "type": "integer", "description": "Horizontal notches: positive scrolls right, negative left. Omit for none." },
                "x": { "type": "integer", "description": "X in the most recent image's pixels. Omit to scroll at the current position." },
                "y": { "type": "integer", "description": "Y in the most recent image's pixels. Omit to scroll at the current position." }
            }
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::Confirm
    }

    fn auth_reason(&self) -> Option<String> {
        Some("将模拟鼠标滚轮——会真实作用于指针位置下的应用".into())
    }

    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(
            "mouse_scroll 必须通过 execute_with_context 调用（需要 conv_id 定位坐标基准）".into(),
        ))
    }

    async fn execute_with_context(&self, args: &str, ctx: &ToolContext) -> AppResult<String> {
        let p: MouseScrollArgs = serde_json::from_str(args).map_err(|e| {
            AppError::Validation(format!("mouse_scroll 参数解析失败: {e}"))
        })?;
        if p.dx == 0 && p.dy == 0 {
            return Err(AppError::Validation(
                "mouse_scroll 参数无效: dx 与 dy 至少一个非零（收到 0, 0）".into(),
            ));
        }
        match (p.x, p.y) {
            (Some(x), Some(y)) => {
                let (meta, px, py, ax, ay) =
                    resolve_point(self.backend.as_ref(), &self.state, ctx, x, y)?;
                self.backend.mouse_move_abs(ax, ay)?;
                tokio::time::sleep(Duration::from_millis(SETTLE_MS)).await;
                self.backend.mouse_scroll(p.dx, p.dy)?;
                tracing::info!(
                    target: "ice_paw.screen",
                    conv = %ctx.conv_id, dx = p.dx, dy = p.dy,
                    image = ?(x, y), phys = ?(px, py),
                    "mouse_scroll 成功"
                );
                let mut out = serde_json::json!({
                    "action": "mouse_scroll",
                    "dx": p.dx, "dy": p.dy,
                    "note": "Scrolled. Capture again to see the result."
                });
                out.as_object_mut()
                    .unwrap()
                    .extend(echo_point(&meta, x, y, px, py));
                Ok(out.to_string())
            }
            (None, None) => {
                self.backend.mouse_scroll(p.dx, p.dy)?;
                tracing::info!(
                    target: "ice_paw.screen",
                    conv = %ctx.conv_id, dx = p.dx, dy = p.dy,
                    "mouse_scroll 成功（原位）"
                );
                Ok(serde_json::json!({
                    "action": "mouse_scroll",
                    "dx": p.dx, "dy": p.dy,
                    "position": "current_cursor",
                    "note": "Scrolled at the current cursor position. Capture again to see the result."
                })
                .to_string())
            }
            _ => Err(AppError::Validation(
                "mouse_scroll 参数不完整: x 与 y 必须同时给出（或同时省略表示原位滚动）".into(),
            )),
        }
    }
}

// =========================================================================
// 单测（Fake 输入后端 —— 记录绝对坐标序列，与 GDI 同管道）
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::mcp::screen::coords::{PhysRect, VirtualScreenLayout};
    use crate::harness::mcp::screen::{RgbaFrame, WindowInfo};
    use std::sync::Mutex;

    /// 可变布局 + 输入记录的假后端（捕获方法最小实现——输入测试不经过它们）。
    struct FakeInputBackend {
        layout: Mutex<VirtualScreenLayout>,
        moves: Mutex<Vec<(i32, i32)>>,
        buttons: Mutex<Vec<(MouseButton, bool)>>,
        scrolls: Mutex<Vec<(i32, i32)>>,
    }

    impl FakeInputBackend {
        fn new() -> Self {
            Self {
                layout: Mutex::new(VirtualScreenLayout {
                    origin_x: 0,
                    origin_y: 0,
                    width: 1920,
                    height: 1080,
                }),
                moves: Mutex::new(Vec::new()),
                buttons: Mutex::new(Vec::new()),
                scrolls: Mutex::new(Vec::new()),
            }
        }

        /// 模拟插拔显示器（布局变化 → 坐标过期）。
        fn change_layout(&self) {
            let mut g = self.layout.lock().unwrap();
            g.width += 100;
        }
    }

    impl ScreenBackend for FakeInputBackend {
        fn name(&self) -> &'static str {
            "fake-input"
        }
        fn virtual_screen(&self) -> AppResult<VirtualScreenLayout> {
            Ok(*self.layout.lock().unwrap())
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
        fn mouse_move_abs(&self, abs_x: i32, abs_y: i32) -> AppResult<()> {
            self.moves.lock().unwrap().push((abs_x, abs_y));
            Ok(())
        }
        fn mouse_button(&self, button: MouseButton, down: bool) -> AppResult<()> {
            self.buttons.lock().unwrap().push((button, down));
            Ok(())
        }
        fn mouse_scroll(&self, dx_notches: i32, dy_notches: i32) -> AppResult<()> {
            self.scrolls.lock().unwrap().push((dx_notches, dy_notches));
            Ok(())
        }
    }

    /// 全屏 1920×1080 → 发送 1600×900 的坐标基准（scale 1.2）。
    fn full_screen_meta() -> CaptureMeta {
        CaptureMeta {
            layout: VirtualScreenLayout {
                origin_x: 0,
                origin_y: 0,
                width: 1920,
                height: 1080,
            },
            phys_region: PhysRect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            },
            sent_width: 1600,
            sent_height: 900,
            monitor: None,
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
    async fn move_requires_prior_capture() {
        let tool = MouseMoveTool::new(Arc::new(FakeInputBackend::new()), Arc::new(ScreenState::new()));
        let ctx = make_ctx("m0").await;
        let err = tool.execute_with_context(r#"{"x":10,"y":10}"#, &ctx).await.unwrap_err();
        assert!(
            err.to_string().contains("screen 坐标基准缺失"),
            "家族前缀应为坐标基准缺失，实际: {err}"
        );
    }

    #[tokio::test]
    async fn move_maps_image_px_through_phys_to_absolute() {
        let backend = Arc::new(FakeInputBackend::new());
        let state = Arc::new(ScreenState::new());
        state.update("m1", full_screen_meta());
        let tool = MouseMoveTool::new(backend.clone(), state);
        let ctx = make_ctx("m1").await;

        // 图中 (800, 450) → 物理 (960, 540)（1.2×）→ 绝对（端点精确映射 65535/span-1）
        tool.execute_with_context(r#"{"x":800,"y":450}"#, &ctx)
            .await
            .unwrap();
        let moves = backend.moves.lock().unwrap().clone();
        assert_eq!(moves.len(), 1);
        assert_eq!(moves[0], (960 * 65535 / 1919, 540 * 65535 / 1079));

        // 摘要回显物理坐标 + 图片尺寸（坐标契约自文档）
        let out = tool
            .execute_with_context(r#"{"x":0,"y":0}"#, &ctx)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["physical_xy"][0].as_i64().unwrap(), 0);
        assert_eq!(v["image_size"]["width"].as_u64().unwrap(), 1600);
    }

    #[tokio::test]
    async fn stale_layout_is_rejected_with_family_error() {
        let backend = Arc::new(FakeInputBackend::new());
        let state = Arc::new(ScreenState::new());
        state.update("m2", full_screen_meta());
        backend.change_layout(); // 截图后插了显示器
        let tool = MouseClickTool::new(backend.clone(), state);
        let ctx = make_ctx("m2").await;
        let err = tool
            .execute_with_context(r#"{"x":100,"y":100}"#, &ctx)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("screen 坐标过期"),
            "家族前缀应为坐标过期，实际: {err}"
        );
        // 未落任何输入
        assert!(backend.moves.lock().unwrap().is_empty());
        assert!(backend.buttons.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn click_moves_then_presses_release_once() {
        let backend = Arc::new(FakeInputBackend::new());
        let state = Arc::new(ScreenState::new());
        state.update("c1", full_screen_meta());
        let tool = MouseClickTool::new(backend.clone(), state);
        let ctx = make_ctx("c1").await;

        tool.execute_with_context(r#"{"x":400,"y":300}"#, &ctx)
            .await
            .unwrap();
        assert_eq!(backend.moves.lock().unwrap().len(), 1);
        let buttons = backend.buttons.lock().unwrap().clone();
        assert_eq!(buttons, vec![(MouseButton::Left, true), (MouseButton::Left, false)]);

        // double = 两段按压
        backend.buttons.lock().unwrap().clear();
        tool.execute_with_context(r#"{"x":400,"y":300,"double":true}"#, &ctx)
            .await
            .unwrap();
        assert_eq!(backend.buttons.lock().unwrap().len(), 4);

        // 原位右键：不移动、无坐标基准也能点
        backend.moves.lock().unwrap().clear();
        let state2 = Arc::new(ScreenState::new());
        let tool2 = MouseClickTool::new(backend.clone(), state2);
        let ctx2 = make_ctx("c1-nometa").await;
        tool2.execute_with_context(r#"{"button":"right"}"#, &ctx2)
            .await
            .unwrap();
        assert!(backend.moves.lock().unwrap().is_empty());
        assert_eq!(
            backend.buttons.lock().unwrap().last().copied(),
            Some((MouseButton::Right, false))
        );

        // 半给坐标 = 参数不完整（模型笔误拦下）
        let err = tool
            .execute_with_context(r#"{"x":400}"#, &ctx)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("x 与 y 必须同时给出"));
    }

    #[tokio::test]
    async fn drag_interpolates_from_press_to_release() {
        let backend = Arc::new(FakeInputBackend::new());
        let state = Arc::new(ScreenState::new());
        state.update("d1", full_screen_meta());
        let tool = MouseDragTool::new(backend.clone(), state);
        let ctx = make_ctx("d1").await;

        tool.execute_with_context(
            r#"{"from_x":200,"from_y":150,"to_x":1000,"to_y":750}"#,
            &ctx,
        )
        .await
        .unwrap();

        let moves = backend.moves.lock().unwrap().clone();
        let buttons = backend.buttons.lock().unwrap().clone();
        // 首步 = 起点（200,150 → 物理 240,180），末步 = 终点（1000,750 → 1200,900）
        let start = (240 * 65535 / 1919, 180 * 65535 / 1079);
        let end = (1200 * 65535 / 1919, 900 * 65535 / 1079);
        assert_eq!(moves.first().copied(), Some(start));
        assert_eq!(moves.last().copied(), Some(end));
        // 1 首移 + DRAG_STEPS 步进
        assert_eq!(moves.len(), 1 + DRAG_STEPS as usize);
        // 按下在步进前、释放 在全部移动之后
        assert_eq!(buttons.first().copied(), Some((MouseButton::Left, true)));
        assert_eq!(buttons.last().copied(), Some((MouseButton::Left, false)));
    }

    #[tokio::test]
    async fn scroll_validates_and_optionally_positions() {
        let backend = Arc::new(FakeInputBackend::new());
        let state = Arc::new(ScreenState::new());
        state.update("s1", full_screen_meta());
        let tool = MouseScrollTool::new(backend.clone(), state);
        let ctx = make_ctx("s1").await;

        // 原位滚动：无需坐标基准，dx/dy 透传
        tool.execute_with_context(r#"{"dy":-3}"#, &ctx).await.unwrap();
        assert_eq!(backend.scrolls.lock().unwrap().last().copied(), Some((0, -3)));
        assert!(backend.moves.lock().unwrap().is_empty());

        // 定点滚动：先移动后滚
        tool.execute_with_context(r#"{"x":800,"y":450,"dx":2,"dy":1}"#, &ctx)
            .await
            .unwrap();
        assert_eq!(backend.scrolls.lock().unwrap().last().copied(), Some((2, 1)));
        assert_eq!(backend.moves.lock().unwrap().len(), 1);

        // 零滚动量拒绝
        let err = tool.execute_with_context(r#"{"dx":0,"dy":0}"#, &ctx).await.unwrap_err();
        assert!(err.to_string().contains("至少一个非零"));

        // 半给坐标拒绝
        let err = tool.execute_with_context(r#"{"dy":1,"x":5}"#, &ctx).await.unwrap_err();
        assert!(err.to_string().contains("x 与 y 必须同时给出"));
    }
}
