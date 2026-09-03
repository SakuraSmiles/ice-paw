//! 审批系统通知（toast 直操作）— Windows 带批准/拒绝按钮 + 点主体前置主窗。
//!
//! 链路：前端 useChatEvents（失焦判定 + 恰一次簿记，全部留前端）→
//! bridge.notify.approval → `notify_approval` 命令 → 本模块。
//!
//! Windows（tauri-winrt-notification）：
//! - AUMID 恒 `app.config().identifier`（com.icepaw.app）——NSIS 快捷方式已带
//!   AUMID，安装版 toast 以 IcePaw 身份发出；dev 机装过 IcePaw 时快捷方式也在，
//!   dev 同样显示真身（优于 plugin 的 dev 借 PowerShell AUMID 行为）。
//! - `request_id` 有值 = 工具授权通知：加「批准/拒绝」两按钮；无值（提案/自检）
//!   不加按钮——提案批准需应用内看 diff / 亲手填 key，通知上不可能完成。
//! - `on_activated` 是进程内事件，应用运行中即可收（审批请求来自应用内生成
//!   流程，那一刻应用必然在跑，无需 COM activator）。
//! - 点主体（action=None）→ 前置主窗；按钮 → 应答回灌既有 oneshot 通道。
//! - dev 限制：借 AUMID 时按钮激活路由由 Windows 决定，回调不保证回到 dev
//!   进程——按钮功能只能在装机版验证（dev 能看样式）。
//!
//! 非 Windows：tauri-plugin-notification builder 纯提醒（无按钮）。
//!
//! ⚠️ 不变式：
//! 1. 应答唯一入口 = `ToolAuthRegistry::respond`（与 `respond_tool_auth` 命令
//!    同路，不另开通道）；respond 幂等（未匹配 warn + Ok）是 120s 超时后点
//!    过期 toast 按钮的安全网（toast 无 hide API，超时后仍挂在通知中心）。
//! 2. 应答必 emit `chat:tool-auth-responded`——toast 按钮路径前端无乐观删，
//!    无此事件则 pendingAuthRequests 条目残留到 120s 超时。
//! 3. AUMID 恒 identifier，勿按 exe 路径分支（plugin 的 dev 借 PS AUMID 不延续）。

use tauri::{AppHandle, Emitter, Manager};

use crate::infra::protocol::{AuthScope, ToolAuthResponse, ToolAuthRespondedPayload};

/// 主窗 label（tauri.conf.json `app.windows[0].label`）
const MAIN_WINDOW: &str = "main";

/// 前置主窗：show + unminimize + set_focus。
///
/// 两处复用：single-instance 回调（点 toast / 双击 exe 拉起第二实例被拦时）+
/// toast 点主体。set_focus 在 Windows 有前台锁定策略（后台进程
/// SetForegroundWindow 可能被系统拒），先最简实现，装机版手测若观察不到
/// 前置再补 AllowSetForegroundWindow / flash（windows-sys 已在依赖）。
pub fn focus_main_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(MAIN_WINDOW) {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
}

/// 发审批系统通知。`request_id` 有值 = 工具授权（Windows 下带批准/拒绝按钮）。
pub fn show_approval_toast(app: &AppHandle, title: &str, body: &str, request_id: Option<&str>) {
    #[cfg(windows)]
    {
        show_toast_windows(app, title, body, request_id);
    }
    #[cfg(not(windows))]
    {
        use tauri_plugin_notification::NotificationExt;
        if let Err(e) = app.notification().builder().title(title).body(body).show() {
            tracing::warn!(target: "ice_paw.mgmt", "审批通知发送失败: {e}");
        }
    }
}

/// toast 批准的 scope 固定 `Once`（最保守档）：应用内卡片有 once/this_dir/
/// this_tool 三档选择权，通知上不猜用户意图（好默认，L1）。
#[cfg(windows)]
fn show_toast_windows(app: &AppHandle, title: &str, body: &str, request_id: Option<&str>) {
    use tauri_winrt_notification::Toast;

    // 模板行：title = 加粗标题行，text1 = 正文行（ToastText02 形态）
    let mut toast = Toast::new(&app.config().identifier).title(title).text1(body);
    if let Some(rid) = request_id {
        let app_handle = app.clone();
        let rid = rid.to_string();
        toast = toast
            .add_button("批准", "approve")
            .add_button("拒绝", "reject")
            .on_activated(move |action| {
                route_action(app_handle.clone(), rid.clone(), action);
                Ok(())
            });
    } else {
        // 无按钮（提案/自检）：点主体仍前置主窗——提案卡片在应用内
        let app_handle = app.clone();
        toast = toast.on_activated(move |action| {
            // 无按钮时 action 恒 None（点主体）；防御性兜底：任何值都前置
            let _ = action;
            focus_main_window(&app_handle);
            Ok(())
        });
    }
    if let Err(e) = toast.show() {
        tracing::warn!(target: "ice_paw.mgmt", "审批 toast 发送失败: {e}");
    }
}

/// toast 激活回调路由：None=点主体 → 前置主窗；approve/reject → 应答回灌。
#[cfg(windows)]
fn route_action(app: AppHandle, request_id: String, action: Option<String>) {
    match action.as_deref() {
        None => focus_main_window(&app),
        Some("approve") => spawn_respond(app, request_id, true),
        Some("reject") => spawn_respond(app, request_id, false),
        Some(other) => {
            tracing::warn!(target: "ice_paw.tool_auth", "审批 toast 未知 action: {other}")
        }
    }
}

/// `on_activated` 是 WinRT 同步线程回调，应答（async registry 锁 + emit）走
/// tauri runtime spawn，勿在回调线程里 await。
#[cfg(windows)]
fn spawn_respond(app: AppHandle, request_id: String, allowed: bool) {
    tauri::async_runtime::spawn(async move {
        let registry = app.state::<crate::harness::tool_executor::ToolAuthRegistry>();
        let handled = respond_tool_auth_and_emit(
            &app,
            registry.inner(),
            ToolAuthResponse {
                request_id,
                allowed,
                scope: AuthScope::Once,
            },
        )
        .await;
        tracing::info!(
            target: "ice_paw.tool_auth",
            "toast 按钮应答: allowed={allowed} handled={handled}"
        );
    });
}

/// 应答工具授权并广播 `chat:tool-auth-responded`（单一入口，勿复制两份）。
///
/// toast 按钮路径与 `respond_tool_auth` 命令共用：前端 invoke 路径已有乐观删，
/// 事件幂等双保险；toast 路径全靠此事件删 pendingAuthRequests 条目。
/// 未匹配 request_id（已超时/取消）→ warn + 返回 false，调用方无害。
pub async fn respond_tool_auth_and_emit(
    app: &AppHandle,
    registry: &crate::harness::tool_executor::ToolAuthRegistry,
    response: ToolAuthResponse,
) -> bool {
    let payload = ToolAuthRespondedPayload {
        request_id: response.request_id.clone(),
        allowed: response.allowed,
    };
    let handled = registry.respond(response).await;
    if handled {
        if let Err(e) = app.emit("chat:tool-auth-responded", payload) {
            tracing::warn!(target: "ice_paw.tool_auth", "tool-auth-responded 事件发送失败: {e}");
        }
    } else {
        tracing::warn!(
            target: "ice_paw.tool_auth",
            "授权 respond：未找到匹配的 request_id（可能已超时/取消）"
        );
    }
    handled
}
