<script setup lang="ts">
// 首页内联 Agent 创建表单（WelcomeScreen 专用）
//
// 职责：
//   - 在 ChatPage 首屏直接展示一个紧凑的创建表单
//   - 用户填写「名称 / Provider / Model / API Key」后点击「创建并开始对话」
//   - 创建成功后自动调用 agentsStore.setCurrent，ChatPage 会切换到有 Agent 无会话的 WelcomeScreen
//   - 底部「跳过，稍后配置」链接跳转到 AgentManagerPage
//
// props: 无（自包含，读取 / 写入 agentsStore）
//
// emits: 无（侧边栏保持显示，不需要额外的状态通知）

import { computed, ref } from "vue";
import { useRouter } from "vue-router";
import { Input, Button } from "@ice-paw/ui";
import { Plus, ArrowRight } from "lucide-vue-next";
import { useAgentsStore } from "../../stores/agents";
import { useToast } from "../../composables/useToast";
import type { NewAgent } from "../../types";

// ============================================================================
// Provider / Model 预设
// ============================================================================

const PROVIDERS = ["OpenAI", "Anthropic", "GLM", "DeepSeek"] as const;
type ProviderName = (typeof PROVIDERS)[number];

const MODEL_PRESETS: Record<ProviderName, string[]> = {
  OpenAI: ["gpt-4o", "gpt-4o-mini"],
  Anthropic: ["claude-sonnet-4-20250514"],
  GLM: ["glm-4-flash", "glm-4-plus"],
  DeepSeek: ["deepseek-chat", "deepseek-reasoner"],
};

// ============================================================================
// Store / Router
// ============================================================================

const agentsStore = useAgentsStore();
const router = useRouter();
const toast = useToast();

// ============================================================================
// 表单状态
// ============================================================================

const name = ref<string>("");
const provider = ref<string>(PROVIDERS[0]);
const model = ref<string>(MODEL_PRESETS[PROVIDERS[0]][0]);
const apiKey = ref<string>("");

/** 提交状态（防止重复点击） */
const submitting = ref<boolean>(false);

/** 字段错误 */
const errors = ref<Record<string, string>>({});

// ============================================================================
// 派生
// ============================================================================

const presetModels = computed<string[]>(() => {
  return MODEL_PRESETS[provider.value as ProviderName] ?? [];
});

const nameError = computed<string>(() => errors.value.name ?? "");
const apiKeyError = computed<string>(() => errors.value.api_key ?? "");

// ============================================================================
// 联动：Provider 切换 → 重置 Model
// ============================================================================

function onProviderChange(e: Event): void {
  const target = e.target as EventTarget & { value: string };
  provider.value = target.value;
  const presets = MODEL_PRESETS[provider.value as ProviderName] ?? [];
  model.value = presets[0] ?? "";
}

// ============================================================================
// 校验
// ============================================================================

function validate(): boolean {
  const errs: Record<string, string> = {};
  if (!name.value.trim()) {
    errs.name = "请输入 Agent 名称";
  }
  if (!apiKey.value.trim()) {
    errs.api_key = "请输入 API Key";
  } else if (apiKey.value.trim().length < 10) {
    errs.api_key = "API Key 至少 10 位";
  }
  errors.value = errs;
  return Object.keys(errs).length === 0;
}

// ============================================================================
// 提交
// ============================================================================

async function handleSubmit(): Promise<void> {
  if (submitting.value) return;
  if (!validate()) return;

  submitting.value = true;
  try {
    const input: NewAgent = {
      name: name.value.trim(),
      provider: provider.value.toLowerCase(),
      model: model.value,
      api_key: apiKey.value.trim(),
    };
    const created = await agentsStore.createOne(input);
    // createOne 内部已经 setCurrent；此处显式再设一次以保证 store 状态稳定
    agentsStore.setCurrent(created.id);
    // 不弹 Toast，由界面切换反馈成功；如有 error 字段可弹
    if (agentsStore.error) {
      toast.error(agentsStore.error);
    }
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    toast.error(`创建失败：${msg}`);
  } finally {
    submitting.value = false;
  }
}

// ============================================================================
// 跳过 → 跳到 AgentManagerPage
// ============================================================================

function goToAgentManager(): void {
  void router.push({ name: "AgentManager" });
}
</script>

<template>
  <div class="inline-create-root">
    <div class="inline-create-card">
      <header class="inline-create-header">
        <h2 class="inline-create-title">创建你的第一个 Agent</h2>
        <p class="inline-create-subtitle">
          填写下方四个字段，立即开始与 AI 对话。
        </p>
      </header>

      <form class="inline-form" @submit.prevent="handleSubmit">
        <div class="form-group">
          <label class="form-label" for="iac-name">名称</label>
          <Input
            id="iac-name"
            v-model="name"
            size="md"
            placeholder="例如：我的助手"
            autocomplete="off"
            :error="Boolean(nameError)"
            :error-message="nameError"
          />
        </div>

        <div class="form-row">
          <div class="form-group">
            <label class="form-label" for="iac-provider">Provider</label>
            <select
              id="iac-provider"
              :value="provider"
              class="form-select"
              @change="onProviderChange"
            >
              <option v-for="p in PROVIDERS" :key="p" :value="p">{{ p }}</option>
            </select>
          </div>

          <div class="form-group">
            <label class="form-label" for="iac-model">模型</label>
            <select id="iac-model" v-model="model" class="form-select">
              <option v-for="m in presetModels" :key="m" :value="m">{{ m }}</option>
            </select>
          </div>
        </div>

        <div class="form-group">
          <label class="form-label" for="iac-apikey">API Key</label>
          <Input
            id="iac-apikey"
            v-model="apiKey"
            size="md"
            type="password"
            placeholder="sk-..."
            autocomplete="off"
            :error="Boolean(apiKeyError)"
            :error-message="apiKeyError"
          />
        </div>

        <div class="form-actions">
          <Button
            type="submit"
            variant="primary"
            size="md"
            :disabled="submitting"
            @click="handleSubmit"
          >
            <template #icon-left>
              <Plus :size="16" aria-hidden="true" />
            </template>
            {{ submitting ? "创建中…" : "创建并开始对话" }}
          </Button>
        </div>
      </form>

      <footer class="inline-create-footer">
        <button type="button" class="skip-link" @click="goToAgentManager">
          跳过，稍后配置
          <ArrowRight :size="14" aria-hidden="true" />
        </button>
      </footer>
    </div>
  </div>
</template>

<style scoped>
.inline-create-root {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 48px var(--ip-spacing-6);
  background: var(--ip-color-bg-primary);
  overflow-y: auto;
}

.inline-create-card {
  width: 100%;
  max-width: 480px;
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-lg, 12px);
  box-shadow: 0 4px 20px -8px rgba(0, 0, 0, 0.08);
  padding: var(--ip-spacing-6);
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-5);
}

.inline-create-header {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-1);
}

.inline-create-title {
  margin: 0;
  font-size: var(--ip-text-h3-size, 18px);
  font-weight: var(--ip-font-weight-semibold, 600);
  color: var(--ip-color-text-primary);
}

.inline-create-subtitle {
  margin: 0;
  font-size: var(--ip-text-body-sm-size, 13px);
  color: var(--ip-color-text-tertiary);
  line-height: var(--ip-line-height-relaxed, 1.5);
}

.inline-form {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-4);
}

.form-row {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: var(--ip-spacing-3);
}

.form-group {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-1);
}

.form-label {
  font-size: var(--ip-text-body-sm-size, 13px);
  font-weight: var(--ip-font-weight-medium, 500);
  color: var(--ip-color-text-primary);
}

.form-select {
  width: 100%;
  padding: 8px 12px;
  font-family: inherit;
  font-size: var(--ip-text-body-sm-size, 13px);
  color: var(--ip-color-text-body);
  background: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-input-radius, 8px);
  outline: none;
  transition:
    border-color var(--ip-duration-fast, 150ms) var(--ip-ease-out, ease-out),
    box-shadow var(--ip-duration-fast, 150ms) var(--ip-ease-out, ease-out);
  cursor: pointer;
}

.form-select:hover {
  border-color: var(--ip-color-border-strong);
}

.form-select:focus {
  border-color: var(--ip-color-border-focus);
  box-shadow: var(--ip-shadow-focus, 0 0 0 3px rgba(59, 130, 246, 0.3));
}

.form-actions {
  display: flex;
  justify-content: flex-end;
  margin-top: var(--ip-spacing-1);
}

.inline-create-footer {
  display: flex;
  justify-content: center;
  border-top: 1px dashed var(--ip-color-border-default);
  padding-top: var(--ip-spacing-3);
}

.skip-link {
  appearance: none;
  background: none;
  border: none;
  padding: 4px 8px;
  font-family: inherit;
  font-size: var(--ip-text-body-sm-size, 13px);
  color: var(--ip-color-text-tertiary);
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  border-radius: var(--ip-radius-sm, 4px);
  transition:
    color var(--ip-duration-fast, 150ms) var(--ip-ease-out, ease-out),
    background-color var(--ip-duration-fast, 150ms) var(--ip-ease-out, ease-out);
}

.skip-link:hover {
  color: var(--ip-color-text-primary);
  background: var(--ip-color-bg-tertiary);
}

.skip-link:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus, 0 0 0 3px rgba(59, 130, 246, 0.3));
}
</style>