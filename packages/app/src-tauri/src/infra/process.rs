//! 跨平台进程辅助。

/// Windows: 为子进程设置 `CREATE_NO_WINDOW` 创建标志，避免 GUI 应用 spawn 子进程时
/// 控制台窗口「一闪而过」。
///
/// IcePaw 是 `windows_subsystem = "windows"` 的应用（自身无控制台），Windows 默认会
/// 为每个子进程新建一个控制台——表现就是黑色 cmd 窗口闪现。设置此标志后子进程静默
/// 运行。
///
/// `std::process::Command` 通过 `CommandExt` trait 提供该能力，而 `tokio::process::Command`
/// 有自己的原生 `creation_flags` 方法——两者无共同 trait，故用本地 trait 统一入口。
/// 非 Windows 平台为空操作。

#[cfg(windows)]
pub(crate) trait NoWindowExt {
    fn no_window(&mut self);
}

#[cfg(windows)]
impl NoWindowExt for std::process::Command {
    fn no_window(&mut self) {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::os::windows::process::CommandExt::creation_flags(self, CREATE_NO_WINDOW);
    }
}

#[cfg(windows)]
impl NoWindowExt for tokio::process::Command {
    fn no_window(&mut self) {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        // tokio::process::Command 在 Windows 上的原生方法，内部转发给底层 std Command。
        self.creation_flags(CREATE_NO_WINDOW);
    }
}

/// 隐藏子进程的控制台窗口（Windows）；非 Windows 平台为空操作。
#[cfg(windows)]
pub(crate) fn suppress_console_window<C: NoWindowExt>(cmd: &mut C) {
    cmd.no_window();
}

#[cfg(not(windows))]
pub(crate) fn suppress_console_window<C>(_cmd: &mut C) {
    // 非 Windows 无控制台弹窗问题。
}
