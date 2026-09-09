// missHint — slug 映射 / shortMissHint 弱展示规则 / missHintTitle 披露
import { describe, expect, it } from "vitest";
import { MISS_HINT_LABELS, missHintTitle, shortMissHint } from "../missHint";

// 与后端 turn_cost::miss_slug 词表镜像（7 slug；改动两边同步）
const ALL_SLUGS = [
  "first_request",
  "tools_changed",
  "system_stable_changed",
  "os_env_changed",
  "injection_changed",
  "model_switched",
  "no_detectable_change",
];

describe("missHint", () => {
  it("词表全覆盖：7 slug 各有中文短标签", () => {
    expect(Object.keys(MISS_HINT_LABELS).sort()).toEqual([...ALL_SLUGS].sort());
    for (const slug of ALL_SLUGS) {
      expect(MISS_HINT_LABELS[slug].length).toBeGreaterThan(0);
    }
  });

  it("shortMissHint：取首个非 first_request 因", () => {
    expect(shortMissHint(["tools_changed", "injection_changed"])).toBe("工具列表变化");
    expect(shortMissHint(["model_switched"])).toBe("模型换档");
    // first_request 排最前也跳过（互斥场景不出现，但防御性取后者）
    expect(shortMissHint(["first_request", "os_env_changed"])).toBe("运行环境变化");
  });

  it("shortMissHint：仅 first_request / 空 / null / 未知 slug → null", () => {
    expect(shortMissHint(["first_request"])).toBeNull();
    expect(shortMissHint([])).toBeNull();
    expect(shortMissHint(null)).toBeNull();
    expect(shortMissHint(undefined)).toBeNull();
    // 未知 slug 不透出原词（后端新增 slug 前端未跟时的诚实缺席，title 仍透出）
    expect(shortMissHint(["future_slug"])).toBeNull();
  });

  it("missHintTitle：全因 + 机理 + 推断披露三段", () => {
    const title = missHintTitle(["no_detectable_change"]);
    expect(title).toContain("全未命中缓存");
    expect(title).toContain("TTL 过期");
    expect(title).toContain("本地推断");
    expect(title).toContain("非 provider 报告");
  });

  it("missHintTitle：多因并列各带机理", () => {
    const title = missHintTitle(["tools_changed", "injection_changed"]);
    expect(title).toContain("工具列表变化");
    expect(title).toContain("相关性裁剪");
    expect(title).toContain("注入变化");
    expect(title).toContain("翻转");
  });

  it("missHintTitle：空输入返回空串（title 属性缺席）", () => {
    expect(missHintTitle(null)).toBe("");
    expect(missHintTitle([])).toBe("");
  });

  it("missHintTitle：未知 slug 原样透出（不吞因素）", () => {
    expect(missHintTitle(["future_slug"])).toContain("future_slug");
  });
});
