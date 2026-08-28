//! 真机冒烟测试——批次③手测面里可自动化的部分（默认 `#[ignore]`，显式运行）：
//!
//! ```text
//! SODIUM_LIB_DIR=... SODIUM_STATIC=true \
//!   cargo test --manifest-path packages/app/src-tauri/Cargo.toml --lib real_smoke -- --ignored --nocapture
//! ```
//!
//! 覆盖：真实 GDI 三路捕获（整虚拟桌面 / 单显示器 / 单窗口 PrintWindow）、
//! 虚拟桌面布局与显示器枚举的包围盒不变式、SendInput 绝对坐标在**每台显示器
//! 中心**的落点（`GetCursorPos` 读回数值核对——多屏机器天然覆盖负原点/非主屏）。
//! 不做点击/键盘/滚轮（会作用于用户当前焦点窗口）——那部分留人工手测清单。
//!
//! 产物：PNG dump 写 `%TEMP%\icepaw-screen-smoke\`，供人工复核画面正确性。
//!
//! DPI：生产环境由 tao 在运行时设 PerMonitorV2（见 backend.rs 头注释），测试
//! 进程默认不感知——DPI 缩放机器上 `GetSystemMetrics` 会拿到虚拟化坐标。测试
//! 先自己补设同一档（已设过时调用失败无害，忽略返回值）。

use std::collections::HashSet;
use std::path::PathBuf;

use windows_sys::Win32::Foundation::POINT;
use windows_sys::Win32::UI::HiDpi::{
    GetDpiForSystem, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;

use super::coords::{phys_to_absolute, sent_size_for};
use super::{GdiBackend, PhysRect, RgbaFrame, ScreenBackend};

/// 与生产同坐标基准：PerMonitorV2（物理像素）。必须在任何屏幕几何查询前调用。
fn ensure_per_monitor_dpi() {
    // SAFETY: 进程级一次性设置；已设过返回 0（失败）无害，返回值无 must_use。
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

fn dump_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("icepaw-screen-smoke");
    std::fs::create_dir_all(&dir).expect("冒烟产物目录创建失败");
    dir
}

/// 帧存 PNG（长边 ≤1600 降采样，与 capture_screen 发送档一致）。
fn dump_png(frame: RgbaFrame, name: &str) -> PathBuf {
    use image::imageops::FilterType;
    use image::DynamicImage;

    let img = image::RgbaImage::from_raw(frame.width, frame.height, frame.rgba)
        .expect("帧缓冲与尺寸不符");
    let (w, h) = sent_size_for(frame.width, frame.height, 1600);
    let out = if (w, h) == (frame.width, frame.height) {
        img
    } else {
        image::imageops::resize(&img, w, h, FilterType::Triangle)
    };
    let path = dump_dir().join(name);
    DynamicImage::ImageRgba8(out)
        .save(&path)
        .expect("PNG 写盘失败");
    path
}

/// 帧内容非退化：抽样 ≤4096 像素，唯一颜色 ≥8 种（黑屏/纯色 = 锁屏或捕获链路异常）。
fn assert_live_content(frame: &RgbaFrame, what: &str) {
    let total = frame.width as usize * frame.height as usize;
    let step = (total / 4096).max(1);
    let mut colors = HashSet::new();
    for px in frame.rgba.chunks_exact(4).step_by(step) {
        colors.insert(u32::from_le_bytes([px[0], px[1], px[2], px[3]]));
    }
    assert!(
        colors.len() >= 8,
        "{what} 抽样唯一颜色仅 {} 种——画面疑似黑屏/纯色（屏幕锁定、全部最小化，或捕获链路异常）",
        colors.len()
    );
}

fn cursor_pos() -> (i32, i32) {
    let mut pt = POINT { x: 0, y: 0 };
    // SAFETY: 写入栈上 POINT；失败（无交互会话）返回 0——冒烟环境必然有。
    assert_ne!(unsafe { GetCursorPos(&mut pt) }, 0, "GetCursorPos 失败（无交互会话？）");
    (pt.x, pt.y)
}

// =========================================================================
// 捕获链路（GDI 真硬件）
// =========================================================================

#[test]
#[ignore = "真机硬件：真实 GDI 捕获 + PNG dump（--ignored --nocapture 显式运行）"]
fn real_smoke_captures_full_monitor_window() {
    ensure_per_monitor_dpi();
    let b = GdiBackend;

    let layout = b.virtual_screen().unwrap();
    let monitors = b.monitors().unwrap();
    // SAFETY: 只读系统 DPI。
    println!("系统 DPI: {}", unsafe { GetDpiForSystem() });
    println!(
        "虚拟桌面: origin=({},{}) size={}x{}",
        layout.origin_x, layout.origin_y, layout.width, layout.height
    );
    for (i, m) in monitors.iter().enumerate() {
        println!("显示器[{i}]: ({},{}) {}x{}", m.x, m.y, m.width, m.height);
    }

    // 不变式：显示器矩形包围盒 == 虚拟桌面（SM_*VIRTUALSCREEN 的定义即并集包围盒）
    let min_x = monitors.iter().map(|m| m.x).min().unwrap();
    let min_y = monitors.iter().map(|m| m.y).min().unwrap();
    let max_r = monitors.iter().map(|m| m.right()).max().unwrap();
    let max_b = monitors.iter().map(|m| m.bottom()).max().unwrap();
    assert_eq!(
        (min_x, min_y, max_r, max_b),
        (
            layout.origin_x,
            layout.origin_y,
            layout.origin_x + layout.width,
            layout.origin_y + layout.height
        ),
        "显示器包围盒应与虚拟桌面一致"
    );

    // ① 整虚拟桌面（多屏机器含负原点区域）
    let full = PhysRect {
        x: layout.origin_x,
        y: layout.origin_y,
        width: layout.width.max(1) as u32,
        height: layout.height.max(1) as u32,
    };
    let frame = b.capture(full).unwrap();
    let dims = (frame.width, frame.height);
    assert_eq!(dims, (full.width, full.height));
    assert_live_content(&frame, "整桌面捕获");
    println!(
        "整桌面 PNG ({}x{}): {}",
        dims.0,
        dims.1,
        dump_png(frame, "full_desktop.png").display()
    );

    // ② 末位显示器（非主屏/负原点路径）
    let mi = monitors.len() - 1;
    let frame = b.capture(monitors[mi]).unwrap();
    assert_eq!((frame.width, frame.height), (monitors[mi].width, monitors[mi].height));
    assert_live_content(&frame, "末位显示器捕获");
    println!(
        "显示器[{mi}] PNG: {}",
        dump_png(frame, &format!("monitor_{mi}.png")).display()
    );

    // ③ 窗口 PrintWindow（免聚焦路径）——取前台窗口：真实可见内容有保证。
    //（首跑教训：按尺寸挑窗会选中 Kook 等应用的后台离屏渲染窗——纯色帧，
    // 捕获链路没错但内容无意义；离屏窗污染 list_windows 是已知产品债，见路线图。）
    let wins = b.windows().unwrap();
    println!("可捕获窗口数: {}", wins.len());
    let fg = b.foreground_window().expect("无前台窗口（锁屏？）");
    let title = wins
        .iter()
        .find(|w| w.hwnd == fg)
        .map(|w| w.title.clone())
        .unwrap_or_else(|| "（前台窗口未入列表——自身/工具窗）".to_string());
    let (frame, rect) = b.capture_window(fg).unwrap();
    assert_eq!((frame.width, frame.height), (rect.width, rect.height));
    assert_live_content(&frame, &format!("窗口「{title}」"));
    println!(
        "窗口「{title}」({}x{}) PNG: {}",
        rect.width,
        rect.height,
        dump_png(frame, "window.png").display()
    );
}

// =========================================================================
// 输入链路（SendInput 绝对坐标 → GetCursorPos 读回）
// =========================================================================

#[test]
#[ignore = "真机硬件：真实 SendInput 鼠标移动（光标会短暂移动，结束即还原）"]
fn real_smoke_mouse_abs_lands_on_targets() {
    ensure_per_monitor_dpi();
    let b = GdiBackend;
    let layout = b.virtual_screen().unwrap();
    let monitors = b.monitors().unwrap();

    let original = cursor_pos();
    println!("初始光标: ({},{})", original.0, original.1);

    // 目标点 = 每台显示器中心：多屏机器天然覆盖负原点/非主屏的完整换算链
    //（img→phys 由生产代码换算，这里直接从物理坐标走 phys→abs→SendInput）。
    for (i, m) in monitors.iter().enumerate() {
        let target = (
            m.x + m.width as i32 / 2,
            m.y + m.height as i32 / 2,
        );
        let (ax, ay) = phys_to_absolute(&layout, target.0, target.1);
        b.mouse_move_abs(ax, ay).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(150));
        let now = cursor_pos();
        let dist = (now.0 - target.0).abs() + (now.1 - target.1).abs();
        println!(
            "显示器[{i}] 中心 ({},{}) → abs ({ax},{ay}) → 实际落点 ({},{}) 曼哈顿偏差 {dist}",
            target.0, target.1, now.0, now.1
        );
        assert!(
            dist <= 3,
            "显示器[{i}] 落点偏差 {dist}px（目标 ({},{})，实际 ({},{})）——\
             测试期间手动动过鼠标会干扰，请勿操作并重跑",
            target.0,
            target.1,
            now.0,
            now.1
        );
    }

    // 还原初始位置（尽力而为，不断言——用户可能正在用鼠标）
    let (ax, ay) = phys_to_absolute(&layout, original.0, original.1);
    b.mouse_move_abs(ax, ay).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(150));
    println!("还原后光标: {:?}", cursor_pos());
}
