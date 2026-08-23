// stylePresets.test.ts — 风格预设常量与两个纯helper的形状锁定
// （素材本身是文案，不逐字断言；断结构不变式：三档 id 唯一 / {name} 全替换 /
// 出生默认句判定与后端 agent_cmd.rs 同步）。
import { describe, it, expect } from "vitest";
import { STYLE_PRESETS, fillPresetName, isBirthDefaultPrompt } from "../stylePresets";

describe("STYLE_PRESETS", () => {
  it("三档且 id 唯一", () => {
    expect(STYLE_PRESETS).toHaveLength(3);
    expect(new Set(STYLE_PRESETS.map((p) => p.id)).size).toBe(3);
  });

  it("每档带适用说明与非空文本（多行）", () => {
    for (const p of STYLE_PRESETS) {
      expect(p.name).toBeTruthy();
      expect(p.note).toBeTruthy();
      expect(p.text.trim().length).toBeGreaterThan(20);
      expect(p.text.split("\n").length).toBeGreaterThan(3);
    }
  });

  it("工程档含意图确认条（2026-08-23 从平台层下沉），三档不含平台纪律文案", () => {
    const eng = STYLE_PRESETS.find((p) => p.id === "engineering")!;
    expect(eng.text).toContain("先确认再动手");
    // 平台层纪律（system_prompt.rs）不进素材——DRY
    for (const p of STYLE_PRESETS) {
      expect(p.text).not.toContain("与你的人设叠加生效");
      expect(p.text).not.toContain("不要编造");
    }
  });
});

describe("fillPresetName", () => {
  it("替换全部 {name} 占位", () => {
    const out = fillPresetName("你是{name}。\n{name} 在场。", "小冰");
    expect(out).toBe("你是小冰。\n小冰 在场。");
  });

  it("名称为空退「AI 助手」；首尾空白剥掉", () => {
    expect(fillPresetName("你是{name}。", "  ")).toBe("你是AI 助手。");
    expect(fillPresetName("你是{name}。", " 小冰 ")).toBe("你是小冰。");
  });
});

describe("isBirthDefaultPrompt", () => {
  it("出生句判定（与 agent_cmd.rs 默认句同步）", () => {
    expect(isBirthDefaultPrompt("小冰 是一个 AI 助手。", "小冰")).toBe(true);
    // 尾换行容忍（| 块解析值恒带尾换行）
    expect(isBirthDefaultPrompt("小冰 是一个 AI 助手。\n", "小冰")).toBe(true);
    expect(isBirthDefaultPrompt("你是小冰，一名工程助手。", "小冰")).toBe(false);
  });
});
