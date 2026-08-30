//! HUD 工具栏窗 + 红边框窗 的生命周期（批次④ 步骤 2，§4.7）。
//!
//! 两扇窗都**不进 tauri.conf.json**（静态窗随进程常驻；这两扇随通道开/关
//! 建/毁）：
//! - `screen-hud`：420×44 无边框置顶工具条，HUD 显示器顶部居中。内容 = 前端
//!   `/screen-hud` 路由页（事件驱动 + 1s 轮询，见 ScreenHud.vue）。
//! - `screen-frame`：透明置顶点击穿透窗，覆盖所有显示器的 union 矩形，3px
//!   danger 描边——「屏幕正在被共享」的持续可见信号。内容 = `/screen-frame`
//!   纯 CSS 页，零交互零权限。
//!
//! 全部 best-effort：窗口层失败只 warn 不反噬通道状态（通道是授权与仲裁的
//! 单位，可见性是锦上添花——A7 同款分层思路）。
//!
//! ⚠️ Rust 侧窗口 API 不受 capability 约束（ACL 只 gate 前端 invoke/plugin
//! 命令）；HUD 页需要的 `core:event:default` 在 capabilities/screen-hud.json。

use tauri::{Manager as _, WebviewUrl, WebviewWindowBuilder};

use super::channel;

pub const HUD_LABEL: &str = "screen-hud";
pub const FRAME_LABEL: &str = "screen-frame";

/// HUD 逻辑尺寸（宽×高，逻辑像素；定位换算按各显示器 scale）。
const HUD_W: f64 = 440.0;
const HUD_H: f64 = 44.0;
/// HUD 顶部边距（物理像素——紧贴屏幕顶易与系统通知区打架）。
const HUD_TOP_MARGIN_PX: i32 = 12;

/// HUD 收缩微条尺寸（逻辑像素；B7 写避让/手动收起共用形态——§4.7）。
const MINI_W: f64 = 132.0;
const MINI_H: f64 = 28.0;

/// 确保两扇窗存在（通道 Off→Active 时调用；已存在则只重定位，幂等）。
/// 创建顺序：先 frame 后 HUD（后建者在上——HUD 必须压住红边框可点）。
pub fn ensure_windows(app: &tauri::AppHandle) {
    let monitors = match app.available_monitors() {
        Ok(m) if !m.is_empty() => m,
        _ => {
            tracing::warn!(target: "ice_paw.screen_channel", "HUD 窗创建跳过：拿不到显示器列表");
            return;
        }
    };
    ensure_frame(app, &monitors);
    ensure_hud(app, &monitors, channel::global().hud_monitor());
}

/// 摧毁两扇窗（通道 Active→Off 时调用；不存在则无事，幂等）。
pub fn destroy_windows(app: &tauri::AppHandle) {
    for label in [HUD_LABEL, FRAME_LABEL] {
        if let Some(win) = app.get_webview_window(label) {
            if let Err(e) = win.close() {
                tracing::warn!(target: "ice_paw.screen_channel", label, error = %e, "HUD 窗关闭失败");
            }
        }
    }
}

/// HUD 切显示器（cycle 命令调用）：重定位到目标显示器顶部居中。
pub fn move_hud(app: &tauri::AppHandle, index: usize) {
    let monitors = match app.available_monitors() {
        Ok(m) if !m.is_empty() => m,
        _ => return,
    };
    ensure_hud(app, &monitors, index);
}

/// HUD 窗形态切换（B7 写避让 / 手动收起，前端按 writing/collapsed 态驱动）：
/// - full：440×44 HUD 显示器顶部居中；
/// - mini：132×28 同显示器**右上角**微条；
/// - passthrough：点击穿透（仅写执行中的自动收缩用——那几秒用户点不到 HUD
///   是预期[操作在跑，别挡]；手动收起必须可点回，恒不穿透）。
pub fn set_form(app: &tauri::AppHandle, mini: bool, passthrough: bool) {
    let Some(win) = app.get_webview_window(HUD_LABEL) else {
        return;
    };
    let monitors = app.available_monitors().unwrap_or_default();
    let Some(m) = monitors.get(channel::global().hud_monitor()) else {
        return;
    };
    let scale = m.scale_factor();
    let (w, h, pos) = if mini {
        let w = MINI_W * scale;
        let h = MINI_H * scale;
        let pos = tauri::PhysicalPosition::new(
            m.position().x + m.size().width as i32 - w.round() as i32 - HUD_TOP_MARGIN_PX,
            m.position().y + HUD_TOP_MARGIN_PX,
        );
        (w, h, pos)
    } else {
        let w = HUD_W * scale;
        let h = HUD_H * scale;
        let pos = tauri::PhysicalPosition::new(
            m.position().x + ((m.size().width as f64 - w) / 2.0).round() as i32,
            m.position().y + HUD_TOP_MARGIN_PX,
        );
        (w, h, pos)
    };
    let _ = win.set_size(tauri::PhysicalSize::new(w.round() as u32, h.round() as u32));
    let _ = win.set_position(pos);
    let _ = win.set_ignore_cursor_events(passthrough);
}

fn ensure_hud(app: &tauri::AppHandle, monitors: &[tauri::Monitor], index: usize) {
    let Some(m) = monitors.get(index) else {
        tracing::warn!(target: "ice_paw.screen_channel", index, "HUD 显示器索引越界，回落 0");
        return ensure_hud(app, monitors, 0);
    };
    let scale = m.scale_factor();
    // 物理坐标定位：inner_size 是逻辑值，换算物理宽再居中（顶边距固定物理 12px）。
    let hud_w_phys = HUD_W * scale;
    let pos_x = m.position().x as f64 + (m.size().width as f64 - hud_w_phys) / 2.0;
    let pos_y = m.position().y + HUD_TOP_MARGIN_PX;
    let pos = tauri::PhysicalPosition::new(pos_x.round() as i32, pos_y);

    if let Some(win) = app.get_webview_window(HUD_LABEL) {
        let _ = win.set_position(pos);
        let _ = win.set_always_on_top(true); // 重新断言置顶（防被其它 topmost 压住）
        return;
    }
    let built = WebviewWindowBuilder::new(app, HUD_LABEL, WebviewUrl::App("screen-hud".into()))
        .title("IcePaw 屏幕共享")
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .shadow(false)
        .inner_size(HUD_W, HUD_H)
        .position(pos.x as f64, pos.y as f64)
        .focused(false) // 不偷主窗焦点（HUD 是仪表不是对话框）
        .build();
    match built {
        Ok(_) => tracing::info!(target: "ice_paw.screen_channel", index, "HUD 工具栏窗已创建"),
        Err(e) => tracing::warn!(target: "ice_paw.screen_channel", error = %e, "HUD 工具栏窗创建失败"),
    }
}

fn ensure_frame(app: &tauri::AppHandle, monitors: &[tauri::Monitor]) {
    // union 所有显示器矩形（含负坐标副屏）——透明窗铺满整个虚拟桌面。
    let mut x0 = i32::MAX;
    let mut y0 = i32::MAX;
    let mut x1 = i32::MIN;
    let mut y1 = i32::MIN;
    for m in monitors {
        let p = m.position();
        let s = m.size();
        x0 = x0.min(p.x);
        y0 = y0.min(p.y);
        x1 = x1.max(p.x + s.width as i32);
        y1 = y1.max(p.y + s.height as i32);
    }
    let (phys_w, phys_h) = ((x1 - x0).max(1), (y1 - y0).max(1));
    if let Some(win) = app.get_webview_window(FRAME_LABEL) {
        // 已存在：重对齐（显示器配置可能变了——分辨率/增删屏）
        let _ = win.set_position(tauri::PhysicalPosition::new(x0, y0));
        let _ = win.set_size(tauri::PhysicalSize::new(phys_w, phys_h));
        let _ = win.set_always_on_top(true);
        return;
    }
    // inner_size/position(builder) 是逻辑值：按主显示器 scale 换算。
    // ⚠️ 混合 DPI（各屏缩放不同）下单窗跨屏本就无法逐屏精确——红边框只是
    // 可见性信号，容差可接受；主流配置（同缩放比）下精确。
    let scale = app
        .primary_monitor()
        .ok()
        .flatten()
        .map(|m| m.scale_factor())
        .unwrap_or(1.0);
    let built = WebviewWindowBuilder::new(
        app,
        FRAME_LABEL,
        WebviewUrl::App("screen-frame".into()),
    )
    .title("IcePaw 屏幕共享边框")
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .closable(false)
    .shadow(false)
    .inner_size(phys_w as f64 / scale, phys_h as f64 / scale)
    .position(x0 as f64 / scale, y0 as f64 / scale)
    .focused(false)
    .build();
    let win = match built {
        Ok(win) => win,
        Err(e) => {
            tracing::warn!(target: "ice_paw.screen_channel", error = %e, "红边框窗创建失败");
            return;
        }
    };
    // 点击穿透：红边框是给眼睛的，不挡任何输入（builder 无此项，建后设置）。
    if let Err(e) = win.set_ignore_cursor_events(true) {
        tracing::warn!(target: "ice_paw.screen_channel", error = %e, "红边框窗设置点击穿透失败");
    }
}
