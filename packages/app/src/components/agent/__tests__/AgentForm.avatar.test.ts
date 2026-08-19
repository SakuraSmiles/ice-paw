// AgentForm.avatar.test.ts — 头像出生证字段读写锁：
// 编辑态预填（预览两级链：图片/名字渐变首字）→ 保存透传 update payload
// （null=清空语义）；清除归 null；新建透传 create input。
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { mount, flushPromises, type VueWrapper } from "@vue/test-utils";
import type { Agent } from "../../../types";

// 顶层 Tauri 插件 import 须先 mock（同 AgentForm.providers.test.ts）
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({ revealItemInDir: vi.fn() }));

const providersListMock = vi.fn();
const createMock = vi.fn();
const updateMock = vi.fn();
const rotateKeyMock = vi.fn();

vi.mock("../../../api/bridge", () => ({
  bridge: {
    providers: {
      list: (...a: unknown[]) => providersListMock(...a),
      testConnection: vi.fn(),
    },
    preferences: { get: async () => ({}) },
    agents: {
      create: (...a: unknown[]) => createMock(...a),
      update: (...a: unknown[]) => updateMock(...a),
      rotateKey: (...a: unknown[]) => rotateKeyMock(...a),
      list: vi.fn(async () => []),
    },
  },
}));

function editAgent(overrides?: Partial<Agent>): Agent {
  return {
    id: "ag-1",
    name: "助手",
    provider: "openai",
    model: "gpt-4o",
    system_prompt: "",
    base_url: null,
    temperature: 0.7,
    max_tokens: 16384,
    extra_params: {},
    sort_order: 0,
    cache_prompt: true,
    workspace_path: null,
    config_from_file: false,
    created_at: "2026-08-15 00:00:00",
    updated_at: "2026-08-15 00:00:00",
    has_api_key: true,
    ...overrides,
  } as Agent;
}

const wrappers: VueWrapper[] = [];

async function mountForm(agent: Agent | null = null) {
  providersListMock.mockResolvedValue([
    { name: "openai", protocol: "openai", default_url: "https://api.openai.com", alt_urls: [], label: "OpenAI", note: null, requires_key: true, requires_base_url: false, hidden: false, models: ["gpt-4o"] },
  ]);
  const { default: AgentForm } = await import("../AgentForm.vue");
  const w = mount(AgentForm, { props: { agent }, attachTo: document.body });
  wrappers.push(w);
  await flushPromises();
  return w;
}

/** 预览头像（表单头像行的 EntityAvatar） */
function preview(w: VueWrapper) {
  return w.find(".avatar-row .entity-avatar");
}

/** 保存按钮（配置区头部） */
function saveBtn(w: VueWrapper) {
  return w.find(".section-actions .btn-primary");
}

describe("AgentForm 头像字段", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });
  afterEach(() => {
    for (const w of wrappers) w.unmount();
    wrappers.length = 0;
    document.body.innerHTML = "";
  });

  it("编辑态预填：图片档预览 img；无图回名字首字渐变", async () => {
    const w = await mountForm(editAgent({ avatar: "data:image/webp;base64,old" }));
    expect(preview(w).find("img").attributes("src")).toBe("data:image/webp;base64,old");

    const w2 = await mountForm(editAgent());
    expect(preview(w2).find("img").exists()).toBe(false);
    expect(preview(w2).text()).toBe("助"); // 首字兜底
  });

  it("编辑态保存透传：avatar 原样进 update payload", async () => {
    const a = editAgent({ avatar: "data:image/webp;base64,old" });
    const w = await mountForm(a);
    await saveBtn(w).trigger("click");
    await flushPromises();
    expect(updateMock).toHaveBeenCalledTimes(1);
    const payload = updateMock.mock.calls[0][0];
    expect(payload.avatar).toBe("data:image/webp;base64,old");
  });

  it("清除按钮：图片归 null（预览回兜底首字），保存 payload avatar=null", async () => {
    const w = await mountForm(editAgent({ avatar: "data:image/webp;base64,x" }));
    // 只剩两个操作钮（上传/清除），清除是第二个
    await w.findAll(".avatar-actions .avatar-btn")[1].trigger("click");
    expect(preview(w).text()).toBe("助");

    await saveBtn(w).trigger("click");
    await flushPromises();
    const payload = updateMock.mock.calls[0][0];
    expect(payload.avatar).toBeNull(); // 清空（双层 Option 的 Some(None) 语义）
  });

  it("新建：未选头像 create input 透传 avatar undefined", async () => {
    createMock.mockResolvedValue(editAgent());
    const w = await mountForm(null);
    const nameInput = w.findAll("input.input")[0];
    await nameInput.setValue("新助手");
    const idInput = w.findAll("input.input")[1];
    await idInput.setValue("new-helper");

    // 模型：开下拉点预设条目（真实用户路径，同 providers 测试范式）
    await w.find(".gs-input").trigger("focus");
    const opt = w.findAll(".gs-option").find((o) => o.text().includes("gpt-4o"));
    expect(opt, "下拉应含 gpt-4o").toBeTruthy();
    await opt!.trigger("click");
    await flushPromises();

    // Key 必填（openai requires_key）
    const keyInput = w.findAll('input[type="password"]')[0];
    await keyInput.setValue("sk-test-12345678");

    await saveBtn(w).trigger("click");
    await flushPromises();
    expect(createMock).toHaveBeenCalledTimes(1);
    const input = createMock.mock.calls[0][0];
    expect(input.avatar).toBeUndefined();
  });
});
