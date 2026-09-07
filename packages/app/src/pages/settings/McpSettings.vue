<script setup lang="ts">
// McpSettings.vue — MCP Server 设置（状态机驱动，单一数据源）
import { ref, computed, onMounted, onActivated, onDeactivated, onBeforeUnmount } from "vue";
import McpForm from "../../components/mcp/McpForm.vue";
import Switch from "../../components/common/Switch.vue";
import ErrorBanner from "../../components/common/ErrorBanner.vue";
import type { McpServer, McpServerSnapshot } from "../../types";
import { bridge } from "../../api/bridge";
import { GLM_MCP_TEMPLATES, type GlmMcpTemplate } from "../../data/glmMcpTemplates";
import { toolDisplayName } from "../../utils/toolLabels";

const servers = ref<McpServerSnapshot[]>([]);
const loading = ref(true);
const loadError = ref<string | null>(null);
const nodeAvailable = ref(true);
const lastLoadTime = ref(0);

async function reload() {
  loadError.value = null;
  loading.value = true;
  try {
    const [serverList, builtins] = await Promise.all([
      bridge.mcp.list(),
      bridge.mcp.listBuiltinTools(),
    ]);
    servers.value = serverList;
    builtinTools.value = builtins
      .map(t => {
        const meta = BUILTIN_TOOL_META[t.name];
        return {
          name: t.name,
          desc: meta?.zh ?? t.description,
          orig: t.description,
          category: meta?.category ?? "other",
        };
      })
      .sort((a, b) => a.name.localeCompare(b.name));
    lastLoadTime.value = Date.now();
  } catch (e) {
    console.error("加载 MCP Server 列表失败:", e);
    loadError.value = e instanceof Error ? e.message : String(e);
  } finally {
    loading.value = false;
  }
  bridge.mcp.checkNodejs().then(v => { nodeAvailable.value = v; }).catch(() => {});
}

// KeepAlive: 超过 30s 才刷新；有 server 在初始化中时自动轮询
let hasLoaded = false;
let pollTimer: ReturnType<typeof setInterval> | null = null;

onMounted(async () => {
  await reload();
  hasLoaded = true;
  startPollIfNeeded();
});
onActivated(() => {
  if (hasLoaded && Date.now() - lastLoadTime.value > 30_000) {
    reload().then(() => startPollIfNeeded());
  }
});
onDeactivated(() => {
  if (pollTimer) { clearInterval(pollTimer); pollTimer = null; }
});
onBeforeUnmount(() => {
  if (pollTimer) { clearInterval(pollTimer); pollTimer = null; }
});

function startPollIfNeeded() {
  if (pollTimer) clearInterval(pollTimer);
  if (servers.value.some(s => s.status === "starting")) {
    pollTimer = setInterval(async () => {
      await reload();
      if (!servers.value.some(s => s.status === "starting")) {
        if (pollTimer) { clearInterval(pollTimer); pollTimer = null; }
      }
    }, 2000);
  }
}

const expandedEditId = ref<string | null>(null);
const isCreating = ref(false);

function toggleEdit(s: McpServerSnapshot) {
  isCreating.value = false;
  expandedEditId.value = expandedEditId.value === s.id ? null : s.id;
}

function toggleNew() {
  expandedEditId.value = null;
  isCreating.value = !isCreating.value;
}

function statusLabel(s: McpServerSnapshot): string {
  if (!s.enabled) return "已停用";
  return { running: "运行中", starting: "初始化中…", failed: "未就绪", disabled: "已停用" }[s.status];
}

function statusClass(s: McpServerSnapshot): string {
  if (!s.enabled) return "stopped";
  return s.status; // running / starting / failed
}

// 切换/重试失败状态（UI-2：条目级 inline，错误贴身对应卡片）
const toggleError = ref<{ id: string; msg: string; retry: () => void } | null>(null);
// 删除失败（UI-2 批次二 1/3）：与切换失败同卡片同位置，独立状态位防互相覆盖
const deleteError = ref<{ id: string; msg: string; retry: () => void } | null>(null);

async function toggleEnabled(s: McpServerSnapshot, enabled: boolean) {
  try {
    await bridge.mcp.setEnabled(s.id, enabled);
    toggleError.value = null;
    await reload();
  } catch (e) {
    console.error("切换启用状态失败:", e);
    toggleError.value = { id: s.id, msg: e instanceof Error ? e.message : String(e), retry: () => void toggleEnabled(s, enabled) };
    await reload();
  }
}

async function retryServer(s: McpServerSnapshot) {
  try {
    await bridge.mcp.retry(s.id);
    await reload();
  } catch (e) {
    console.error("重试失败:", e);
    await reload();
  }
}

function onSaved(_s: McpServer) { isCreating.value = false; expandedEditId.value = null; reload(); }
function onCancel() { isCreating.value = false; expandedEditId.value = null; }
async function onDelete(s: McpServer) {
  try {
    await bridge.mcp.remove(s.id);
    deleteError.value = null;
    isCreating.value = false; expandedEditId.value = null;
    await reload();
  } catch (e) {
    console.error("删除 MCP Server 失败:", e);
    deleteError.value = { id: s.id, msg: e instanceof Error ? e.message : String(e), retry: () => void onDelete(s) };
  }
}

// ---- GLM 模板：从 GLM Coding Plan 的 MCP 服务一键添加（仅前端组装，复用 bridge.mcp.create）----
const showGlm = ref(false);
const selectedGlmKey = ref<string | null>(null);
const glmApiKey = ref("");
const addingGlm = ref(false);
const glmError = ref("");

function selectGlm(key: string) {
  selectedGlmKey.value = key;
  glmApiKey.value = "";
  glmError.value = "";
}

async function confirmAddGlm(t: GlmMcpTemplate) {
  if (addingGlm.value || !glmApiKey.value.trim()) return;
  addingGlm.value = true;
  glmError.value = "";
  try {
    const input = t.build(glmApiKey.value.trim());
    // 用模板 key 作稳定 id：据此识别「已配置」并复用运行状态；重复添加由后端 id 唯一约束拦截
    await bridge.mcp.create({ id: t.key, ...input });
    selectedGlmKey.value = null;
    glmApiKey.value = "";
    // 不折叠卡片：让用户看到该行从「添加」变为运行状态，明确反馈成功与否
    await reload();
  } catch (e) {
    glmError.value = e instanceof Error ? e.message : "添加失败";
    console.error("添加 GLM 模板失败:", e);
  } finally {
    addingGlm.value = false;
  }
}

/** 模板是否已配置（id === t.key）：已配置返回状态文案，否则空串（兼作 v-if 真值判断） */
function glmTag(t: GlmMcpTemplate): string {
  const s = servers.value.find(x => x.id === t.key);
  return s ? statusLabel(s) : "";
}
/** 已配置模板的状态 class（运行中 / 初始化中 / 未就绪 / 已停用） */
function glmStatusCls(t: GlmMcpTemplate): string {
  const s = servers.value.find(x => x.id === t.key);
  return s ? statusClass(s) : "";
}

// 内置工具集——动态从后端拉取（register_builtin 为单一事实来源，前端不再手抄）
const builtinExpanded = ref(false);
const builtinSearch = ref("");
const builtinTools = ref<BuiltinToolRow[]>([]);

interface BuiltinToolRow {
  name: string;
  /** 展示描述：中文覆盖优先，缺失回退后端原文 */
  desc: string;
  /** 后端原始（英文）描述——搜索匹配 + 悬浮全文用 */
  orig: string;
  category: string;
}

// 工具分组顺序（「其他」兜底在末位：新增工具忘了补 meta 也能显示，不会漏）
const BUILTIN_CATEGORIES: { key: string; label: string }[] = [
  { key: "files", label: "文件与命令" },
  { key: "web", label: "网络获取" },
  { key: "kb", label: "知识库" },
  { key: "attach", label: "附件与引用" },
  { key: "docx", label: "Word 文档" },
  { key: "config", label: "配置与计划" },
  { key: "screen", label: "屏幕操作" },
  { key: "other", label: "其他" },
];

// 中文友好描述 + 分组（本地化文案层）：仅用于设置页展示，缺失时回退后端原始描述
// 并落「其他」组。工具清单与计数始终来自后端，这里只决定某工具显示中文短描述
// 还是后端原文——新增工具忘了补这里，工具照样显示（只是英文原文 + 其他组）。
const BUILTIN_TOOL_META: Record<string, { zh: string; category: string }> = {
  // 文件与命令
  read_file: { zh: "读取文件内容（office/PDF 提取，大文件分页）", category: "files" },
  list_directory: { zh: "列出目录内容（非递归，目录优先排序）", category: "files" },
  directory_tree: { zh: "递归目录树（跳噪音目录，限深 8）", category: "files" },
  get_file_info: { zh: "文件元信息（大小/类型/时间戳）", category: "files" },
  read_multiple_files: { zh: "批量读多个文件（≤20）", category: "files" },
  write_file: { zh: "写入文件（覆盖，改前自动备份）", category: "files" },
  edit_file: { zh: "精准字符串替换（先读后改）", category: "files" },
  delete_file: { zh: "删除文件或空目录（文件先备份）", category: "files" },
  move_file: { zh: "移动 / 重命名（跨盘自动复制）", category: "files" },
  copy_file: { zh: "复制文件或目录（源保留）", category: "files" },
  create_directory: { zh: "建目录含父目录（幂等）", category: "files" },
  search_files: { zh: "正则内容搜索（grep 语义）", category: "files" },
  run_command: { zh: "执行 shell 命令（需确认授权）", category: "files" },
  git: { zh: "git 只读操作（status/diff/log/show）", category: "files" },
  // 网络获取
  web_fetch: { zh: "抓取 URL 正文（原文返回，自看状态码）", category: "web" },
  // 知识库
  search_kb: { zh: "检索知识库（关键词+语义融合）", category: "kb" },
  read_kb_document: { zh: "读取知识库文档全文", category: "kb" },
  save_to_kb: { zh: "保存资料到知识库（Markdown）", category: "kb" },
  // 附件与引用
  read_attachment_page: { zh: "按页读取聊天大附件", category: "attach" },
  view_attachment_image: { zh: "视觉读取图片型附件（扫描件等）", category: "attach" },
  read_reference: { zh: "读取 @引用 会话的快照全文", category: "attach" },
  // Word 文档
  inspect_docx: { zh: "读 Word 结构投影（五档下钻）", category: "docx" },
  edit_docx: { zh: "编辑 Word（批量事务 + 自动备份）", category: "docx" },
  validate_docx: { zh: "Word 断言验收（失败是数据非错误）", category: "docx" },
  write_docx: { zh: "从模板生成整篇 Word 文档", category: "docx" },
  // 配置与计划
  read_agent_config: { zh: "读取自己的 agent.yaml 配置", category: "config" },
  propose_config_change: { zh: "提出 agent 配置变更提案（待审批）", category: "config" },
  update_plan: { zh: "维护任务计划（待办清单）", category: "config" },
  // 屏幕操作
  capture_screen: { zh: "截取整个屏幕画面", category: "screen" },
  list_windows: { zh: "列出当前打开的窗口", category: "screen" },
  capture_window: { zh: "截取指定窗口画面", category: "screen" },
  mouse_move: { zh: "移动鼠标指针（需屏幕共享）", category: "screen" },
  mouse_click: { zh: "点击鼠标（需屏幕共享）", category: "screen" },
  mouse_drag: { zh: "拖拽鼠标（需屏幕共享）", category: "screen" },
  mouse_scroll: { zh: "滚动鼠标滚轮（需屏幕共享）", category: "screen" },
  type_text: { zh: "输入文字（需屏幕共享）", category: "screen" },
  press_key: { zh: "按键 / 组合键（需屏幕共享）", category: "screen" },
  wait: { zh: "等待指定秒数（屏幕操作节奏）", category: "screen" },
  request_screen_session: { zh: "请求开启屏幕共享会话", category: "screen" },
};

// 搜索 + 分组视图：按工具名 / 中文描述 / 后端原文过滤；空分组隐藏（无搜索词 = 全部分组）
const filteredBuiltinGroups = computed(() => {
  const q = builtinSearch.value.trim().toLowerCase();
  const matched = builtinTools.value.filter(
    t => !q
      || t.name.toLowerCase().includes(q)
      || t.desc.toLowerCase().includes(q)
      || t.orig.toLowerCase().includes(q),
  );
  const byCat = new Map<string, BuiltinToolRow[]>();
  for (const t of matched) {
    const list = byCat.get(t.category) ?? [];
    list.push(t);
    byCat.set(t.category, list);
  }
  return BUILTIN_CATEGORIES
    .map(c => ({ ...c, tools: byCat.get(c.key) ?? [] }))
    .filter(g => g.tools.length > 0);
});
</script>

<template>
  <div class="settings-content-inner">
    <div class="content-header">
      <h2 class="content-title">工具集</h2>
    </div>

    <div v-if="!nodeAvailable && !loading" class="node-warning">
      <span class="node-warn-icon">!</span>
      <span>未检测到系统 Node.js。深度推理 / 知识图谱记忆已内置运行时，不受影响；浏览器自动化等用户自加的 npx 类型 MCP Server 仍需 <a href="https://nodejs.org" target="_blank">安装 Node.js</a>（LTS）后重启生效。</span>
    </div>

    <div class="mcp-list">
      <!-- 新建 -->
      <div class="mcp-card new-card" :class="{ expanded: isCreating }" @click="toggleNew">
        <div class="card-top">
          <div class="card-body">
            <div class="card-name-row">
              <svg class="new-plus" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <line x1="12" y1="5" x2="12" y2="19" /><line x1="5" y1="12" x2="19" y2="12" />
              </svg>
              <span class="card-name new-name">新建 MCP Server</span>
            </div>
            <div class="new-hint">接入一个外部工具服务…</div>
          </div>
          <svg class="card-chevron" :class="{ rotated: isCreating }" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="9 18 15 12 9 6" />
          </svg>
        </div>
        <div v-if="isCreating" class="expand-panel" @click.stop>
          <McpForm :server="null" @saved="onSaved" @cancel="onCancel" @delete="onDelete" />
        </div>
      </div>

      <!-- GLM 模板：与「新建」同款虚线入口卡，区别仅在图标与标题 -->
      <div class="mcp-card glm-card" :class="{ expanded: showGlm }" @click="showGlm = !showGlm">
        <div class="card-top">
          <div class="card-body">
            <div class="card-name-row">
              <svg class="tpl-icon" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <polygon points="12 2 2 7 12 12 22 7 12 2" />
                <polyline points="2 17 12 22 22 17" />
                <polyline points="2 12 12 17 22 12" />
              </svg>
              <span class="card-name new-name">从 GLM 模板添加</span>
            </div>
            <div class="new-hint">联网搜索 · 网页读取 · 开源仓库</div>
          </div>
          <svg class="card-chevron" :class="{ rotated: showGlm }" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="9 18 15 12 9 6" /></svg>
        </div>
        <div v-if="showGlm" class="expand-panel" @click.stop>
          <div v-if="glmError" class="glm-error">{{ glmError }}</div>
          <div class="glm-templates">
            <div v-for="t in GLM_MCP_TEMPLATES" :key="t.key" class="glm-template">
              <!-- 正在输入 Key -->
              <div v-if="selectedGlmKey === t.key" class="glm-key-row">
                <input v-model="glmApiKey" type="password" class="input glm-key-input input-mono" placeholder="GLM API Key" />
                <button class="btn btn-sm btn-primary" :disabled="addingGlm || !glmApiKey.trim()" @click="confirmAddGlm(t)">{{ addingGlm ? "添加中" : "确认" }}</button>
                <button class="btn-link" @click="selectedGlmKey = null">取消</button>
              </div>
              <!-- 已配置：显示运行状态（与下方 server 卡同款状态点 / 标签），不再显示「添加」 -->
              <template v-else-if="glmTag(t)">
                <div class="glm-template-main">
                  <span class="glm-template-name">{{ t.name }}</span>
                  <span v-if="['starting', 'failed'].includes(glmStatusCls(t))" class="status-dot" :class="'dot-' + glmStatusCls(t)" :title="glmStatusCls(t) === 'failed' ? '启动失败' : '启动中'" />
                  <span class="status-tag" :class="'tag-' + glmStatusCls(t)">{{ glmTag(t) }}</span>
                </div>
                <div class="glm-template-sub">
                  <span class="glm-template-kind">{{ t.badge }}</span>
                  <span class="glm-template-desc">{{ t.description }}</span>
                </div>
              </template>
              <!-- 未配置 -->
              <template v-else>
                <div class="glm-template-main">
                  <span class="glm-template-name">{{ t.name }}</span>
                  <button class="btn-link" @click="selectGlm(t.key)">添加</button>
                </div>
                <div class="glm-template-sub">
                  <span class="glm-template-kind">{{ t.badge }}</span>
                  <span class="glm-template-desc">{{ t.description }}</span>
                </div>
              </template>
            </div>
          </div>
        </div>
      </div>

      <div class="list-divider"></div>

      <!-- 内置工具 -->
      <div class="mcp-card builtin-card" :class="{ expanded: builtinExpanded }" @click="builtinExpanded = !builtinExpanded">
        <div class="card-top">
          <div class="card-body">
            <div class="card-name-row">
              <span class="card-name">内置工具</span>
              <span class="builtin-badge">系统</span>
            </div>
          </div>
          <span class="builtin-count">{{ builtinTools.length }} 个 · 始终可用</span>
          <svg class="card-chevron" :class="{ rotated: builtinExpanded }" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="9 18 15 12 9 6" /></svg>
        </div>
        <div v-if="builtinExpanded" class="expand-panel" @click.stop>
          <div class="builtin-tools">
            <input
              v-model="builtinSearch"
              type="text"
              class="input builtin-search"
              placeholder="搜索工具名或描述…"
            />
            <template v-for="group in filteredBuiltinGroups" :key="group.key">
              <div class="builtin-group-head">
                <span class="builtin-group-title">{{ group.label }}</span>
                <span class="builtin-group-count">{{ group.tools.length }}</span>
              </div>
              <div v-for="tool in group.tools" :key="tool.name" class="builtin-tool">
                <span class="builtin-tool-name">{{ toolDisplayName(tool.name) }}</span>
                <span class="builtin-tool-id" :title="tool.name">{{ tool.name }}</span>
                <span class="builtin-tool-desc" :title="tool.orig">{{ tool.desc }}</span>
              </div>
            </template>
            <div v-if="!filteredBuiltinGroups.length" class="builtin-empty">没有匹配的工具</div>
          </div>
        </div>
      </div>

      <!-- Server 列表 -->
      <div v-for="s in servers" :key="s.id" class="mcp-card" :class="{ expanded: expandedEditId === s.id }" @click="toggleEdit(s)">
        <div class="card-top">
          <span v-if="['starting', 'failed'].includes(statusClass(s))" class="status-dot" :class="'dot-' + statusClass(s)" :title="statusClass(s) === 'failed' ? '启动失败' : '启动中'" />
          <div class="card-body">
            <div class="card-name-row">
              <span class="card-name">{{ s.name }}</span>
              <span class="status-tag" :class="'tag-' + statusClass(s)">{{ statusLabel(s) }}</span>
              <span v-if="s.status === 'running' && s.tool_count" class="probe-tag probe-done">{{ s.tool_count }} 工具</span>
              <span v-else-if="s.status === 'failed'" class="probe-tag probe-error" :title="s.error ?? undefined">未就绪</span>
            </div>
            <ErrorBanner
              v-if="toggleError?.id === s.id"
              variant="inline"
              :title="s.enabled ? '启用失败' : '停用失败'"
              :detail="toggleError.msg"
              @retry="toggleError?.retry()"
            />
            <ErrorBanner
              v-else-if="deleteError?.id === s.id"
              variant="inline"
              title="删除失败"
              :detail="deleteError.msg"
              @retry="deleteError?.retry()"
            />
          </div>
          <Switch :model-value="s.enabled" @update:model-value="(v: boolean) => toggleEnabled(s, v)" @click.stop />
          <svg class="card-chevron" :class="{ rotated: expandedEditId === s.id }" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="9 18 15 12 9 6" />
          </svg>
        </div>

        <!-- 展开态 -->
        <div v-if="expandedEditId === s.id" class="expand-panel" @click.stop>
          <McpForm :server="s as any" @saved="onSaved" @cancel="onCancel" @delete="onDelete" />

          <div class="region">
            <div v-if="s.status === 'failed'" class="probe-error-bar">
              <span class="probe-error-msg">{{ s.error || '启动失败' }}</span>
              <button class="btn-link probe-retry-btn" @click="retryServer(s)">重试</button>
            </div>
            <div class="region-head">
              <span class="region-title">工具清单</span>
              <button v-if="s.status === 'failed'" class="btn-link" @click="retryServer(s)">
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <polyline points="23 4 23 10 17 10" /><polyline points="1 20 1 14 7 14" />
                  <path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15" />
                </svg>
                重试
              </button>
            </div>
            <div v-if="s.status === 'starting'" class="region-hint">正在初始化工具包…</div>
            <div v-else-if="!s.enabled" class="region-hint">启用后可查看工具清单</div>
            <template v-else-if="s.tools">
              <div class="region-meta">共 {{ s.tools.length }} 个工具</div>
              <div v-if="s.tools.length" class="tool-chips">
                <span v-for="t in s.tools" :key="t.name" class="tool-chip" :title="t.description">{{ t.name }}</span>
              </div>
            </template>
          </div>
        </div>
      </div>

      <div v-if="loading && !servers.length" class="loading-state">加载中...</div>
      <div v-else-if="loadError && !servers.length" class="load-fail">
        <span class="load-fail-icon">!</span>
        <span class="load-fail-msg">MCP Server 列表加载失败</span>
        <span class="load-fail-why">{{ loadError }}</span>
        <button type="button" class="load-fail-retry" @click="reload">重试</button>
      </div>
      <div v-else-if="!loading && servers.length === 0" class="empty-hint">还没有 MCP Server，点上方「新建」接入</div>
    </div>
  </div>
</template>

<style scoped>
.settings-content-inner { flex: 1; display: flex; flex-direction: column; padding: 0; min-height: 0; }

.content-header {
  display: flex; align-items: center;
  padding: 20px 28px 0; flex-shrink: 0; height: 56px;
}
.content-title {
  font-size: var(--ip-text-h3-size); font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary); margin: 0;
}

/* ===== 列表 ===== */
.mcp-list {
  flex: 1; overflow-y: auto; padding: 8px 28px 24px;
  display: flex; flex-direction: column; gap: var(--ip-spacing-2); min-height: 0;
}

/* ===== 卡片 ===== */
.mcp-card {
  padding: 14px 16px;
  background-color: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-lg);
  cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.mcp-card:hover { border-color: var(--ip-primary-300); box-shadow: var(--ip-shadow-sm); }
.mcp-card.expanded { border-color: var(--ip-primary-400); box-shadow: var(--ip-shadow-sm); }

.new-card { border: 1px dashed var(--ip-color-border-default); background-color: transparent; }
.new-card:hover { border-color: var(--ip-primary-400); background-color: var(--ip-color-bg-tertiary); }
.new-card.expanded { border-style: solid; border-color: var(--ip-primary-400); background-color: var(--ip-color-bg-secondary); }
.new-plus { flex-shrink: 0; color: var(--ip-color-primary-tint-text); }

.builtin-badge {
  font-size: var(--ip-text-micro-size); font-weight: var(--ip-font-weight-medium); padding: 0 6px; line-height: 18px;
  color: var(--ip-color-primary-tint-text); background: var(--ip-color-primary-tint-bg); border-radius: var(--ip-radius-full);
}
.builtin-count { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); flex-shrink: 0; }
.builtin-tools { display: flex; flex-direction: column; gap: var(--ip-spacing-2); }
.builtin-search { height: 30px; margin-bottom: var(--ip-spacing-2); }
.builtin-group-head { display: flex; align-items: baseline; gap: var(--ip-spacing-2); margin-top: var(--ip-spacing-2); }
.builtin-group-title { font-size: var(--ip-text-micro-size); font-weight: var(--ip-font-weight-semibold); color: var(--ip-color-text-secondary); letter-spacing: 0.02em; }
.builtin-group-count { font-size: var(--ip-text-micro-size); color: var(--ip-color-text-disabled); }
.builtin-tool { display: flex; align-items: baseline; gap: var(--ip-spacing-2_5); }
/* 主名=中文展示名（toolLabels 单一真相源，与聊天气泡工具行同源）；原名降次位
   mono 灰——enabled_tools 等配置引用的是英文名，保留可见映射 */
.builtin-tool-name { flex-shrink: 0; min-width: 104px; font-size: var(--ip-text-body-sm-size); font-weight: var(--ip-font-weight-medium); color: var(--ip-color-text-primary); }
.builtin-tool-id { flex-shrink: 0; font-family: var(--ip-font-mono); font-size: var(--ip-text-micro-size); color: var(--ip-color-text-disabled); }
.builtin-tool-desc { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.builtin-empty { padding: var(--ip-spacing-2) 0; font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }

.list-divider { height: 1px; background-color: var(--ip-color-border-default); margin: 2px 4px; }

.card-top { display: flex; align-items: flex-start; gap: var(--ip-spacing-2_5); cursor: pointer; }

.status-dot { width: 8px; height: 8px; border-radius: 50%; flex-shrink: 0; margin-top: 6px; }
.dot-failed { background: var(--ip-danger-base); animation: dot-pulse 1.2s ease-in-out infinite; }
.dot-starting { background: var(--ip-primary-500); animation: dot-pulse 1.2s ease-in-out infinite; }
/* v2.0 状态点语义收敛：正常态（running/configured/stopped/disabled）不渲染点——
 * 正常无需标记，视觉只留给异常（starting 蓝脉冲 / failed 红脉冲）。右侧 tag 文案仍标注完整状态。 */
@keyframes dot-pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.4; } }

.card-body { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 3px; }
.card-name-row { display: flex; align-items: center; gap: var(--ip-spacing-2); flex-wrap: wrap; }
.card-name { font-size: var(--ip-text-body-sm-size); font-weight: var(--ip-font-weight-semibold); color: var(--ip-color-text-primary); }
.new-name { color: var(--ip-color-primary-tint-text); }
.new-hint { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); padding-left: 24px; }

.status-tag { font-size: var(--ip-text-micro-size); font-weight: var(--ip-font-weight-medium); padding: 0 6px; line-height: 18px; border-radius: var(--ip-radius-full); flex-shrink: 0; }
.tag-running { background: var(--ip-success-bg); color: var(--ip-success-text); }
.tag-stopped, .tag-disabled { background: var(--ip-color-bg-tertiary); color: var(--ip-color-text-tertiary); }
.tag-failed { background: var(--ip-danger-bg); color: var(--ip-danger-text); }
.tag-starting { background: var(--ip-color-bg-tertiary); color: var(--ip-color-text-tertiary); }

.probe-tag { font-size: var(--ip-text-micro-size); padding: 0 6px; line-height: 18px; border-radius: var(--ip-radius-full); flex-shrink: 0; font-weight: var(--ip-font-weight-medium); }
.probe-probing { background: var(--ip-color-bg-tertiary); color: var(--ip-color-text-tertiary); animation: probe-pulse 1.5s ease-in-out infinite; }
.probe-done { background: var(--ip-success-bg); color: var(--ip-success-text); }
.probe-error { background: var(--ip-danger-bg); color: var(--ip-danger-text); cursor: help; }
@keyframes probe-pulse { 0%,100% { opacity: 1; } 50% { opacity: 0.5; } }
.probe-error-bar { display: flex; align-items: center; gap: var(--ip-spacing-2); padding: 6px 10px; margin-bottom: 8px; background: var(--ip-danger-bg); border-radius: var(--ip-radius-sm); }
.probe-error-msg { font-size: var(--ip-text-caption-size); color: var(--ip-danger-text); flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.probe-retry-btn { flex-shrink: 0; }

.card-chevron { flex-shrink: 0; color: var(--ip-color-text-disabled); transition: transform var(--ip-duration-fast) var(--ip-ease-out); margin-top: 2px; }
.card-chevron.rotated { transform: rotate(90deg); color: var(--ip-primary-600); }

.expand-panel { margin-top: 12px; padding-top: 12px; border-top: 1px solid var(--ip-color-border-default); }

.region { margin-top: 18px; display: flex; flex-direction: column; gap: var(--ip-spacing-2); }
.region-head { display: flex; align-items: center; justify-content: space-between; }
.region-title { font-size: var(--ip-text-caption-size); font-weight: var(--ip-font-weight-semibold); color: var(--ip-color-text-tertiary); letter-spacing: 0.02em; }
.region-meta { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-secondary); }
.region-hint { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }

.tool-chips { display: flex; flex-wrap: wrap; gap: 6px; }
.tool-chip { font-size: var(--ip-text-micro-size); font-family: var(--ip-font-mono); padding: 2px 8px; line-height: 18px; color: var(--ip-color-primary-tint-text); background-color: var(--ip-color-bg-tertiary); border-radius: var(--ip-radius-full); }

.btn-link { display: inline-flex; align-items: center; gap: 4px; height: 26px; padding: 0 8px; font-size: var(--ip-text-caption-size); color: var(--ip-color-text-secondary); background: none; border: none; border-radius: var(--ip-radius-md); cursor: pointer; transition: all var(--ip-duration-fast) var(--ip-ease-out); }
.btn-link:hover { color: var(--ip-primary-600); background-color: var(--ip-color-bg-tertiary); }

.loading-state { padding: 20px; text-align: center; color: var(--ip-color-text-tertiary); font-size: var(--ip-text-body-sm-size); }
.empty-hint { padding: 16px 12px; text-align: center; font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }
.node-warning { display: flex; align-items: flex-start; gap: var(--ip-spacing-2); margin: 0 0 12px; padding: 10px 14px; background: #fffbeb; border: 1px solid #fde68a; border-radius: var(--ip-radius-md); font-size: var(--ip-text-body-sm-size); color: #92400e; }
.node-warning a { color: #d97706; font-weight: var(--ip-font-weight-medium); }
.node-warn-icon { display: flex; align-items: center; justify-content: center; width: 20px; height: 20px; border-radius: 50%; background: #f59e0b; color: #fff; font-size: 12px; font-weight: 700; flex-shrink: 0; }

/* ---- GLM 模板卡片：折叠态对齐 new-card（虚线入口），展开内复用 builtin-tools 列表风 ---- */
.glm-card { border: 1px dashed var(--ip-color-border-default); background-color: transparent; }
.glm-card:hover { border-color: var(--ip-primary-400); background-color: var(--ip-color-bg-tertiary); }
.glm-card.expanded { border-style: solid; border-color: var(--ip-primary-400); background-color: var(--ip-color-bg-secondary); }
.tpl-icon { flex-shrink: 0; color: var(--ip-color-primary-tint-text); }

.glm-error { padding: 6px 10px; margin-bottom: 10px; font-size: var(--ip-text-caption-size); color: var(--ip-danger-text); background-color: var(--ip-danger-bg); border: 1px solid var(--ip-danger-border); border-radius: var(--ip-radius-md); }

/* 模板列表：每个模板「名称 + 操作」主行 + 「类型 · 描述」次行，无边框无分隔线，靠 gap 分隔 */
.glm-templates { display: flex; flex-direction: column; gap: var(--ip-spacing-3); }
.glm-template { display: flex; flex-direction: column; gap: 2px; }
.glm-template-main { display: flex; align-items: center; gap: var(--ip-spacing-2); min-width: 0; }
.glm-template-main .btn-link { margin-left: auto; flex-shrink: 0; }
/* 已配置行的状态点 / 标签：清掉 server 卡的 margin-top，并推到右侧与「添加」按钮对齐 */
.glm-template-main .status-dot { margin-top: 0; margin-left: auto; }
.glm-template-name { font-size: var(--ip-text-body-sm-size); font-weight: var(--ip-font-weight-medium); color: var(--ip-color-text-primary); }
.glm-template-sub { display: flex; align-items: baseline; gap: 6px; min-width: 0; }
.glm-template-kind { flex-shrink: 0; font-size: var(--ip-text-caption-size); color: var(--ip-color-text-secondary); }
.glm-template-desc { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }
.glm-key-row { display: flex; align-items: center; gap: 6px; }
.glm-key-input { flex: 1; min-width: 0; height: 28px; }

/* GLM 卡用到的输入 / 按钮（McpForm 的 scoped 样式不外泄，这里补一份） */
.input { width: 100%; height: var(--ip-input-h-sm); padding: 0 10px; font-size: var(--ip-text-body-sm-size); color: var(--ip-color-text-primary); background-color: var(--ip-color-bg-tertiary); border: 1px solid var(--ip-color-border-default); border-radius: var(--ip-radius-md); outline: none; box-sizing: border-box; transition: all var(--ip-duration-fast) var(--ip-ease-out); }
.input:focus { border-color: var(--ip-color-border-focus); background-color: var(--ip-color-bg-input); box-shadow: var(--ip-shadow-focus); }
.input::placeholder { color: var(--ip-color-text-placeholder); }
.input-mono { font-family: var(--ip-font-mono); }
.btn { display: inline-flex; align-items: center; justify-content: center; gap: 6px; padding: 0 14px; font-size: var(--ip-text-body-sm-size); font-weight: var(--ip-font-weight-medium); border-radius: var(--ip-radius-md); cursor: pointer; white-space: nowrap; transition: all var(--ip-duration-fast) var(--ip-ease-out); }
.btn-sm { height: 28px; }
.btn-primary { color: white; background-color: var(--ip-primary-500); border: none; }
.btn-primary:hover { background-color: var(--ip-primary-600); }  /* 档位镜像语义：浅色 600 更深、深色 600 稍亮，方向都正确 */
.btn-primary:disabled { opacity: 0.6; cursor: not-allowed; }
</style>
