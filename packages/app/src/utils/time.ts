// utils/time.ts — 全局时间格式化（单一时区来源）
//
// 所有「绝对时间」显示（HH:MM、日期）统一走这里，时区取自用户偏好
// (preferences.timezone)，缺省回退本地。这样格式只此一处、不会再出现
// 「这边对、那边错」。
//
// 相对时间（X分钟前）本质是 instant 差值、与时区无关，仍由各组件用 OS 时钟
// (Date.now()) 算差值；只有它超过 30 天回退成日期时，才调这里的 formatDate。

import { ref } from "vue";
import { bridge } from "../api/bridge";

// 应用级单一时区状态：启动时 loadTimezone() 读一次；设置页改动用 setTimezone() 同步。
const timezone = ref<string>("");
let loaded = false;
let loadPromise: Promise<void> | null = null;

/** 加载配置时区（幂等，多处调用只读一次）。App 启动时调用。 */
export function loadTimezone(): Promise<void> {
  if (loaded) return Promise.resolve();
  if (!loadPromise) {
    loadPromise = bridge.preferences.get()
      .then((p) => { timezone.value = p.timezone || ""; })
      .catch(() => { /* 静默：回退本地 */ })
      .finally(() => { loaded = true; loadPromise = null; });
  }
  return loadPromise;
}

/** 设置页改时区后调用，让所有显示即时刷新。 */
export function setTimezone(tz: string): void {
  timezone.value = tz || "";
  loaded = true;
}

/** 当前生效时区：配置 > undefined（回退本地）。 */
function effectiveTz(): string | undefined {
  return timezone.value || undefined;
}

/** tz 下的「年-月-日」键，用于今天/昨天比较与跨日分组。 */
function dateKey(d: Date): string {
  const tz = effectiveTz();
  if (tz) {
    try {
      const parts = new Intl.DateTimeFormat("zh-CN", { timeZone: tz, year: "numeric", month: "numeric", day: "numeric" }).formatToParts(d);
      const get = (t: string) => parts.find((p) => p.type === t)?.value ?? "";
      return `${get("year")}-${get("month")}-${get("day")}`;
    } catch { /* 无效时区 → 兜底本地 */ }
  }
  return `${d.getFullYear()}-${d.getMonth() + 1}-${d.getDate()}`;
}

/** HH:MM（seconds=true 时为 HH:MM:SS）—— 消息时间、日志时间。 */
export function formatTime(iso: string, seconds = false): string {
  const d = new Date(iso);
  if (isNaN(d.getTime())) return "";
  const pad = (n: number) => String(n).padStart(2, "0");
  const tz = effectiveTz();
  if (tz) {
    try {
      const opts: Intl.DateTimeFormatOptions = {
        timeZone: tz, hour: "2-digit", minute: "2-digit", hour12: false,
        ...(seconds ? { second: "2-digit" } : {}),
      };
      return new Intl.DateTimeFormat("zh-CN", opts).format(d);
    } catch { /* 兜底本地 */ }
  }
  const base = `${pad(d.getHours())}:${pad(d.getMinutes())}`;
  return seconds ? `${base}:${pad(d.getSeconds())}` : base;
}

/** 日期分隔线标签：今天 / 昨天 / M月D日 —— 全部在同一时区下计算。 */
export function formatDateLabel(iso: string): string | null {
  const d = new Date(iso);
  if (isNaN(d.getTime())) return null;
  const now = new Date();
  const yesterday = new Date(now);
  yesterday.setDate(yesterday.getDate() - 1);
  const dk = dateKey(d);
  if (dk === dateKey(now)) return "今天";
  if (dk === dateKey(yesterday)) return "昨天";
  // 更早：tz 下的「M月D日」（不混用本地，避免和今天/昨天不在同一时区）
  const tz = effectiveTz();
  if (tz) {
    try {
      const parts = new Intl.DateTimeFormat("zh-CN", { timeZone: tz, month: "numeric", day: "numeric" }).formatToParts(d);
      const m = parts.find((p) => p.type === "month")?.value ?? String(d.getMonth() + 1);
      const day = parts.find((p) => p.type === "day")?.value ?? String(d.getDate());
      return `${m}月${day}日`;
    } catch { /* 兜底本地 */ }
  }
  return `${d.getMonth() + 1}月${d.getDate()}日`;
}

/** 紧凑日期 YYYY-M-D（同一时区）—— 侧栏 >30 天的会话回退用，不再截 UTC 字符串。 */
export function formatDate(iso: string): string {
  const d = new Date(iso);
  if (isNaN(d.getTime())) return "";
  return dateKey(d);
}
