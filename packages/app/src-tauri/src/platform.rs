//! 平台相关工具（当前仅 Windows 实现，其余平台空实现）。
//!
//! `primary_monitor_work_area`：窗口所在显示器（取不到时主屏）的**工作区**——
//! 扣除任务栏后的可用区域。Tauri v2 的 `Monitor` 只暴露全屏 `size()`/`position()`，
//! 没有工作区 API；首启动态默认窗口尺寸需要真实可用区（否则按 100% 高度算
//! 会把窗口塞进任务栏底下）。
//!
//! 返回的是**物理像素** RECT（与 Win32 一致）；调用方按需换算逻辑像素。

/// 窗口 HWND（Windows）；其他平台为单元。
#[cfg(windows)]
pub(crate) type Hwnd = *mut core::ffi::c_void;
#[cfg(not(windows))]
pub(crate) type Hwnd = ();

/// 物理像素矩形（与 Win32 RECT 同构；非 Windows 平台也保留该形状供调用方使用）。
#[derive(Debug, Clone, Copy)]
pub(crate) struct WorkRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// 窗口所在显示器工作区（物理像素）。非 Windows 平台返回 None（调用方跳过动态尺寸）。
#[cfg(windows)]
pub(crate) fn primary_monitor_work_area(hwnd: Hwnd) -> Option<WorkRect> {
    use windows_sys::Win32::Foundation::{POINT, RECT};
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;

    // 优先取窗口所在显示器（用户把窗口拖去副屏时首启也应按该屏计算）；
    // 窗口无有效 HWND 时退光标所在屏（再兜底 (0,0)=主屏，Windows 语义）。
    let hmon = unsafe {
        let direct = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        if direct.is_null() {
            let mut pt = POINT { x: 0, y: 0 };
            let _ = GetCursorPos(&mut pt);
            MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST)
        } else {
            direct
        }
    };
    if hmon.is_null() {
        return None;
    }

    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        rcMonitor: RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        },
        rcWork: RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        },
        dwFlags: 0,
    };
    // SAFETY：hmon 来自上面的 Win32 调用；info 是 cbSize 已初始化的出参缓冲区。
    if unsafe { GetMonitorInfoW(hmon, &mut info) } == 0 {
        return None;
    }
    Some(WorkRect {
        left: info.rcWork.left,
        top: info.rcWork.top,
        right: info.rcWork.right,
        bottom: info.rcWork.bottom,
    })
}

#[cfg(not(windows))]
pub(crate) fn primary_monitor_work_area(_hwnd: Hwnd) -> Option<WorkRect> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_area_shape_sane_or_none() {
        // Windows 真机：工作区宽高为正；非 Windows：恒 None。
        if let Some(r) = primary_monitor_work_area(std::ptr::null_mut()) {
            assert!(r.right > r.left && r.bottom > r.top);
        }
    }
}
