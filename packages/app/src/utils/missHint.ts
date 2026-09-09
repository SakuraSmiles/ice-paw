// missHint — 缓存 miss 归因 slug → 前端文案（③ 上下文开销可观测化）。
//
// 分工（硬约束）：后端 turn_cost::attribute_miss 只判「输入侧谁变了」的
// 可观测事实；**机理说明（TTL 过期 / 前缀块对齐）只住这里**。措辞诚实：
// 归因是本地推断非 provider 报告——title 必须带披露行（勿删）。
// slug 词表与后端 turn_cost::miss_slug 镜像，改动两边同步。

/** slug → chip 短标签（常驻 BudgetPill） */
export const MISS_HINT_LABELS: Record<string, string> = {
  first_request: "首次请求",
  tools_changed: "工具列表变化",
  system_stable_changed: "系统提示变化",
  os_env_changed: "运行环境变化",
  injection_changed: "注入变化",
  model_switched: "模型换档",
  no_detectable_change: "疑缓存过期",
};

/** slug → 机理说明（hover title 用） */
const MISS_HINT_MECHANISMS: Record<string, string> = {
  first_request: "会话首个请求，此前无缓存可命中",
  tools_changed: "本轮发送的工具列表与上次不同（增删/描述变化/相关性裁剪），工具定义位于请求前部，其后前缀整体失效",
  system_stable_changed: "系统提示稳定段（人设/工具提示/委派清单/样式档案）与上回合不同，系统提示是缓存前缀的第一段",
  os_env_changed: "运行环境段变化（工作目录/时区/project.md 等），系统提示随之变化",
  injection_changed: "钩子或预算提醒的注入状态与上次翻转，前缀外追加了新内容",
  model_switched: "降级链换档后缓存命名空间切换，此前缓存不再复用",
  no_detectable_change: "输入侧与上次无可检测差异，疑为 provider 缓存 TTL 过期（缓存有时效，过期后按全价重算）",
};

/** chip 短标签：首个非 first_request 的归因（多因时取首个，完整清单在 title）。
 *  只含 first_request 时返回 null——首次请求无缓存可用是正常态，不做常驻提示
 *  （弱展示：title 里仍可见）。空数组 / 未知 slug 数组也返回 null（诚实缺席）。 */
export function shortMissHint(slugs: string[] | null | undefined): string | null {
  if (!slugs?.length) return null;
  const first = slugs.find((s) => s !== "first_request");
  return first ? (MISS_HINT_LABELS[first] ?? null) : null;
}

/** hover title：全因清单 + 机理 + 推断披露。三段式——发生了什么（本轮输入
 *  全未命中缓存）+ 为什么（各因机理）+ 怎么看（推断非报告）。 */
export function missHintTitle(slugs: string[] | null | undefined): string {
  if (!slugs?.length) return "";
  const lines = slugs.map((s) => {
    const label = MISS_HINT_LABELS[s] ?? s;
    const mech = MISS_HINT_MECHANISMS[s] ?? "";
    return mech ? `${label}：${mech}` : label;
  });
  return [`本轮输入全未命中缓存（prompt ≥ 1024 且命中为 0）`, ...lines, "以上为本地推断，非 provider 报告；命中折扣计费口径见预算说明"].join("\n");
}
