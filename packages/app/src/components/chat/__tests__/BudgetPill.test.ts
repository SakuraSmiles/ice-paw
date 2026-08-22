// BudgetPill — 会话级预算胶囊（chat:budget HUD）测试：数值渲染 / 环形进度 / 80% warn 态 / 续期计数 / 缓存命中 chip
import { describe, it, expect } from "vitest";
import { mount } from "@vue/test-utils";
import BudgetPill from "../BudgetPill.vue";
import type { ChatBudgetPayload } from "../../../types";

const budget = (overrides?: Partial<ChatBudgetPayload>): ChatBudgetPayload => ({
  conversation_id: "c1",
  cumulative_tokens: 120_000,
  cumulative_cached_tokens: 0,
  cumulative_prompt_tokens: 120_000,
  effective_cap: 600_000,
  initial_cap: 600_000,
  renewal_index: 0,
  max_renewals: 2,
  renewed: false,
  round: 3,
  ...overrides,
});

describe("BudgetPill", () => {
  it("渲染「已用 / 上限」短格式（万进制）", () => {
    const w = mount(BudgetPill, { props: { budget: budget() } });
    expect(w.text()).toContain("12万");
    expect(w.text()).toContain("60万");
    expect(w.find(".renewed").exists()).toBe(false);
  });

  it("用量 ≥80% 上限：warn 态（soft 底色语义类）", () => {
    const normal = mount(BudgetPill, { props: { budget: budget() } });
    expect(normal.classes()).not.toContain("warn");
    const hot = mount(BudgetPill, {
      props: { budget: budget({ cumulative_tokens: 500_000 }) }, // 83%
    });
    expect(hot.classes()).toContain("warn");
  });

  it("环形进度：偏移随用量走 + 80% 刻度点存在", () => {
    const w = mount(BudgetPill, { props: { budget: budget() } }); // 12万/60万 = 20%
    // C=2π·8≈50.265，20% 填充 → offset = C×0.8 ≈ 40.21
    expect(w.find(".ring-fill").attributes("stroke-dashoffset")).toContain("40.2");
    expect(w.find(".budget-tick").exists()).toBe(true);
    // 满环钳制：瞬时超限不撑破环
    const over = mount(BudgetPill, {
      props: { budget: budget({ cumulative_tokens: 999_999 }) },
    });
    expect(over.find(".ring-fill").attributes("stroke-dashoffset")).toBe("0");
  });

  it("缓存命中：cached>0 时显示百分比 chip（生产实证量级 96%）", () => {
    const w = mount(BudgetPill, {
      props: {
        budget: budget({
          cumulative_cached_tokens: 413_184,
          cumulative_prompt_tokens: 430_000,
        }),
      },
    });
    expect(w.find(".cached").text()).toContain("缓存命中 96%");
  });

  it("无缓存数据不渲染命中 chip（Ollama 等无缓存 provider）", () => {
    const w = mount(BudgetPill, { props: { budget: budget() } });
    expect(w.find(".cached").exists()).toBe(false);
  });

  it("title 说明计费口径与折扣公式", () => {
    const w = mount(BudgetPill, { props: { budget: budget() } });
    expect(w.attributes("title")).toContain("计费口径");
    expect(w.attributes("title")).toContain("1/10");
  });

  it("续期后：显示（已续期 i/n），上限为续期后的 effective_cap", () => {
    const w = mount(BudgetPill, {
      props: {
        budget: budget({
          renewal_index: 1, effective_cap: 1_200_000, cumulative_tokens: 650_000,
        }),
      },
    });
    expect(w.text()).toContain("65万");
    expect(w.text()).toContain("120万");
    expect(w.find(".renewed").text()).toContain("1/2");
  });

  it("title 提示区分硬上限与可续期语义", () => {
    const hard = mount(BudgetPill, {
      props: { budget: budget({ max_renewals: 0 }) },
    });
    expect(hard.attributes("title")).toContain("硬上限");
    const soft = mount(BudgetPill, { props: { budget: budget() } });
    expect(soft.attributes("title")).toContain("自动续期 2 次");
  });
});
