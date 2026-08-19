// AgentForm.avatar.test.ts — 头像出生证字段读写锁：
// 编辑态预填（预览三级链）→ 保存透传 update payload（null=清空语义）；
// emoji 弹层选择与图片互斥；清除归 null；新建透传 create input。
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

  it("编辑态预填：emoji 档预览显示选定字符", async () => {
    const w = await mountForm(editAgent({ emoji: "🧊" }));
    expect(preview(w).text()).toBe("🧊");
  });

  it("编辑态保存透传：emoji 原样进 update payload", async () => {
    const a = editAgent({ emoji: "🧊" });
    const w = await mountForm(a);
    await saveBtn(w).trigger("click");
    await flushPromises();
    expect(updateMock).toHaveBeenCalledTimes(1);
    const payload = updateMock.mock.calls[0][0];
    expect(payload.avatar).toBeNull();
    expect(payload.emoji).toBe("🧊");
  });

  it("emoji 弹层选择：emit select → 表单态更新 + 图片互斥清空 + 保存清空 avatar", async () => {
    const a = editAgent({ avatar: "data:image/webp;base64,old" });
    const w = await mountForm(a);
    expect(preview(w).find("img").attributes("src")).toBe("data:image/webp;base64,old");

    // 开弹层 → 点第一个 emoji cell
    await w.findAll(".avatar-actions .avatar-btn")[1].trigger("click");
    await w.find(".emoji-pop .emoji-cell").trigger("click");
    await flushPromises();

    // 互斥：图片清空、emoji 生效（预览不再是 img）
    expect(preview(w).find("img").exists()).toBe(false);
    expect(preview(w).text()).toBe("🦊");

    await saveBtn(w).trigger("click");
    await flushPromises();
    const payload = updateMock.mock.calls[0][0];
    expect(payload.avatar).toBeNull(); // 清空（双层 Option 的 Some(None) 语义）
    expect(payload.emoji).toBe("🦊");
  });

  it("「不使用 emoji」清除：emoji 归 null，保存 payload 双 null", async () => {
    const w = await mountForm(editAgent({ emoji: "🧊" }));
    await w.findAll(".avatar-actions .avatar-btn")[1].trigger("click");
    await w.find(".emoji-pop .emoji-clear").trigger("click");
    expect(preview(w).text()).toBe("助"); // 回名字首字兜底

    await saveBtn(w).trigger("click");
    await flushPromises();
    const payload = updateMock.mock.calls[0][0];
    expect(payload.avatar).toBeNull();
    expect(payload.emoji).toBeNull();
  });

  it("清除按钮：图片 + emoji 双清（预览回兜底首字）", async () => {
    const w = await mountForm(editAgent({ avatar: "data:image/webp;base64,x", emoji: null }));
    await w.findAll(".avatar-actions .avatar-btn")[2].trigger("click"); // 第三个按钮=清除
    expect(preview(w).text()).toBe("助");
  });

  it("新建：选 emoji 后 create input 透传 avatar undefined / emoji 值", async () => {
    createMock.mockResolvedValue(editAgent());
    const w = await mountForm(null);
    // 填出生证必填项（模型选预设需交互，直接走 GroupedSelect 之外的兜底：
    // 手输模型落 custom 会要求 URL——用预设组第一项更简单）
    const nameInput = w.findAll("input.input")[0];
    await nameInput.setValue("新助手");
    const idInput = w.findAll("input.input")[1];
    await idInput.setValue("new-helper");

    // 选 emoji
    await w.findAll(".avatar-actions .avatar-btn")[1].trigger("click");
    await w.find(".emoji-pop .emoji-cell").trigger("click");

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
    expect(input.emoji).toBe("🦊");
    expect(input.avatar).toBeUndefined();
  });
});
