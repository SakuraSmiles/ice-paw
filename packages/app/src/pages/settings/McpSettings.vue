<script setup lang="ts">
// McpSettings.vue — MCP Server 设置（状态机驱动，单一数据源）
import { ref, onMounted, onActivated, onDeactivated, onBeforeUnmount } from "vue";
import McpForm from "../../components/mcp/McpForm.vue";
import Switch from "../../components/common/Switch.vue";
import type { McpServer, McpServerSnapshot } from "../../types";
import { bridge } from "../../api/bridge";

const servers = ref<McpServerSnapshot[]>([]);
const loading = ref(true);
const nodeAvailable = ref(true);
const lastLoadTime = ref(0);

async function reload() {
  loading.value = true;
  try {
    const [serverList, builtins] = await Promise.all([
      bridge.mcp.list(),
      bridge.mcp.listBuiltinTools(),
    ]);
    servers.value = serverList;
    builtinTools.value = builtins.map(t => ({
      name: t.name,
      desc: builtinDescZh[t.name] ?? t.description,
    }));
    lastLoadTime.value = Date.now();
  } catch (e) {
    console.error("加载 MCP Server 列表失败:", e);
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

async function toggleEnabled(s: McpServerSnapshot, enabled: boolean) {
  try {
    await bridge.mcp.setEnabled(s.id, enabled);
    await reload();
  } catch (e) {
    console.error("切换启用状态失败:", e);
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
  try { await bridge.mcp.remove(s.id); isCreating.value = false; expandedEditId.value = null; await reload(); }
  catch (e) { console.error("删除 MCP Server 失败:", e); }
}

// 内置工具集——动态从后端拉取（register_builtin 为单一事实来源，前端不再手抄）
const builtinExpanded = ref(false);
const builtinTools = ref<{ name: string; desc: string }[]>([]);

// 中文友好描述（本地化文案层）：仅用于卡片展示优化，缺失时回退后端原始描述。
// 工具清单与计数始终来自后端，这里只决定某工具显示中文短描述还是后端原文；
// 新增工具忘了补这里，工具照样显示（只是描述用后端原文），不会出现数量错 / 漏工具。
const builtinDescZh: Record<string, string> = {
  read_file: "读取本地文件内容",
  list_directory: "列出目录内容",
  directory_tree: "递归目录树（跳噪音目录）",
  get_file_info: "文件元信息（大小/类型/时间）",
  read_multiple_files: "批量读多个文件（≤20）",
  write_file: "写入文件（覆盖）",
  edit_file: "精准字符串替换",
  delete_file: "删除文件或空目录",
  move_file: "移动 / 重命名（跨盘自动复制）",
  create_directory: "建目录含父目录（幂等）",
  search_files: "正则内容搜索（grep）",
  run_command: "执行 shell 命令（需授权）",
  git: "git 只读操作（status/diff/log/show）",
  web_fetch: "抓取 URL 正文",
  read_agent_config: "读取自己的 agent.yaml 配置",
  search_kb: "检索知识库（agent 自动调用）",
  read_kb_document: "读取知识库文档全文",
  save_to_kb: "保存资料到知识库（agent 自动调用）",
  propose_config_change: "提出 agent 配置提案（agent 自动调用）",
};
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

      <div class="list-divider"></div>

      <!-- 内置工具 -->
      <div class="mcp-card builtin-card" :class="{ expanded: builtinExpanded }" @click="builtinExpanded = !builtinExpanded">
        <div class="card-top">
          <span class="status-dot dot-running" />
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
            <div v-for="tool in builtinTools" :key="tool.name" class="builtin-tool">
              <span class="builtin-tool-name">{{ tool.name }}</span>
              <span class="builtin-tool-desc">{{ tool.desc }}</span>
            </div>
          </div>
        </div>
      </div>

      <!-- Server 列表 -->
      <div v-for="s in servers" :key="s.id" class="mcp-card" :class="{ expanded: expandedEditId === s.id }" @click="toggleEdit(s)">
        <div class="card-top">
          <span class="status-dot" :class="'dot-' + statusClass(s)" />
          <div class="card-body">
            <div class="card-name-row">
              <span class="card-name">{{ s.name }}</span>
              <span class="status-tag" :class="'tag-' + statusClass(s)">{{ statusLabel(s) }}</span>
              <span v-if="s.status === 'running' && s.tool_count" class="probe-tag probe-done">{{ s.tool_count }} 工具</span>
              <span v-else-if="s.status === 'failed'" class="probe-tag probe-error" :title="s.error ?? undefined">未就绪</span>
            </div>
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
  display: flex; flex-direction: column; gap: 8px; min-height: 0;
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
  font-size: 10px; font-weight: var(--ip-font-weight-medium); padding: 0 6px; line-height: 18px;
  color: var(--ip-color-primary-tint-text); background: var(--ip-color-primary-tint-bg); border-radius: var(--ip-radius-full);
}
.builtin-count { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); flex-shrink: 0; }
.builtin-tools { display: flex; flex-direction: column; gap: 10px; }
.builtin-tool { display: flex; align-items: baseline; gap: 12px; }
.builtin-tool-name { flex-shrink: 0; min-width: 120px; font-family: var(--ip-font-mono); font-size: var(--ip-text-caption-size); color: var(--ip-color-text-primary); }
.builtin-tool-desc { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }

.list-divider { height: 1px; background-color: var(--ip-color-border-default); margin: 2px 4px; }

.card-top { display: flex; align-items: flex-start; gap: 10px; cursor: pointer; }

.status-dot { width: 8px; height: 8px; border-radius: 50%; flex-shrink: 0; margin-top: 6px; }
.dot-running { background: var(--ip-success-base); }
.dot-stopped, .dot-disabled { background: var(--ip-color-border-default); }
.dot-failed { background: var(--ip-danger-base); animation: dot-pulse 1.2s ease-in-out infinite; }
.dot-starting { background: var(--ip-primary-500); animation: dot-pulse 1.2s ease-in-out infinite; }
@keyframes dot-pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.4; } }

.card-body { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 3px; }
.card-name-row { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
.card-name { font-size: var(--ip-text-body-sm-size); font-weight: var(--ip-font-weight-semibold); color: var(--ip-color-text-primary); }
.new-name { color: var(--ip-color-primary-tint-text); }
.new-hint { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); padding-left: 24px; }

.status-tag { font-size: 10px; font-weight: var(--ip-font-weight-medium); padding: 0 6px; line-height: 18px; border-radius: var(--ip-radius-full); flex-shrink: 0; }
.tag-running { background: var(--ip-success-bg); color: var(--ip-success-text); }
.tag-stopped, .tag-disabled { background: var(--ip-color-bg-tertiary); color: var(--ip-color-text-tertiary); }
.tag-failed { background: var(--ip-danger-bg); color: var(--ip-danger-text); }
.tag-starting { background: var(--ip-color-bg-tertiary); color: var(--ip-color-text-tertiary); }

.probe-tag { font-size: 10px; padding: 0 6px; line-height: 18px; border-radius: var(--ip-radius-full); flex-shrink: 0; font-weight: var(--ip-font-weight-medium); }
.probe-probing { background: var(--ip-color-bg-tertiary); color: var(--ip-color-text-tertiary); animation: probe-pulse 1.5s ease-in-out infinite; }
.probe-done { background: var(--ip-success-bg); color: var(--ip-success-text); }
.probe-error { background: var(--ip-danger-bg); color: var(--ip-danger-text); cursor: help; }
@keyframes probe-pulse { 0%,100% { opacity: 1; } 50% { opacity: 0.5; } }
.probe-error-bar { display: flex; align-items: center; gap: 8px; padding: 6px 10px; margin-bottom: 8px; background: var(--ip-danger-bg); border-radius: var(--ip-radius-sm); }
.probe-error-msg { font-size: var(--ip-text-caption-size); color: var(--ip-danger-text); flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.probe-retry-btn { flex-shrink: 0; }

.card-chevron { flex-shrink: 0; color: var(--ip-color-text-disabled); transition: transform var(--ip-duration-fast) var(--ip-ease-out); margin-top: 2px; }
.card-chevron.rotated { transform: rotate(90deg); color: var(--ip-primary-600); }

.expand-panel { margin-top: 12px; padding-top: 12px; border-top: 1px solid var(--ip-color-border-default); }

.region { margin-top: 18px; display: flex; flex-direction: column; gap: 8px; }
.region-head { display: flex; align-items: center; justify-content: space-between; }
.region-title { font-size: var(--ip-text-caption-size); font-weight: var(--ip-font-weight-semibold); color: var(--ip-color-text-tertiary); letter-spacing: 0.02em; }
.region-meta { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-secondary); }
.region-hint { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }

.tool-chips { display: flex; flex-wrap: wrap; gap: 6px; }
.tool-chip { font-size: 11px; font-family: var(--ip-font-mono); padding: 2px 8px; line-height: 18px; color: var(--ip-color-primary-tint-text); background-color: var(--ip-color-bg-tertiary); border-radius: var(--ip-radius-full); }

.btn-link { display: inline-flex; align-items: center; gap: 4px; height: 26px; padding: 0 8px; font-size: var(--ip-text-caption-size); color: var(--ip-color-text-secondary); background: none; border: none; border-radius: var(--ip-radius-md); cursor: pointer; transition: all var(--ip-duration-fast) var(--ip-ease-out); }
.btn-link:hover { color: var(--ip-primary-600); background-color: var(--ip-color-bg-tertiary); }

.loading-state { padding: 20px; text-align: center; color: var(--ip-color-text-tertiary); font-size: var(--ip-text-body-sm-size); }
.empty-hint { padding: 16px 12px; text-align: center; font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }
.node-warning { display: flex; align-items: flex-start; gap: 8px; margin: 0 0 12px; padding: 10px 14px; background: #fffbeb; border: 1px solid #fde68a; border-radius: var(--ip-radius-md); font-size: var(--ip-text-body-sm-size); color: #92400e; }
.node-warning a { color: #d97706; font-weight: var(--ip-font-weight-medium); }
.node-warn-icon { display: flex; align-items: center; justify-content: center; width: 20px; height: 20px; border-radius: 50%; background: #f59e0b; color: #fff; font-size: 12px; font-weight: 700; flex-shrink: 0; }
</style>
