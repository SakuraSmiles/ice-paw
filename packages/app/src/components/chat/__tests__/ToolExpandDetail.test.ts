// ToolExpandDetail.test.ts — 工具展开详情组件（分派渲染 + 原始 JSON 兜底 + 错误态）
import { describe, it, expect } from "vitest";
import { mount } from "@vue/test-utils";
import ToolExpandDetail from "../ToolExpandDetail.vue";

interface ResultLike {
  content: string;
  isError: boolean;
}

function mountDetail(name: string, argsJson: string, result: ResultLike | null) {
  return mount(ToolExpandDetail, {
    props: { name, argsJson, result },
  });
}

describe("edit_file：红绿对照", () => {
  const args = JSON.stringify({ path: "main.rs", old_string: "let a = 1;", new_string: "let a = 2;" });

  it("替换前/后红绿两块 + 替换计数", () => {
    const w = mountDetail("edit_file", args, {
      content: '{"replacements":1}', isError: false,
    });
    const del = w.find(".diff-del");
    const add = w.find(".diff-add");
    expect(del.exists()).toBe(true);
    expect(add.exists()).toBe(true);
    expect(del.text()).toContain("let a = 1;");
    expect(add.text()).toContain("let a = 2;");
    expect(w.text()).toContain("替换 1 处");
  });
});

describe("write_docx：块清单", () => {
  it("新建 + 块数 + 分类计数 + 模板", () => {
    const w = mountDetail("write_docx", JSON.stringify({ path: "a.docx", template: "formal-report.docx", blocks: [] }), {
      content: '{"created":true,"blocks":18,"paragraphs":12,"tables":3,"images":2,"tocs":1,"bytes":20480,"check":"passed"}',
      isError: false,
    });
    expect(w.text()).toContain("新建");
    expect(w.text()).toContain("18 块");
    expect(w.text()).toContain("段落 12 · 表格 3 · 图片 2 · 目录 1");
    expect(w.text()).toContain("formal-report.docx");
  });
});

describe("validate_docx：断言清单", () => {
  const args = JSON.stringify({ path: "a.docx", assertions: [] });

  it("全过：全部通过 + 通过聚合", () => {
    const w = mountDetail("validate_docx", args, {
      content: '{"passed":true,"total":4,"failed":0,"failures":[],"passed_kinds":["block_text×3","table_shape×1"]}',
      isError: false,
    });
    expect(w.text()).toContain("全部通过（4 项）");
    expect(w.text()).toContain("block_text×3");
    expect(w.findAll(".op-fail").length).toBe(0);
  });

  it("有失败：失败明细红行（kind 中文 + target + detail）", () => {
    const w = mountDetail("validate_docx", args, {
      content: '{"passed":false,"total":2,"failed":1,"failures":[{"kind":"cell_text","target":"块12 r3c2","detail":"期望「合计」，实际「总记」"}],"passed_kinds":["block_count×1"]}',
      isError: false,
    });
    const fails = w.findAll(".op-fail");
    expect(fails.length).toBe(1);
    expect(fails[0].text()).toContain("格文本");
    expect(fails[0].text()).toContain("块12 r3c2");
    expect(w.text()).toContain("1 项未过");
  });
});

describe("edit_docx：逐操作清单", () => {
  it("AppliedOp 逐条（op 中文标签 + 块号 + before → after）", () => {
    const w = mountDetail("edit_docx", JSON.stringify({ path: "a.docx", operations: [] }), {
      content: '{"applied":2,"operations":[{"op":"replace_text","block":12,"before":"旧标题","after":"新标题"},{"op":"set_format","block":13,"before":"","after":"加粗"}]}',
      isError: false,
    });
    const rows = w.findAll(".op-row");
    expect(rows.length).toBe(2);
    expect(rows[0].text()).toContain("替换文本");
    expect(rows[0].text()).toContain("块 12");
    expect(rows[0].text()).toContain("旧标题 → 新标题");
    expect(rows[1].text()).toContain("设格式");
  });
});

describe("兜底与降级", () => {
  it("非 P1 工具：参数 + 结果两块（现状形态）", () => {
    const w = mountDetail("search_kb", '{"query":"预算"}', {
      content: '{"hits":[]}', isError: false,
    });
    expect(w.find(".diff-del").exists()).toBe(false);
    expect(w.text()).toContain("参数");
    expect(w.text()).toContain('{"hits":[]}');
  });

  it("result 为 null（执行中）：兜底工具显等待；结构化工具（write_file）显内容预览", () => {
    const w = mountDetail("write_file", JSON.stringify({ path: "x.md", content: "y" }), null);
    expect(w.text()).toContain("写入内容");
    expect(w.text()).toContain("y");

    const generic = mountDetail("search_kb", '{"query":"预算"}', null);
    expect(generic.text()).toContain("等待执行结果");
  });

  it("错误态：参数 + 错误块，结构化分支不渲染", () => {
    const w = mountDetail("edit_file",
      JSON.stringify({ path: "main.rs", old_string: "a", new_string: "b" }),
      { content: "edit_file: old_string 在 main.rs 中出现 3 次，不唯一", isError: true });
    expect(w.find(".diff-del").exists()).toBe(false);
    expect(w.find(".hdr-err").exists()).toBe(true);
    expect(w.text()).toContain("出现 3 次");
  });
});

describe("原始 JSON 折叠（透明度兜底）", () => {
  it("默认收起，点击展开参数/结果原文", async () => {
    const w = mountDetail("write_file", '{"path":"x.md","content":"y"}', {
      content: '{"bytes_written":1}', isError: false,
    });
    // 收起态：原始键名不出现在渲染文本中（结构化分支只显示内容值）
    const btn = w.find(".raw-toggle");
    expect(btn.exists()).toBe(true);
    await btn.trigger("click");
    expect(w.text()).toContain("参数（原始）");
    expect(w.text()).toContain('{"path":"x.md","content":"y"}');
    expect(w.text()).toContain('{"bytes_written":1}');
  });
});
