// systemNotify — 审批请求的系统级提醒（Windows Toast / macOS 通知中心）。
//
// 触发条件：应用失焦/后台（document.hasFocus() === false）。前台聚焦时审批
// 弹窗本身可见，再发系统通知是重复打扰；审批带 120s 倒计时、错过即超时拒绝，
// 后台时 OS 通知是唯一的「及时拉回」通道（生产反馈 2026-09-03：切后台常错过）。
//
// 好默认（L1，不加设置开关）：
// - Windows 通知权限默认已授予，零弹窗零感知；
// - macOS 首次发送时才请求权限（系统授权框浮在最前，后台也可见），
//   不在应用启动时预弹（启动即讨权限是打扰）；
// - 任何失败静默——通知是旁路锦上添花，不阻塞审批主流程。
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";

/** 审批类系统通知（工具授权 / 配置提案）。fire-and-forget，调用方勿 await。 */
export async function notifyApprovalNeeded(title: string, body: string): Promise<void> {
  if (document.hasFocus()) return;
  try {
    let granted = await isPermissionGranted();
    if (!granted) {
      granted = (await requestPermission()) === "granted";
    }
    if (!granted) return;
    sendNotification({ title, body });
  } catch {
    // 通知链路异常（插件未就绪 / 权限 API 缺失）不进用户视野，也不影响审批弹窗
  }
}
