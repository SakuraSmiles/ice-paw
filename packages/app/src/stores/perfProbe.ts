/**
 * perfProbe — 主线程长任务探针（2026-11-04 线程饥饿根治批）。
 *
 * PerformanceObserver 'longtask'：>50ms 的主线程任务浏览器自动上报；本探针
 * 记录 ≥100ms 的任务（时长 + 归因 + 时间戳），环形保留 50 条，设置-日志页
 * 「主线程长任务」区呈现。常驻开启（观察者本身近零成本——只在长任务发生时
 * 回调）；WebView2 不支持 entryType 时静默不启用（watching=false）。
 *
 * 用途：装机即可取证「卡的主线程源」与验证修复前后对比——此前 0.9.8 遗留的
 * 「第二主线程源 Performance 面板取证」依赖用户手开 DevTools，从未执行过。
 */
import { defineStore } from "pinia";

export interface LongTaskRecord {
  id: number;
  /** 发生时刻（ISO） */
  at: string;
  durationMs: number;
  /** 归因（containerName/name；WebView2 常为空串——时长+时间戳仍可对照日志） */
  name: string;
}

const THRESHOLD_MS = 100;
const CAP = 50;

let observer: PerformanceObserver | null = null;
let nextId = 1;

export const usePerfProbeStore = defineStore("perfProbe", {
  state: () => ({
    records: [] as LongTaskRecord[],
    watching: false,
  }),
  actions: {
    /** main.ts 装配时调用一次；重复调用幂等。 */
    start() {
      if (this.watching) return;
      if (typeof PerformanceObserver === "undefined") return;
      try {
        observer = new PerformanceObserver((list) => {
          for (const e of list.getEntries()) {
            if (e.duration < THRESHOLD_MS) continue;
            const attr = (e as PerformanceEntry & { attribution?: Array<{ containerName?: string; name?: string }> })
              .attribution?.[0];
            this.add(Math.round(e.duration), attr?.containerName || attr?.name || "");
          }
        });
        observer.observe({ entryTypes: ["longtask"] });
        this.watching = true;
      } catch {
        // 该 WebView2 版本不支持 longtask entryType——探针静默不启用
        observer = null;
      }
    },
    add(durationMs: number, name: string) {
      this.records.unshift({ id: nextId++, at: new Date().toISOString(), durationMs, name });
      if (this.records.length > CAP) this.records.length = CAP;
    },
    clear() {
      this.records = [];
    },
  },
});
