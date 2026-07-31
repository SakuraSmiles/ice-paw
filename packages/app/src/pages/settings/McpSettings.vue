<script setup lang="ts">
// McpSettings.vue — MCP Server 设置（卡片展开内联编辑 + 顶部虚线新建卡片）
import { ref, onMounted } from "vue";
import McpForm from "../../components/mcp/McpForm.vue";
import Switch from "../../components/common/Switch.vue";
import type { McpServer, McpToolDef } from "../../types";
import { bridge } from "../../api/bridge";

const servers = ref<McpServer[]>([]);
const activeIds = ref<Set<string>>(new Set());
const loading = ref(true);

const expandedEditId = ref<string | null>(null);
const isCreating = ref(false);

// 工具清单缓存
const toolsMap = ref<Record<string, McpToolDef[]>>({});
const toolsLoading = ref<Record<string, boolean>>({});

async function reload() {
  loading.value = true;
  try {
    const [list, active] = await Promise.all([bridge.mcp.list(), bridge.mcp.listActive()]);
    servers.value = list;
    activeIds.value = new Set(active.map(([id]) => id));
  } catch (e) {
    console.error("加载 MCP Server 列表失败:", e);
  } finally {
    loading.value = false;
  }
}

onMounted(reload);

// 内置工具集（系统自带，只读展示；工具名以 register_builtin 为准）
const builtinExpanded = ref(false);
const builtinTools: { name: string; desc: string }[] = [
  { name: "read_file", desc: "读取本地文件内容" },
  { name: "list_directory", desc: "列出目录内容" },
  { name: "search_kb", desc: "检索知识库（对话中 agent 自动调用）" },
  { name: "read_kb_document", desc: "读取知识库文档全文（检索命中后读细节）" },
  { name: "save_to_kb", desc: "保存资料到知识库（对话中 agent 自动调用）" },
];

function isActive(id: string) { return activeIds.value.has(id); }

/** 状态：running 运行中 / stopped 已停用 / error 启用但未运行 */
function statusOf(s: McpServer): "running" | "stopped" | "error" | "per_agent" {
  if (s.scope === "per_agent") return "per_agent";
  if (isActive(s.id)) return "running";
  if (!s.enabled) return "stopped";
  return "error";
}
function statusLabel(s: McpServer): string {
  return { running: "运行中", stopped: "已停用", error: "启动失败", per_agent: "按 Agent" }[statusOf(s)];
}

function toggleEdit(s: McpServer) {
  isCreating.value = false;
  expandedEditId.value = expandedEditId.value === s.id ? null : s.id;
  // 展开且运行中时加载工具清单（仅首次）
  if (expandedEditId.value && !(s.id in toolsMap.value) && isActive(s.id)) {
    loadTools(s.id);
  }
}

function toggleNew() {
  expandedEditId.value = null;
  isCreating.value = !isCreating.value;
}

async function loadTools(id: string) {
  toolsLoading.value[id] = true;
  try {
    toolsMap.value[id] = await bridge.mcp.listTools(id);
  } catch (e) {
    console.error("加载工具清单失败:", e);
    toolsMap.value[id] = [];
  } finally {
    toolsLoading.value[id] = false;
  }
}

async function restart(s: McpServer) {
  try {
    await bridge.mcp.restart(s.id);
    await reload();
    if (isActive(s.id)) loadTools(s.id);
  } catch (e) {
    console.error("重启 MCP Server 失败:", e);
    await reload();
  }
}

/** 折叠卡片的即时启停（不需展开） */
async function toggleEnabled(s: McpServer, enabled: boolean) {
  try {
    await bridge.mcp.update({ id: s.id, enabled });
    await reload();
  } catch (e) {
    console.error("切换启用状态失败:", e);
    await reload();
  }
}

function onSaved(_s: McpServer) {
  isCreating.value = false;
  expandedEditId.value = null;
  reload();
}
function onCancel() {
  isCreating.value = false;
  expandedEditId.value = null;
}
async function onDelete(s: McpServer) {
  try {
    await bridge.mcp.remove(s.id);
    isCreating.value = false;
    expandedEditId.value = null;
    await reload();
  } catch (e) {
    console.error("删除 MCP Server 失败:", e);
  }
}
</script>

<template>
  <div class="settings-content-inner">
    <div class="content-header">
      <h2 class="content-title">工具集</h2>
    </div>

    <div class="mcp-list">
      <!-- 新建 MCP Server（列表第一条特殊卡片，虚线） -->
      <div class="mcp-card new-card" :class="{ expanded: isCreating }" @click="toggleNew">
        <div class="card-top">
          <div class="new-icon">
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
              <line x1="12" y1="5" x2="12" y2="19" /><line x1="5" y1="12" x2="19" y2="12" />
            </svg>
          </div>
          <div class="card-body">
            <div class="card-name-row">
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

      <!-- 分隔线 -->
      <div class="list-divider"></div>

      <!-- 内置工具集（系统自带，只读，折叠展开） -->
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

      <!-- server 列表 -->
      <div
        v-for="s in servers"
        :key="s.id"
        class="mcp-card"
        :class="{ expanded: expandedEditId === s.id }"
        @click="toggleEdit(s)"
      >
        <div class="card-top">
          <span class="status-dot" :class="'dot-' + statusOf(s)" />
          <div class="card-body">
            <div class="card-name-row">
              <span class="card-name">{{ s.name }}</span>
              <span class="status-tag" :class="'tag-' + statusOf(s)">{{ statusLabel(s) }}</span>
            </div>
          </div>
          <Switch :model-value="s.enabled" @update:model-value="(v: boolean) => toggleEnabled(s, v)" @click.stop />
          <svg class="card-chevron" :class="{ rotated: expandedEditId === s.id }" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="9 18 15 12 9 6" />
          </svg>
        </div>

        <!-- 展开态：配置 + 工具清单 -->
        <div v-if="expandedEditId === s.id" class="expand-panel" @click.stop>
          <McpForm :server="s" @saved="onSaved" @cancel="onCancel" @delete="onDelete" />

          <!-- 工具清单（caption 区段，无嵌套框） -->
          <div class="region">
            <div class="region-head">
              <span class="region-title">工具清单</span>
              <button class="btn-link" @click="restart(s)">
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <polyline points="23 4 23 10 17 10" /><polyline points="1 20 1 14 7 14" />
                  <path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15" />
                </svg>
                重启
              </button>
            </div>
            <div v-if="toolsLoading[s.id]" class="region-hint">加载中…</div>
            <div v-else-if="!isActive(s.id)" class="region-hint">启动后可查看工具清单</div>
            <template v-else>
              <div class="region-meta">共 {{ (toolsMap[s.id] ?? []).length }} 个工具</div>
              <div v-if="(toolsMap[s.id] ?? []).length" class="tool-chips">
                <span v-for="t in (toolsMap[s.id] ?? [])" :key="t.name" class="tool-chip" :title="t.description">{{ t.name }}</span>
              </div>
            </template>
          </div>
        </div>
      </div>

      <div v-if="loading && !servers.length" class="loading-state">加载中...</div>
      <div v-else-if="servers.length === 0" class="empty-hint">还没有 MCP Server，点上方「新建」接入</div>
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
.mcp-card:hover {
  border-color: var(--ip-primary-300);
  box-shadow: var(--ip-shadow-sm);
}
.mcp-card.expanded {
  border-color: var(--ip-primary-400);
  box-shadow: var(--ip-shadow-sm);
}

/* 新建卡片（虚线） */
.new-card {
  border: 1px dashed var(--ip-color-border-default);
  background-color: transparent;
}
.new-card:hover {
  border-color: var(--ip-primary-400);
  background-color: var(--ip-color-bg-tertiary);
}
.new-card.expanded {
  border-style: solid;
  border-color: var(--ip-primary-400);
  background-color: var(--ip-color-bg-secondary);
}
.new-icon {
  width: 36px; height: 36px; flex-shrink: 0;
  display: flex; align-items: center; justify-content: center;
  color: var(--ip-primary-600);
  background: var(--ip-primary-50);
  border-radius: var(--ip-radius-md);
}

/* 内置工具集 */
.builtin-badge {
  font-size: 10px;
  font-weight: var(--ip-font-weight-medium);
  padding: 0 6px;
  line-height: 18px;
  color: var(--ip-primary-700);
  background: var(--ip-primary-100);
  border-radius: var(--ip-radius-full);
}
.builtin-count {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  flex-shrink: 0;
}
.builtin-tools {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.builtin-tool {
  display: flex;
  align-items: baseline;
  gap: 12px;
}
.builtin-tool-name {
  flex-shrink: 0;
  min-width: 120px;
  font-family: var(--ip-font-mono);
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-primary);
}
.builtin-tool-desc {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
}

/* 分隔线 */
.list-divider {
  height: 1px;
  background-color: var(--ip-color-border-default);
  margin: 2px 4px;
}

.card-top {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  cursor: pointer;
}

/* 状态圆点 */
.status-dot {
  width: 8px; height: 8px; border-radius: 50%; flex-shrink: 0;
  margin-top: 6px;
}
.dot-running { background: var(--ip-success-base); }
.dot-stopped { background: var(--ip-color-border-default); }
.dot-error {
  background: var(--ip-danger-base);
  animation: dot-pulse 1.2s ease-in-out infinite;
}
.dot-per_agent { background: var(--ip-primary-500); }
@keyframes dot-pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.4; } }

.card-body {
  flex: 1; min-width: 0;
  display: flex; flex-direction: column; gap: 3px;
}
.card-name-row { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
.card-name {
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
}
.new-name { color: var(--ip-primary-600); }
.new-hint { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }

.status-tag {
  font-size: 10px; font-weight: var(--ip-font-weight-medium);
  padding: 0 6px; line-height: 18px; border-radius: var(--ip-radius-full); flex-shrink: 0;
}
.tag-running { background: var(--ip-success-bg); color: var(--ip-success-text); }
.tag-stopped { background: var(--ip-color-bg-tertiary); color: var(--ip-color-text-tertiary); }
.tag-error { background: var(--ip-danger-bg); color: var(--ip-danger-text); }
.tag-per_agent { background: var(--ip-primary-100); color: var(--ip-primary-700); }

.card-desc {
  font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
.card-cmd {
  font-size: var(--ip-text-caption-size); color: var(--ip-color-text-secondary);
  font-family: var(--ip-font-mono);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}

.card-chevron {
  flex-shrink: 0;
  color: var(--ip-color-text-disabled);
  transition: transform var(--ip-duration-fast) var(--ip-ease-out);
  margin-top: 2px;
}
.card-chevron.rotated {
  transform: rotate(90deg);
  color: var(--ip-primary-600);
}

/* ===== 展开面板 ===== */
.expand-panel {
  margin-top: 12px;
  padding-top: 12px;
  border-top: 1px solid var(--ip-color-border-default);
}

/* 区段（工具清单，caption 标题 + 留白） */
.region {
  margin-top: 18px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.region-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.region-title {
  font-size: var(--ip-text-caption-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-tertiary);
  letter-spacing: 0.02em;
}
.region-meta {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-secondary);
}
.region-hint {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
}

/* 工具标签（扁平，无强框） */
.tool-chips {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}
.tool-chip {
  font-size: 11px;
  font-family: var(--ip-font-mono);
  padding: 2px 8px;
  line-height: 18px;
  color: var(--ip-primary-700);
  background-color: var(--ip-color-bg-tertiary);
  border-radius: var(--ip-radius-full);
}

/* 文字按钮（重启） */
.btn-link {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  height: 26px;
  padding: 0 8px;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-secondary);
  background: none;
  border: none;
  border-radius: var(--ip-radius-md);
  cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.btn-link:hover {
  color: var(--ip-primary-600);
  background-color: var(--ip-color-bg-tertiary);
}

/* ===== 状态 ===== */
.loading-state { padding: 20px; text-align: center; color: var(--ip-color-text-tertiary); font-size: var(--ip-text-body-sm-size); }
.empty-hint { padding: 16px 12px; text-align: center; font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }
</style>
