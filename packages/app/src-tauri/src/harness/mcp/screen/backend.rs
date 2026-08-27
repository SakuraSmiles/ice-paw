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
//! DPI：Tauri v2 清单默认 PerMonitorV2 DPI 感知，`GetSystemMetrics` 与
//! BitBlt 坐标同为物理像素——捕获与 SendInput 共用同一坐标空间，无换算。

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

/// 屏幕捕获后端。
///
/// 阶段一（看屏）方法集：虚拟桌面布局 / 显示器枚举 / 区域捕获。
/// capture_window / list_windows（看屏2）与 send_input（操作阶段）后续在此扩展。
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
}

// =========================================================================
// GDI 实现（Windows）
// =========================================================================

#[cfg(windows)]
mod gdi {
    use super::*;

    use windows_sys::Win32::Foundation::{GetLastError, LPARAM, RECT};
    use windows_sys::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject,
        EnumDisplayMonitors, GetDC, GetDIBits, ReleaseDC, SelectObject, BITMAPINFO,
        BITMAPINFOHEADER, CAPTUREBLT, DIB_RGB_COLORS, SRCCOPY,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };

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
                // GDI 32bpp 是 BGRA —— 换位成 RGBA 供 image crate 直用。
                for px in buf.chunks_exact_mut(4) {
                    px.swap(0, 2);
                }
                Ok(RgbaFrame {
                    width: w as u32,
                    height: h as u32,
                    rgba: buf,
                })
            }
        }
    }

    /// GDI 错误家族化：`screen 捕获失败: <步骤>（GDI 错误码 N）——<怎么办>`。
    /// 错误首行 = 稳定家族前缀（doom_detect 冒号切分依赖），步骤词在前。
    fn capture_err(step: &str, hint: &str) -> AppError {
        // SAFETY: GetLastError 是线程槽查询，无副作用。
        let gle = unsafe { GetLastError() };
        AppError::Internal(format!("screen 捕获失败: {step}（GDI 错误码 {gle}）——{hint}"))
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
}

#[cfg(not(windows))]
fn unsupported() -> AppError {
    AppError::Validation(
        "screen 不支持: computer use 屏幕工具当前仅支持 Windows——\
         在 macOS/Linux 上请改用其它方式完成任务".into(),
    )
}
