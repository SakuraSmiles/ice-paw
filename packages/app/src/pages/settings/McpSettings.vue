<script setup lang="ts">
// McpSettings.vue — MCP Server 配置管理
import { ref, onMounted } from "vue";
import type { McpServer, McpToolDef } from "../../types";
import { bridge } from "../../api/bridge";
import Switch from "../../components/common/Switch.vue";
import McpFormModal from "../../components/mcp/McpFormModal.vue";

const servers = ref<McpServer[]>([]);
const activeIds = ref<Set<string>>(new Set());
const loading = ref(true);

const showForm = ref(false);
const editing = ref<McpServer | null>(null);

// 工具清单展开状态
const expandedId = ref<string | null>(null);
const toolsMap = ref<Record<string, McpToolDef[]>>({});
const toolsLoading = ref<Record<string, boolean>>({});

async function reload() {
  loading.value = true;
  try {
    const [list, active] = await Promise.all([
      bridge.mcp.list(),
      bridge.mcp.listActive(),
    ]);
    servers.value = list;
    activeIds.value = new Set(active.map(([id]) => id));
  } catch (e) {
    console.error("加载 MCP Server 列表失败:", e);
  } finally {
    loading.value = false;
  }
}

onMounted(reload);

function isActive(id: string) { return activeIds.value.has(id); }

/** 状态：running 运行中 / stopped 已停用 / error 启用但未运行 */
function statusOf(s: McpServer): "running" | "stopped" | "error" {
  if (isActive(s.id)) return "running";
  if (!s.enabled) return "stopped";
  return "error";
}
function statusLabel(s: McpServer): string {
  return { running: "运行中", stopped: "已停用", error: "启动失败" }[statusOf(s)];
}

function openNew() { editing.value = null; showForm.value = true; }
function openEdit(s: McpServer) { editing.value = s; showForm.value = true; }
function closeForm() { showForm.value = false; editing.value = null; }

async function onSaved(_s: McpServer) {
  await reload();
  closeForm();
}

async function onDelete(s: McpServer) {
  try {
    await bridge.mcp.remove(s.id);
    await reload();
    closeForm();
  } catch (e) {
    console.error("删除 MCP Server 失败:", e);
  }
}

async function toggleEnabled(s: McpServer, enabled: boolean) {
  try {
    await bridge.mcp.update({ id: s.id, enabled });
    await reload();
  } catch (e) {
    console.error("切换启用状态失败:", e);
    await reload();
  }
}

async function restart(s: McpServer) {
  try {
    await bridge.mcp.restart(s.id);
    await reload();
  } catch (e) {
    console.error("重启 MCP Server 失败:", e);
    await reload();
  }
}

async function toggleExpand(s: McpServer) {
  if (expandedId.value === s.id) {
    expandedId.value = null;
    return;
  }
  expandedId.value = s.id;
  if (!(s.id in toolsMap.value) && isActive(s.id)) {
    toolsLoading.value[s.id] = true;
    try {
      toolsMap.value[s.id] = await bridge.mcp.listTools(s.id);
    } catch (e) {
      console.error("加载工具清单失败:", e);
      toolsMap.value[s.id] = [];
    } finally {
      toolsLoading.value[s.id] = false;
    }
  }
}

function trustLabel(s: McpServer) {
  return s.trust_level === "trusted" ? "免确认" : "每次确认";
}
</script>

<template>
  <div class="settings-content-inner">
    <div class="content-header">
      <h2 class="content-title">MCP 工具</h2>
      <button class="btn-primary" @click="openNew">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <line x1="12" y1="5" x2="12" y2="19" /><line x1="5" y1="12" x2="19" y2="12" />
        </svg>
        新建
      </button>
    </div>

    <div class="mcp-list">
      <div v-for="s in servers" :key="s.id" class="mcp-card">
        <div class="card-top">
          <div class="card-info" @click="openEdit(s)">
            <span class="status-dot" :class="'dot-' + statusOf(s)" />
            <div class="card-text">
              <div class="card-name-row">
                <span class="card-name">{{ s.name }}</span>
                <span class="status-tag" :class="'tag-' + statusOf(s)">{{ statusLabel(s) }}</span>
                <span v-if="s.description" class="card-desc">{{ s.description }}</span>
              </div>
              <div class="card-cmd">{{ s.command }}<span v-if="s.args.length"> {{ s.args.join(" ") }}</span></div>
              <div class="card-meta">
                <span>信任：{{ trustLabel(s) }}</span>
              </div>
            </div>
          </div>

          <div class="card-actions">
            <Switch :model-value="s.enabled" @update:model-value="(v: boolean) => toggleEnabled(s, v)" />
            <button class="icon-btn" title="重启" :disabled="!isActive(s.id)" @click="restart(s)">
              <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <polyline points="23 4 23 10 17 10" /><polyline points="1 20 1 14 7 14" />
                <path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15" />
              </svg>
            </button>
            <button class="icon-btn" title="编辑" @click="openEdit(s)">
              <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7" />
                <path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z" />
              </svg>
            </button>
          </div>
        </div>

        <div class="tools-toggle" @click="toggleExpand(s)">
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" :class="{ rotated: expandedId === s.id }">
            <polyline points="6 9 12 15 18 9" />
          </svg>
          {{ expandedId === s.id ? "收起" : "查看工具清单" }}
        </div>

        <div v-if="expandedId === s.id" class="tools-panel">
          <div v-if="toolsLoading[s.id]" class="tools-hint">加载中…</div>
          <div v-else-if="!isActive(s.id)" class="tools-hint">启动后可查看工具清单</div>
          <template v-else>
            <div class="tools-count">共 {{ (toolsMap[s.id] ?? []).length }} 个工具</div>
            <div v-if="(toolsMap[s.id] ?? []).length" class="tool-chips">
              <span v-for="t in (toolsMap[s.id] ?? [])" :key="t.name" class="tool-chip" :title="t.description">
                {{ t.name }}
              </span>
            </div>
          </template>
        </div>
      </div>

      <div v-if="loading" class="loading-state">加载中...</div>
      <div v-else-if="servers.length === 0" class="empty-state">
        <div class="empty-icon">
          <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <path d="M9 2v6" /><path d="M15 2v6" /><path d="M6 8h12v3a6 6 0 0 1-12 0V8z" /><path d="M12 17v5" />
          </svg>
        </div>
        <h3 class="empty-title">还没有 MCP Server</h3>
        <p class="empty-desc">添加一个 MCP Server，为对话接入外部工具能力</p>
        <button class="btn-primary" @click="openNew">添加 MCP Server</button>
      </div>
    </div>

    <McpFormModal v-if="showForm" :server="editing" @close="closeForm" @saved="onSaved" @delete="onDelete" />
  </div>
</template>

<style scoped>
.settings-content-inner { flex: 1; display: flex; flex-direction: column; padding: 0; min-height: 0; }

.content-header {
  display: flex; align-items: center; justify-content: space-between;
  padding: 20px 28px 0; flex-shrink: 0; height: 56px;
}
.content-title {
  font-size: var(--ip-text-h3-size); font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary); margin: 0;
}

.btn-primary {
  display: flex; align-items: center; gap: 6px; padding: 0 14px; height: 32px;
  font-size: var(--ip-text-body-sm-size); font-weight: var(--ip-font-weight-medium);
  color: white; background-color: var(--ip-primary-600); border: none;
  border-radius: var(--ip-radius-md); cursor: pointer; white-space: nowrap;
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out);
}
.btn-primary:hover { background-color: var(--ip-primary-700); }

/* ===== 列表 ===== */
.mcp-list {
  flex: 1; overflow-y: auto; padding: 8px 28px 24px;
  display: flex; flex-direction: column; gap: 8px; min-height: 0;
}

.mcp-card {
  padding: 14px 16px;
  background-color: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-lg);
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.mcp-card:hover {
  border-color: var(--ip-primary-300);
  box-shadow: var(--ip-shadow-sm);
}

.card-top {
  display: flex; align-items: flex-start; gap: 12px;
}
.card-info {
  flex: 1; min-width: 0; display: flex; align-items: flex-start; gap: 10px;
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
@keyframes dot-pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.4; } }

.card-text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 3px; }
.card-name-row { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
.card-name {
  font-size: var(--ip-text-body-sm-size); font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
}
.status-tag {
  font-size: 10px; font-weight: var(--ip-font-weight-medium);
  padding: 0 6px; line-height: 18px; border-radius: var(--ip-radius-full); flex-shrink: 0;
}
.tag-running { background: var(--ip-success-bg); color: var(--ip-success-text); }
.tag-stopped { background: var(--ip-color-bg-tertiary); color: var(--ip-color-text-tertiary); }
.tag-error { background: var(--ip-danger-bg); color: var(--ip-danger-text); }
.card-desc {
  font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
.card-cmd {
  font-size: var(--ip-text-caption-size); color: var(--ip-color-text-secondary);
  font-family: var(--ip-font-mono);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
.card-meta {
  font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary);
}

.card-actions {
  display: flex; align-items: center; gap: 6px; flex-shrink: 0;
}
.icon-btn {
  display: flex; align-items: center; justify-content: center;
  width: 28px; height: 28px; border-radius: var(--ip-radius-md);
  color: var(--ip-color-text-secondary); cursor: pointer;
  background: none; border: 1px solid transparent;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.icon-btn:hover:not(:disabled) {
  color: var(--ip-primary-600); background-color: var(--ip-color-bg-tertiary);
}
.icon-btn:disabled { opacity: 0.35; cursor: not-allowed; }

.tools-toggle {
  display: flex; align-items: center; gap: 5px;
  margin-top: 8px; padding-left: 18px;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary); cursor: pointer;
  user-select: none;
}
.tools-toggle:hover { color: var(--ip-primary-600); }
.tools-toggle svg { transition: transform var(--ip-duration-fast) var(--ip-ease-out); }
.tools-toggle svg.rotated { transform: rotate(180deg); }

.tools-panel {
  margin-top: 8px; padding: 10px 12px 10px 18px;
  background-color: var(--ip-color-bg-tertiary);
  border-radius: var(--ip-radius-md);
}
.tools-count {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-secondary); margin-bottom: 8px;
}
.tool-chips { display: flex; flex-wrap: wrap; gap: 6px; }
.tool-chip {
  font-size: 11px; font-family: var(--ip-font-mono);
  padding: 2px 8px; line-height: 18px;
  color: var(--ip-primary-700);
  background-color: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-full);
}
.tools-hint {
  font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary);
}

/* ===== 状态 ===== */
.loading-state { flex: 1; display: flex; align-items: center; justify-content: center; color: var(--ip-color-text-tertiary); font-size: var(--ip-text-body-sm-size); }
.empty-state { flex: 1; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 12px; padding: 40px; }
.empty-icon { width: 48px; height: 48px; margin-bottom: 8px; color: var(--ip-color-text-tertiary); }
.empty-title { font-size: var(--ip-text-body-size); font-weight: var(--ip-font-weight-semibold); color: var(--ip-color-text-primary); margin: 0; }
.empty-desc { font-size: var(--ip-text-body-sm-size); color: var(--ip-color-text-secondary); margin: 0 0 8px; }
</style>
