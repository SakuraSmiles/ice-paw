//! [`ScreenBackend`] —— Win32 调用与工具逻辑解耦的可注入后端。
//!
//! - [`GdiBackend`]（`#[cfg(windows)]`）：GDI `BitBlt` 捕获。选 GDI 而非 DXGI：
//!   零新 crate、API 面小；已知盲区（DRM/HDR 内容、独占全屏黑块、UAC 安全桌面）
//!   以家族前缀错误诚实暴露，DXGI 是远期升级路。
//! - [`UnsupportedBackend`]（非 Windows）：同 schema 注册、返回「仅支持 Windows」
//!   家族错误——工具列表跨平台一致，模型拿到的失败是可读的自文档。
//! - 测试用 Fake 实现直接在测试模块里造（纯色缓冲），不进生产代码。
//!
//! 线程模型：GDI 调用全是阻塞的同步 API，trait 方法即同步；工具层用
//! `spawn_blocking` 把捕获挪出 async runtime（BitBlt 大屏可耗时几十 ms）。
//!
//! DPI：tao 在运行时调 SetProcessDpiAwarenessContext 设 PerMonitorV2（非清单
//! 声明——测试进程无 tao 时需自设同档，见 real_smoke.rs），`GetSystemMetrics`
//! 与 BitBlt 坐标同为物理像素——捕获与 SendInput 共用同一坐标空间，无换算。

use crate::error::{AppError, AppResult};

use super::coords::{PhysRect, VirtualScreenLayout};

// =========================================================================
// 数据形状
// =========================================================================

/// 后端产出的未压缩帧（RGBA 8bit、行序自上而下）。
///
/// GDI 原生产出 BGRA + 自下而上行序，`GdiBackend` 内部已翻转/换位——
/// 本结构之后的所有处理（降采样/PNG 编码）与后端无关。
pub struct RgbaFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

// =========================================================================
// Trait
// =========================================================================

/// 可捕获窗口的摘要信息（`list_windows` 产出）。
///
/// `hwnd` 是稳定句柄值（窗口存活期内不变），模型把它原样传回
/// `capture_window` 即可精确锁定窗口——比标题匹配可靠（同名窗口/标题变动）。
#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub hwnd: i64,
    pub title: String,
    pub rect: PhysRect,
}

/// 鼠标按键（输入方法用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// 屏幕捕获后端。
///
/// 阶段一（看屏）：虚拟桌面布局 / 显示器枚举 / 区域捕获 / 窗口枚举与捕获。
/// 阶段二（操作）：鼠标三原语（移动 / 按键 / 滚轮）——工具层负责全部坐标数学
/// （`coords::phys_to_absolute`），后端只收 SendInput 绝对坐标，零换算。
pub trait ScreenBackend: Send + Sync {
    /// 后端名（写进截图摘要，诊断用）。
    fn name(&self) -> &'static str;

    /// 虚拟桌面布局快照。截图时存进 [`super::coords::CaptureMeta`]，
    /// 输入前重取对比——布局变了 = 全部坐标过期。
    fn virtual_screen(&self) -> AppResult<VirtualScreenLayout>;

    /// 显示器矩形列表（`EnumDisplayMonitors` 顺序；index 即工具 `monitor` 参数）。
    fn monitors(&self) -> AppResult<Vec<PhysRect>>;

    /// 捕获物理区域（虚拟桌面绝对坐标，可为负原点）→ RGBA 帧。
    fn capture(&self, region: PhysRect) -> AppResult<RgbaFrame>;

    /// 枚举可捕获窗口（实现负责过滤：可见 + 有标题 + 非工具窗 + 非自身进程 +
    /// 非 DWM cloaked；最小化窗口保留在列表里但捕获时会诚实报错）。
    fn windows(&self) -> AppResult<Vec<WindowInfo>>;

    /// 按句柄捕获窗口（PrintWindow 免聚焦，被遮挡窗口可捕获）。
    ///
    /// 返回帧 + 捕获时刻的窗口矩形（写进 CaptureMeta.phys_region——
    /// 窗口会移动，坐标基准锚定在捕获那一刻的位置）。
    fn capture_window(&self, hwnd: i64) -> AppResult<(RgbaFrame, PhysRect)>;

    /// 前台窗口句柄（无前台窗口/锁屏时 None）。
    fn foreground_window(&self) -> Option<i64>;

    // -------------------- 阶段二：输入原语（操作） --------------------

    /// 移动鼠标到 SendInput 绝对坐标（`MOUSEEVENTF_ABSOLUTE|VIRTUALDESK`，
    /// 0..=65535——工具层用 [`super::coords::phys_to_absolute`] 算好）。
    fn mouse_move_abs(&self, abs_x: i32, abs_y: i32) -> AppResult<()>;

    /// 按下/释放鼠标按键（drag/click 由工具层组合）。
    fn mouse_button(&self, button: MouseButton, down: bool) -> AppResult<()>;

    /// 滚轮（刻数 notch：dy 正=向上、dx 正=向右；`WHEEL_DELTA=120` 由后端换算）。
    /// 两个分量都非零时分两次事件发。
    fn mouse_scroll(&self, dx_notches: i32, dy_notches: i32) -> AppResult<()>;

    /// 按下/释放一个虚拟键（VK 码；组合键由工具层解析拼装）。
    fn key_vk(&self, vk: u16, down: bool) -> AppResult<()>;

    /// 按下/释放一个 Unicode 字符（UTF-16 单元——BMP 外字符由工具层
    /// 拆代理对逐单元发；type_text 用）。
    fn key_unicode(&self, unit: u16, down: bool) -> AppResult<()>;
}

// =========================================================================
// GDI 实现（Windows）
// =========================================================================

#[cfg(windows)]
mod gdi {
    use super::*;

    use windows_sys::Win32::Foundation::{GetLastError, LPARAM, RECT};
    use windows_sys::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_KEYUP,
        KEYEVENTF_UNICODE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN,
        MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE,
        MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL,
        MOUSEINPUT,
    };
    use windows_sys::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject,
        EnumDisplayMonitors, GetDC, GetDIBits, ReleaseDC, SelectObject, BITMAPINFO,
        BITMAPINFOHEADER, CAPTUREBLT, DIB_RGB_COLORS, SRCCOPY,
    };
    // PrintWindow 在 windows-sys 0.59 被归进 Xps 打印路径的 feature（Win32_Storage_Xps）
    use windows_sys::Win32::Storage::Xps::PrintWindow;
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetForegroundWindow, GetSystemMetrics, GetWindowLongW, GetWindowRect,
        GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
        GWL_EXSTYLE, PW_RENDERFULLCONTENT, WS_EX_TOOLWINDOW, SM_CXVIRTUALSCREEN,
        SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    };

    type Hwnd = windows_sys::Win32::Foundation::HWND;

    /// GDI 截屏后端（无状态，可全局单例）。
    pub struct GdiBackend;

    impl ScreenBackend for GdiBackend {
        fn name(&self) -> &'static str {
            "gdi"
        }

        fn virtual_screen(&self) -> AppResult<VirtualScreenLayout> {
            // SAFETY: GetSystemMetrics 只读系统指标，无句柄/状态。
            let (x, y, w, h) = unsafe {
                (
                    GetSystemMetrics(SM_XVIRTUALSCREEN),
                    GetSystemMetrics(SM_YVIRTUALSCREEN),
                    GetSystemMetrics(SM_CXVIRTUALSCREEN),
                    GetSystemMetrics(SM_CYVIRTUALSCREEN),
                )
            };
            if w <= 0 || h <= 0 {
                return Err(AppError::Internal(format!(
                    "screen 捕获失败: 虚拟桌面尺寸非法（{w}×{h}）。\
                     可能是显示驱动正在切换模式（插拔显示器/改分辨率），稍后重试；\
                     持续出现请检查显卡驱动"
                )));
            }
            Ok(VirtualScreenLayout {
                origin_x: x,
                origin_y: y,
                width: w,
                height: h,
            })
        }

        fn monitors(&self) -> AppResult<Vec<PhysRect>> {
            let mut out: Vec<PhysRect> = Vec::new();
            // SAFETY: 回调只把 RECT 追加进 out；枚举期间 out 独占（同步单线程），
            // LPARAM 回传指针即约定通道。返回 0（FALSE）表示枚举被中止。
            let ok = unsafe {
                EnumDisplayMonitors(
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    Some(enum_monitor_proc),
                    &mut out as *mut Vec<PhysRect> as LPARAM,
                )
            };
            if ok == 0 || out.is_empty() {
                return Err(AppError::Internal(format!(
                    "screen 捕获失败: 显示器枚举无结果（GDI 错误码 {}）。\
                     系统可能处于无显示会话（远程桌面断开/无头模式）",
                    unsafe { GetLastError() }
                )));
            }
            Ok(out)
        }

        fn capture(&self, region: PhysRect) -> AppResult<RgbaFrame> {
            let (w, h) = (region.width as i32, region.height as i32);
            if w <= 0 || h <= 0 {
                return Err(AppError::Validation(format!(
                    "screen 捕获失败: 捕获区域尺寸非法（{w}×{h}），宽高必须 ≥ 1"
                )));
            }
            // SAFETY: 以下 GDI 序列是经典 BitBlt 截屏模板——句柄创建后逐一回滚释放
            //（错误路径同样 DeleteDC/DeleteObject，不泄漏）；无跨线程共享。
            unsafe {
                let hdc_screen = GetDC(std::ptr::null_mut());
                if hdc_screen.is_null() {
                    return Err(capture_err("GetDC 失败", "无法访问屏幕设备上下文"));
                }
                let hdc_mem = CreateCompatibleDC(hdc_screen);
                let hbmp = if !hdc_mem.is_null() {
                    CreateCompatibleBitmap(hdc_screen, w, h)
                } else {
                    std::ptr::null_mut()
                };
                if hdc_mem.is_null() || hbmp.is_null() {
                    release_capture_objects(hdc_screen, hdc_mem, hbmp, std::ptr::null_mut());
                    return Err(capture_err(
                        "创建兼容 DC/位图失败",
                        "系统资源不足或显示驱动异常，稍后重试",
                    ));
                }
                let old = SelectObject(hdc_mem, hbmp);
                // CAPTUREBLT：把分层窗口（WS_EX_LAYERED，如部分悬浮窗）也捕进来；
                // 代价是鼠标光标可能闪烁一帧——截图工具的标准取舍。
                let blt = BitBlt(
                    hdc_mem,
                    0,
                    0,
                    w,
                    h,
                    hdc_screen,
                    region.x,
                    region.y,
                    SRCCOPY | CAPTUREBLT,
                );
                if blt == 0 {
                    release_capture_objects(hdc_screen, hdc_mem, hbmp, old);
                    return Err(capture_err(
                        "BitBlt 失败",
                        "目标区域可能位于已断开的显示器（坐标过期），重新截图后再试；\
                         独占全屏/DRM 内容是 GDI 已知盲区",
                    ));
                }
                // GetDIBits：biHeight 取负 = 自上而下行序，免手动翻转行序。
                let mut bmi: BITMAPINFO = std::mem::zeroed();
                bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
                bmi.bmiHeader.biWidth = w;
                bmi.bmiHeader.biHeight = -h;
                bmi.bmiHeader.biPlanes = 1;
                bmi.bmiHeader.biBitCount = 32;
                bmi.bmiHeader.biCompression = 0; // BI_RGB
                let mut buf = vec![0u8; (w as usize) * (h as usize) * 4];
                let got = GetDIBits(
                    hdc_mem,
                    hbmp,
                    0,
                    h as u32,
                    buf.as_mut_ptr().cast(),
                    &mut bmi,
                    DIB_RGB_COLORS,
                );
                release_capture_objects(hdc_screen, hdc_mem, hbmp, old);
                if got == 0 {
                    return Err(capture_err("GetDIBits 失败", "读取像素位图失败，稍后重试"));
                }
                // GDI 32bpp 是 BGRA —— 换位成 RGBA 供 image crate 直用（alpha 压 255）。
                bgra_to_opaque_rgba(&mut buf);
                Ok(RgbaFrame {
                    width: w as u32,
                    height: h as u32,
                    rgba: buf,
                })
            }
        }

        fn windows(&self) -> AppResult<Vec<WindowInfo>> {
            let mut out: Vec<WindowInfo> = Vec::new();
            // SAFETY: 枚举回调与 monitors 同通道模式（LPARAM 回传 Vec 指针）。
            let ok = unsafe {
                EnumWindows(
                    Some(enum_window_proc),
                    &mut out as *mut Vec<WindowInfo> as LPARAM,
                )
            };
            if ok == 0 {
                return Err(capture_err("EnumWindows 中止", "窗口枚举失败，稍后重试"));
            }
            Ok(out)
        }

        fn capture_window(&self, hwnd: i64) -> AppResult<(RgbaFrame, PhysRect)> {
            let hwnd = hwnd as Hwnd;
            // SAFETY: 句柄来自模型回传——所有查询都做失败防御，不做存活假设。
            unsafe {
                if IsIconic(hwnd) != 0 {
                    return Err(AppError::Validation(
                        "screen 捕获失败: 目标窗口已最小化，PrintWindow 无法渲染最小化窗口\
                         ——请让用户还原窗口，或改用 capture_screen 截全屏".into(),
                    ));
                }
                let rect = window_rect(hwnd).ok_or_else(|| {
                    AppError::Validation(
                        "screen 捕获失败: 窗口不存在或矩形不可得——句柄可能已失效\
                         （窗口被关闭），请重新 list_windows".into(),
                    )
                })?;
                let (w, h) = (rect.width as i32, rect.height as i32);
                let hdc_screen = GetDC(std::ptr::null_mut());
                let hdc_mem = CreateCompatibleDC(hdc_screen);
                let hbmp = if !hdc_mem.is_null() {
                    CreateCompatibleBitmap(hdc_screen, w, h)
                } else {
                    std::ptr::null_mut()
                };
                if hdc_mem.is_null() || hbmp.is_null() {
                    release_capture_objects(hdc_screen, hdc_mem, hbmp, std::ptr::null_mut());
                    return Err(capture_err(
                        "创建窗口捕获 DC/位图失败",
                        "系统资源不足或显示驱动异常，稍后重试",
                    ));
                }
                let old = SelectObject(hdc_mem, hbmp);
                // PW_RENDERFULLCONTENT：连 DirectX 渲染内容（Chrome/Electron/游戏 UI）
                // 一起渲染；个别老应用不认 FULLCONTENT，失败降级普通 PrintWindow 再试。
                let mut printed = PrintWindow(hwnd, hdc_mem, PW_RENDERFULLCONTENT);
                if printed == 0 {
                    printed = PrintWindow(hwnd, hdc_mem, 0);
                }
                if printed == 0 {
                    release_capture_objects(hdc_screen, hdc_mem, hbmp, old);
                    return Err(AppError::Validation(
                        "screen 捕获失败: PrintWindow 渲染失败——该窗口可能不允许抓取\
                         （DRM 保护/特殊渲染管线），可改用 capture_screen 截其所在区域".into(),
                    ));
                }
                let mut bmi: BITMAPINFO = std::mem::zeroed();
                bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
                bmi.bmiHeader.biWidth = w;
                bmi.bmiHeader.biHeight = -h;
                bmi.bmiHeader.biPlanes = 1;
                bmi.bmiHeader.biBitCount = 32;
                let mut buf = vec![0u8; (w as usize) * (h as usize) * 4];
                let got = GetDIBits(
                    hdc_mem,
                    hbmp,
                    0,
                    h as u32,
                    buf.as_mut_ptr().cast(),
                    &mut bmi,
                    DIB_RGB_COLORS,
                );
                release_capture_objects(hdc_screen, hdc_mem, hbmp, old);
                if got == 0 {
                    return Err(capture_err("GetDIBits 失败", "读取窗口像素位图失败，稍后重试"));
                }
                bgra_to_opaque_rgba(&mut buf);
                Ok((
                    RgbaFrame {
                        width: w as u32,
                        height: h as u32,
                        rgba: buf,
                    },
                    rect,
                ))
            }
        }

        fn foreground_window(&self) -> Option<i64> {
            // SAFETY: GetForegroundWindow 只读，无句柄则返回 null。
            let hwnd = unsafe { GetForegroundWindow() };
            if hwnd.is_null() {
                None
            } else {
                Some(hwnd as i64)
            }
        }

        fn mouse_move_abs(&self, abs_x: i32, abs_y: i32) -> AppResult<()> {
            send_mouse(MOUSEINPUT {
                dx: abs_x,
                dy: abs_y,
                mouseData: 0,
                dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                time: 0,
                dwExtraInfo: 0,
            })
        }

        fn mouse_button(&self, button: MouseButton, down: bool) -> AppResult<()> {
            let flags = match (button, down) {
                (MouseButton::Left, true) => MOUSEEVENTF_LEFTDOWN,
                (MouseButton::Left, false) => MOUSEEVENTF_LEFTUP,
                (MouseButton::Right, true) => MOUSEEVENTF_RIGHTDOWN,
                (MouseButton::Right, false) => MOUSEEVENTF_RIGHTUP,
                (MouseButton::Middle, true) => MOUSEEVENTF_MIDDLEDOWN,
                (MouseButton::Middle, false) => MOUSEEVENTF_MIDDLEUP,
            };
            send_mouse(MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            })
        }

        fn mouse_scroll(&self, dx_notches: i32, dy_notches: i32) -> AppResult<()> {
            if dy_notches != 0 {
                send_mouse(MOUSEINPUT {
                    dx: 0,
                    dy: 0,
                    // 负刻数（向下/向左）经补码落进 DWORD——API 即按全 32 位解读。
                    mouseData: (dy_notches * WHEEL_DELTA_UNITS) as u32,
                    dwFlags: MOUSEEVENTF_WHEEL,
                    time: 0,
                    dwExtraInfo: 0,
                })?;
            }
            if dx_notches != 0 {
                send_mouse(MOUSEINPUT {
                    dx: 0,
                    dy: 0,
                    mouseData: (dx_notches * WHEEL_DELTA_UNITS) as u32,
                    dwFlags: MOUSEEVENTF_HWHEEL,
                    time: 0,
                    dwExtraInfo: 0,
                })?;
            }
            Ok(())
        }

        fn key_vk(&self, vk: u16, down: bool) -> AppResult<()> {
            send_keyboard(KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: if down { 0 } else { KEYEVENTF_KEYUP },
                time: 0,
                dwExtraInfo: 0,
            })
        }

        fn key_unicode(&self, unit: u16, down: bool) -> AppResult<()> {
            let mut flags = KEYEVENTF_UNICODE;
            if !down {
                flags |= KEYEVENTF_KEYUP;
            }
            send_keyboard(KEYBDINPUT {
                wVk: 0,
                wScan: unit,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            })
        }
    }

    /// 滚轮一格的 SendInput 单位数（`WHEEL_DELTA` 恒 120；不 import 系统常量，
    /// windows-sys 各版本对它的模块归属漂过）。
    const WHEEL_DELTA_UNITS: i32 = 120;

    /// 发一条鼠标 SendInput 事件；被阻断（安全软件/UIPI 提权目标）时家族化报错。
    fn send_mouse(mi: MOUSEINPUT) -> AppResult<()> {
        // SAFETY: INPUT 是纯数据结构；zeroed 后逐字段填值，绕过 union 命名
        //（windows-sys 各版本对 INPUT 内嵌 union 的类型名不稳定）。
        let mut input: INPUT = unsafe { std::mem::zeroed() };
        input.r#type = INPUT_MOUSE;
        input.Anonymous.mi = mi;
        // SAFETY: SendInput 只读输入数组，无句柄/状态；返回实际注入条数。
        let sent = unsafe { SendInput(1, &input, std::mem::size_of::<INPUT>() as i32) };
        if sent != 1 {
            return Err(AppError::Internal(
                "screen 输入失败: SendInput 鼠标事件被系统拒绝——\
                 目标可能是提权窗口（UIPI 拦截非同权限输入）或安全软件/反作弊\
                 拦截了输入模拟；请改用需要用户手动完成的替代方式".into(),
            ));
        }
        Ok(())
    }

    /// 发一条键盘 SendInput 事件（与 [`send_mouse`] 同错误家族）。
    fn send_keyboard(ki: KEYBDINPUT) -> AppResult<()> {
        // SAFETY: 同 send_mouse——zeroed INPUT 绕 union 命名，逐字段填值。
        let mut input: INPUT = unsafe { std::mem::zeroed() };
        input.r#type = INPUT_KEYBOARD;
        input.Anonymous.ki = ki;
        // SAFETY: SendInput 只读输入数组；返回实际注入条数。
        let sent = unsafe { SendInput(1, &input, std::mem::size_of::<INPUT>() as i32) };
        if sent != 1 {
            return Err(AppError::Internal(
                "screen 输入失败: SendInput 键盘事件被系统拒绝——\
                 目标可能是提权窗口（UIPI 拦截非同权限输入）或安全软件/反作弊\
                 拦截了输入模拟；请改用需要用户手动完成的替代方式".into(),
            ));
        }
        Ok(())
    }

    /// GDI 错误家族化：`screen 捕获失败: <步骤>（GDI 错误码 N）——<怎么办>`。
    /// 错误首行 = 稳定家族前缀（doom_detect 冒号切分依赖），步骤词在前。
    fn capture_err(step: &str, hint: &str) -> AppError {
        // SAFETY: GetLastError 是线程槽查询，无副作用。
        let gle = unsafe { GetLastError() };
        AppError::Internal(format!("screen 捕获失败: {step}（GDI 错误码 {gle}）——{hint}"))
    }

    /// GDI 32bpp DIB 是 BGRA，且 alpha 字节文档未定义（实测 DWM 屏幕路径常 255，
    /// 但 RDP/服务会话等环境见过 0）——统一换位 + 压 255：PNG 侧永远不透明，
    /// 红蓝通道对调也在此一处钉死（契约测试见 mod gdi_contract_tests）。
    fn bgra_to_opaque_rgba(buf: &mut [u8]) {
        for px in buf.chunks_exact_mut(4) {
            px.swap(0, 2);
            px[3] = 255;
        }
    }

    /// 窗口标题（空标题返回空串；Win32 窗口标题无长度上限约定，动态探测）。
    ///
    /// # Safety
    /// `hwnd` 必须是存活窗口句柄（调用方已过 EnumWindows/前台来源）。
    unsafe fn window_title(hwnd: Hwnd) -> String {
        // SAFETY: GetWindowTextW 写入的缓冲区按 GetWindowTextLengthW 预留 +1 NUL。
        unsafe {
            let len = GetWindowTextLengthW(hwnd);
            if len <= 0 {
                return String::new();
            }
            let mut buf = vec![0u16; len as usize + 1];
            let got = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
            if got <= 0 {
                return String::new();
            }
            String::from_utf16_lossy(&buf[..got as usize])
        }
    }

    /// DWM cloaked 判定：cloaked 窗口（UWP 挂起的幽灵窗/另一虚拟桌面的窗口）
    /// 不可见且 PrintWindow 产出空帧，枚举时直接排除。
    ///
    /// # Safety
    /// `hwnd` 必须是存活窗口句柄。
    unsafe fn is_cloaked(hwnd: Hwnd) -> bool {
        // SAFETY: DwmGetWindowAttribute 写入 4 字节 u32；失败（如 DWM 未运行）
        // 视为未 cloaked——宁多列不漏列（列出的坏窗口捕获时会诚实报错）。
        unsafe {
            let mut cloaked: u32 = 0;
            let hr = DwmGetWindowAttribute(
                hwnd,
                DWMWA_CLOAKED as u32,
                &mut cloaked as *mut u32 as *mut core::ffi::c_void,
                std::mem::size_of::<u32>() as u32,
            );
            hr == 0 && cloaked != 0
        }
    }

    /// 窗口在屏幕上的矩形（失败返回 None——窗口销毁竞态）。
    ///
    /// # Safety
    /// `hwnd` 必须是存活窗口句柄。
    unsafe fn window_rect(hwnd: Hwnd) -> Option<PhysRect> {
        // SAFETY: GetWindowRect 写入调用方栈上 RECT。
        unsafe {
            let mut r = RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            if GetWindowRect(hwnd, &mut r) == 0 {
                return None;
            }
            let width = (r.right - r.left).max(0) as u32;
            let height = (r.bottom - r.top).max(0) as u32;
            if width == 0 || height == 0 {
                return None;
            }
            Some(PhysRect {
                x: r.left,
                y: r.top,
                width,
                height,
            })
        }
    }

    /// 释放一次捕获涉及的三个 GDI 对象（old 为 SelectObject 的返回值，可能为 1/0 之外的
    /// 无效值时跳过回选）。错误路径与成功路径共用，防句柄泄漏。
    ///
    /// # Safety
    /// 三个句柄必须来自同一次 GetDC/CreateCompatibleDC/CreateCompatibleBitmap 且未释放过。
    unsafe fn release_capture_objects(
        hdc_screen: windows_sys::Win32::Graphics::Gdi::HDC,
        hdc_mem: windows_sys::Win32::Graphics::Gdi::HDC,
        hbmp: windows_sys::Win32::Graphics::Gdi::HBITMAP,
        old: windows_sys::Win32::Graphics::Gdi::HGDIOBJ,
    ) {
        if !old.is_null() {
            SelectObject(hdc_mem, old);
        }
        if !hbmp.is_null() {
            DeleteObject(hbmp);
        }
        if !hdc_mem.is_null() {
            DeleteDC(hdc_mem);
        }
        if !hdc_screen.is_null() {
            ReleaseDC(std::ptr::null_mut(), hdc_screen);
        }
    }

    /// `EnumDisplayMonitors` 回调：把显示器 RECT 追加进 LPARAM 携带的 Vec。
    unsafe extern "system" fn enum_monitor_proc(
        _hmon: windows_sys::Win32::Graphics::Gdi::HMONITOR,
        _hdc: windows_sys::Win32::Graphics::Gdi::HDC,
        rect: *mut RECT,
        data: LPARAM,
    ) -> i32 {
        // SAFETY: data 由调用方传 &mut Vec（同步独占）；rect 指向系统给的临时 RECT。
        let out = unsafe { &mut *(data as *mut Vec<PhysRect>) };
        if let Some(r) = unsafe { rect.as_ref() } {
            out.push(PhysRect {
                x: r.left,
                y: r.top,
                width: (r.right - r.left).max(0) as u32,
                height: (r.bottom - r.top).max(0) as u32,
            });
        }
        1 // TRUE：继续枚举
    }

    /// `EnumWindows` 回调：五条过滤（可见/有标题/非工具窗/非自身/非 cloaked）
    /// 全过才进结果。返回 TRUE 继续枚举——过滤失败不中止枚举（漏一个好过断列）。
    unsafe extern "system" fn enum_window_proc(hwnd: Hwnd, data: LPARAM) -> i32 {
        // SAFETY: 同 monitors 通道模式；窗口查询全部只读。
        unsafe {
            let out = &mut *(data as *mut Vec<WindowInfo>);
            if IsWindowVisible(hwnd) == 0 {
                return 1;
            }
            let title = window_title(hwnd);
            if title.is_empty() {
                return 1;
            }
            if GetWindowLongW(hwnd, GWL_EXSTYLE) & WS_EX_TOOLWINDOW as i32 != 0 {
                return 1;
            }
            let mut pid: u32 = 0;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid != 0 && pid == GetCurrentProcessId() {
                return 1; // 自身窗口（IcePaw 主窗/托盘）不进列表——不遮自身窗
            }
            if is_cloaked(hwnd) {
                return 1;
            }
            let Some(rect) = window_rect(hwnd) else {
                return 1;
            };
            out.push(WindowInfo {
                hwnd: hwnd as i64,
                title,
                rect,
            });
        }
        1
    }

    /// GDI 字节序契约（无头、确定性，随常规套件跑）：`SetPixel` 写已知色 →
    /// 走与 capture/capture_window **完全相同形状**的 GetDIBits（负 biHeight
    /// 顶向下 / 32bpp）→ 过 `bgra_to_opaque_rgba` → 断言通道序与行序。
    /// 红蓝对调是方差类数值检查抓不住的（对调后的图照样五颜六色），只有
    /// 契约测试能钉死——真机冒烟（real_smoke.rs）管内容，这里管字节。
    #[cfg(test)]
    mod gdi_contract_tests {
        use super::bgra_to_opaque_rgba;
        use windows_sys::Win32::Graphics::Gdi::{
            CreateBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
            ReleaseDC, SelectObject, SetPixel, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS,
        };

        #[test]
        fn gdi_32bpp_dib_is_bgra_top_down() {
            const W: i32 = 8;
            const H: i32 = 6;
            // SAFETY: 与生产捕获序列同模板——句柄逐一回滚释放，无跨线程共享。
            unsafe {
                let hdc_screen = GetDC(std::ptr::null_mut());
                assert!(!hdc_screen.is_null(), "GetDC 失败（无显示会话？）");
                let hdc_mem = CreateCompatibleDC(hdc_screen);
                // 直接造 32bpp DDB（不随屏幕色深漂移），SetPixel 写 COLORREF=0x00BBGGRR
                let hbmp = CreateBitmap(W, H, 1, 32, std::ptr::null());
                assert!(!hdc_mem.is_null() && !hbmp.is_null(), "内存 DC/位图创建失败");
                let old = SelectObject(hdc_mem, hbmp);
                // 顶行区：红(2,2) 绿(4,2) 蓝(6,2)；末行：白(2,5) 黑(4,5)
                for (x, y, c) in [
                    (2, 2, 0xFFu32),     // RGB(255,0,0) 纯红
                    (4, 2, 0xFF_00),     // RGB(0,255,0) 纯绿
                    (6, 2, 0xFF_0000),   // RGB(0,0,255) 纯蓝
                    (2, 5, 0xFF_FF_FF),  // 白
                    (4, 5, 0x00_00_00),  // 黑
                ] {
                    assert_ne!(SetPixel(hdc_mem, x, y, c), u32::MAX, "SetPixel({x},{y}) 失败");
                }
                // 与生产完全相同的 GetDIBits 形状（负 biHeight = 自上而下）
                let mut bmi: BITMAPINFO = std::mem::zeroed();
                bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
                bmi.bmiHeader.biWidth = W;
                bmi.bmiHeader.biHeight = -H;
                bmi.bmiHeader.biPlanes = 1;
                bmi.bmiHeader.biBitCount = 32;
                bmi.bmiHeader.biCompression = 0; // BI_RGB
                let mut buf = vec![0u8; (W as usize) * (H as usize) * 4];
                let got = GetDIBits(
                    hdc_mem,
                    hbmp,
                    0,
                    H as u32,
                    buf.as_mut_ptr().cast(),
                    &mut bmi,
                    DIB_RGB_COLORS,
                );
                SelectObject(hdc_mem, old);
                DeleteObject(hbmp);
                DeleteDC(hdc_mem);
                ReleaseDC(std::ptr::null_mut(), hdc_screen);
                assert_ne!(got, 0, "GetDIBits 失败");

                bgra_to_opaque_rgba(&mut buf);
                let px = |x: i32, y: i32| -> [u8; 4] {
                    let i = ((y * W + x) * 4) as usize;
                    buf[i..i + 4].try_into().unwrap()
                };
                // 通道序：BGRA 换位后 R 在首字节（若漏换位，纯红像素会是 [0,0,255,255]）
                assert_eq!(px(2, 2), [255, 0, 0, 255], "纯红像素通道序错误（BGRA 未换位？）");
                assert_eq!(px(4, 2), [0, 255, 0, 255], "纯绿像素通道序错误");
                assert_eq!(px(6, 2), [0, 0, 255, 255], "纯蓝像素通道序错误");
                // 行序：顶行区的色应在 y=2；若行序翻转，(2,2) 处会是空像素/黑
                assert_eq!(px(2, 5), [255, 255, 255, 255], "末行白像素错位（行序翻转？）");
                assert_eq!(px(4, 5), [0, 0, 0, 255], "末行黑像素错位（行序翻转？）");
            }
        }
    }
}

#[cfg(windows)]
pub use gdi::GdiBackend;

// =========================================================================
// 非 Windows 降级实现
// =========================================================================

/// 非 Windows 平台的占位后端：工具 schema 一致，执行时返回「仅支持 Windows」。
///
/// 工具列表跨平台稳定（模型/前端不用条件判断），失败信息自文档。
#[cfg(not(windows))]
pub struct UnsupportedBackend;

#[cfg(not(windows))]
impl ScreenBackend for UnsupportedBackend {
    fn name(&self) -> &'static str {
        "unsupported"
    }

    fn virtual_screen(&self) -> AppResult<VirtualScreenLayout> {
        Err(unsupported())
    }

    fn monitors(&self) -> AppResult<Vec<PhysRect>> {
        Err(unsupported())
    }

    fn capture(&self, _region: PhysRect) -> AppResult<RgbaFrame> {
        Err(unsupported())
    }

    fn windows(&self) -> AppResult<Vec<WindowInfo>> {
        Err(unsupported())
    }

    fn capture_window(&self, _hwnd: i64) -> AppResult<(RgbaFrame, PhysRect)> {
        Err(unsupported())
    }

    fn foreground_window(&self) -> Option<i64> {
        None
    }

    fn mouse_move_abs(&self, _abs_x: i32, _abs_y: i32) -> AppResult<()> {
        Err(unsupported())
    }

    fn mouse_button(&self, _button: MouseButton, _down: bool) -> AppResult<()> {
        Err(unsupported())
    }

    fn mouse_scroll(&self, _dx_notches: i32, _dy_notches: i32) -> AppResult<()> {
        Err(unsupported())
    }

    fn key_vk(&self, _vk: u16, _down: bool) -> AppResult<()> {
        Err(unsupported())
    }

    fn key_unicode(&self, _unit: u16, _down: bool) -> AppResult<()> {
        Err(unsupported())
    }
}

#[cfg(not(windows))]
fn unsupported() -> AppError {
    AppError::Validation(
        "screen 不支持: computer use 屏幕工具当前仅支持 Windows——\
         在 macOS/Linux 上请改用其它方式完成任务".into(),
    )
}
