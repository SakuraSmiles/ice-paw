<script setup lang="ts">
/**
 * Fixtures — E2E 测试专用页面
 *
 * 挂载 IpSelect、IpPopconfirm、IpDropdownMenu 三个组件，
 * 每个组件包裹在带有稳定 data-testid 的容器中。
 * 路径：/test/fixtures
 */
import { ref } from 'vue'
import {
  IpSelect,
  IpPopconfirm,
  IpDropdownMenu,
  Button,
} from '../../src'
import { Trash2 } from 'lucide-vue-next'

/* ── Select ── */
const selectValue = ref<string | null>(null)
const selectClearable = ref<string | null>('opt-b')
const selectError = ref(false)
const selectErrorMessage = ref('')

const toneOptions = [
  { value: 'concise', label: '简洁', description: '简短直接的回复' },
  { value: 'friendly', label: '友好', description: '带情感温度的回复' },
  { value: 'formal', label: '正式', description: '商务场合的回复' },
]

const modelOptions = [
  { value: 'gpt-4o', label: 'GPT-4o', description: 'OpenAI 最新模型' },
  { value: 'claude-3.5', label: 'Claude 3.5', description: 'Anthropic 最新模型' },
  { value: 'llama-local', label: 'Llama 本地', description: '本地部署的 Llama' },
  { value: 'deepseek', label: 'DeepSeek', description: '国产开源模型' },
]

function triggerSelectError(): void {
  selectError.value = true
  selectErrorMessage.value = '请选择一个有效的模型'
}

/* ── Popconfirm ── */
const popOpen = ref(false)
const popDangerOpen = ref(false)
const popConfirmCalled = ref(false)
const popCancelCalled = ref(false)

function confirmDelete(): void {
  popConfirmCalled.value = true
  popOpen.value = false
}
function onPopCancel(): void {
  popCancelCalled.value = true
}

/* ── DropdownMenu ── */
const ddOpen = ref(false)
const _ddItems = [
  { key: 'copy', label: '复制', shortcut: '⌘C' },
  { key: 'paste', label: '粘贴', shortcut: '⌘V' },
  { key: 'duplicate', label: '复制副本', shortcut: '⌘D' },
  { type: 'divider' as const, key: 'div1' },
  { type: 'label' as const, text: '操作', key: 'lbl1' },
  { key: 'share', label: '分享', icon: Trash2 },
  { key: 'delete', label: '删除', danger: true, shortcut: '⌫' },
]

/* divider / label 混合的完整菜单 */
const ddFullItems = [
  { type: 'label' as const, text: '编辑', key: 'lbl-edit' },
  { key: 'copy', label: '复制', shortcut: '⌘C' },
  { key: 'paste', label: '粘贴', shortcut: '⌘V' },
  { key: 'cut', label: '剪切', shortcut: '⌘X' },
  { type: 'divider' as const, key: 'div1' },
  { type: 'label' as const, text: '导出', key: 'lbl-export' },
  { key: 'export-pdf', label: '导出为 PDF' },
  { key: 'export-csv', label: '导出为 CSV' },
  { type: 'divider' as const, key: 'div2' },
  { key: 'delete', label: '删除', danger: true, shortcut: '⌫' },
]
</script>

<template>
  <div class="fixtures-root">
    <!-- ══════════════════════════════════════════
         SELECT
    ══════════════════════════════════════════ -->
    <section id="select" data-testid="fixture-select">
      <h1>E2E Fixtures — Select</h1>

      <!-- 基本展开/收起 -->
      <div data-testid="select-basic-wrap">
        <IpSelect
          v-model="selectValue"
          :options="toneOptions"
          placeholder="选择语气"
        />
      </div>

      <!-- 可清除（选中 opt-b） -->
      <div data-testid="select-clearable-wrap">
        <IpSelect
          v-model="selectClearable"
          :options="modelOptions"
          placeholder="选择一个模型"
          :clearable="true"
        />
      </div>

      <!-- 禁用 -->
      <div data-testid="select-disabled-wrap">
        <IpSelect
          :model-value="null"
          :options="toneOptions"
          placeholder="禁用状态"
          disabled
        />
      </div>

      <!-- 错误状态 -->
      <div data-testid="select-error-wrap">
        <IpSelect
          v-model="selectValue"
          :options="modelOptions"
          placeholder="选择一个模型"
          :error="selectError"
          :error-message="selectErrorMessage"
        />
        <button data-testid="select-error-trigger" @click="triggerSelectError">触发错误</button>
      </div>
    </section>

    <!-- ══════════════════════════════════════════
         POPCONFIRM
    ══════════════════════════════════════════ -->
    <section id="popconfirm" data-testid="fixture-popconfirm">
      <h1>E2E Fixtures — Popconfirm</h1>

      <!-- 基础确认 -->
      <div data-testid="popconfirm-basic-wrap">
        <IpPopconfirm
          v-model="popOpen"
          title="确定要删除吗？"
          description="此操作不可撤销。"
          confirm-text="确认"
          cancel-text="取消"
          @confirm="confirmDelete"
          @cancel="onPopCancel"
        >
          <template #trigger>
            <Button variant="danger" size="sm">删除</Button>
          </template>
        </IpPopconfirm>
      </div>

      <!-- 危险样式 -->
      <div data-testid="popconfirm-danger-wrap">
        <IpPopconfirm
          v-model="popDangerOpen"
          title="确认删除？"
          :danger="true"
          confirm-text="删除"
          cancel-text="保留"
          placement="bottom"
        >
          <template #trigger>
            <Button variant="ghost" size="sm">
              <Trash2 :size="14" />
              删除
            </Button>
          </template>
        </IpPopconfirm>
      </div>
    </section>

    <!-- ══════════════════════════════════════════
         DROPDOWN MENU
    ══════════════════════════════════════════ -->
    <section id="dropdown" data-testid="fixture-dropdown">
      <h1>E2E Fixtures — DropdownMenu</h1>

      <!-- 带分隔线和 label 的完整菜单 -->
      <div data-testid="dropdown-divider-wrap">
        <IpDropdownMenu v-model="ddOpen" :items="ddFullItems" placement="bottom-start">
          <template #trigger>
            <Button variant="secondary" size="sm">
              操作
            </Button>
          </template>
        </IpDropdownMenu>
      </div>
    </section>
  </div>
</template>

<style scoped>
.fixtures-root {
  padding: 48px;
  font-family: system-ui, sans-serif;
  display: flex;
  flex-direction: column;
  gap: 48px;
}
section {
  display: flex;
  flex-direction: column;
  gap: 16px;
}
h1 {
  font-size: 18px;
  font-weight: 700;
  margin-bottom: 8px;
  color: #333;
}
</style>
