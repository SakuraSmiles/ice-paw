// systemNotify — 审批请求的系统级提醒（Windows Toast / macOS 通知中心）。
//
// 发送走 Rust 命令（harness/approval_toast）：Windows toast 带批准/拒绝按钮
//（点击直接应答，无需回应用）+ 点主体前置主窗；request_id 有值 = 工具授权，
// 无值 = 配置提案/dev 自检（纯提醒——提案批准需应用内看 diff，通知上不可能完成）。
//
// 触发条件（调用方 useChatEvents 已按恰一次簿记）：应用失焦/后台
//（document.hasFocus() === false）。前台聚焦时审批弹窗本身可见，再发系统通知
// 是重复打扰；审批带 120s 倒计时、错过即超时拒绝，后台时 OS 通知是唯一的
// 「及时拉回」通道（生产反馈 2026-09-03：切后台常错过）。
//
// 好默认（L1，不加设置开关）：任何失败静默留痕——通知是旁路锦上添花，
// 不阻塞审批主流程。
import { bridge } from "../api/bridge";

/** 无守卫直发（自检用）。 */
async function sendApprovalToast(title: string, body: string, requestId?: string): Promise<void> {
  await bridge.chat.notifyApproval({ title, body, request_id: requestId });
}

/** 审批类系统通知（工具授权 / 配置提案）。fire-and-forget，调用方勿 await。
 *  工具授权传 requestId（toast 带批准/拒绝按钮）；提案不传（纯提醒）。 */
export async function notifyApprovalNeeded(title: string, body: string, requestId?: string): Promise<void> {
  if (document.hasFocus()) return;
  try {
    await sendApprovalToast(title, body, requestId);
  } catch (e) {
    // 通知链路异常（命令缺失 / 系统层拦截）不影响审批弹窗，但要留痕——
    // 静默吞会让「没收到通知」无从排查（2026-09-03 dev 实测排查时是盲区）
    console.error("审批系统通知发送失败:", e);
  }
}

/** dev 通路自检：跳过焦点判定直发一条（生产构建自动消失）。
 *  与生产同一条链路（Rust 命令，非旧 JS plugin 通道）；带两枚按钮，
 *  dev 顺带看按钮样式（按钮回调在 dev 不保证——借 AUMID 时激活路由由
 *  Windows 决定，功能验证在装机版）。点按钮无害（Rust 侧 respond 幂等 warn）。 */
export async function notifySelfCheck(): Promise<void> {
  try {
    await sendApprovalToast(
      "IcePaw 通知自检",
      "看到这条 = 系统通知链路正常（仅 dev）",
      `self-check-${Date.now()}`,
    );
  } catch (e) {
    console.error("[通知自检] 发送失败:", e);
  }
}
