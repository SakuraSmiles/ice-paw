// toolSummary.test.ts — 工具行级摘要解析器（展示名/次级信息/文件名位/降级路径）
import { describe, it, expect } from "vitest";
import {
  summarizeToolCall,
  basenameOf,
  dirnameOf,
} from "../toolSummary";
import { toolDisplayName, TOOL_LABELS } from "../toolLabels";

describe("basenameOf / dirnameOf", () => {
  it("Windows 与 Unix 分隔符混用都取末段", () => {
    expect(basenameOf("D:\\ws\\报告.docx")).toBe("报告.docx");
    expect(basenameOf("/usr/local/a.txt")).toBe("a.txt");
    expect(basenameOf("src\\lib/util.rs")).toBe("util.rs");
    expect(basenameOf("裸名.md")).toBe("裸名.md");
  });

  it("dirname 含尾分隔符（openPath 可直接用）；无分隔符为空串", () => {
    expect(dirnameOf("D:\\ws\\报告.docx")).toBe("D:\\ws\\");
    expect(dirnameOf("/usr/local/a.txt")).toBe("/usr/local/");
    expect(dirnameOf("裸名.md")).toBe("");
  });
});

describe("展示名词表", () => {
  it("结构不变式：40 项、值非空、值唯一（仿 stylePresets 结构测试范式）", () => {
    const entries = Object.entries(TOOL_LABELS);
    expect(entries.length).toBe(40);
    for (const [, label] of entries) {
      expect(label.trim().length).toBeGreaterThan(0);
    }
    const labels = new Set(entries.map(([, l]) => l));
    expect(labels.size).toBe(entries.length);
  });

  it("词表外裸透英文原值（降级不猜）", () => {
    expect(toolDisplayName("some_external_tool")).toBe("some_external_tool");
    expect(toolDisplayName("t99_custom")).toBe("t99_custom");
    expect(toolDisplayName("write_file")).toBe("写入文件");
    expect(toolDisplayName("request_screen_session")).toBe("请求屏幕共享");
  });
});

describe("summarizeToolCall：降级路径", () => {
  it("非 P1 工具返回 null（回退通用行）", () => {
    expect(summarizeToolCall("search_kb", '{"query":"x"}', null)).toBeNull();
    expect(summarizeToolCall("delegate_to_agent", "{}", null)).toBeNull();
  });

  it("参数非 JSON（流式未到齐）返回 null", () => {
    expect(summarizeToolCall("write_file", '{"path":"D:/a', null)).toBeNull();
  });

  it("path 字段缺失/非字符串返回 null（畸形参数不硬造文件名）", () => {
    expect(summarizeToolCall("write_file", '{"content":"x"}', null)).toBeNull();
    expect(summarizeToolCall("write_file", '{"path":42}', null)).toBeNull();
  });
});

describe("summarizeToolCall：各工具摘要", () => {
  it("write_file：backup null=新建 / 有值=覆盖，附字节数", () => {
    const args = JSON.stringify({ path: "D:/ws/report.md", content: "hi" });
    const fresh = summarizeToolCall("write_file", args, {
      content: '{"path":"D:/ws/report.md","bytes_written":2,"backup":null}', isError: false,
    });
    expect(fresh?.display).toBe("写入文件");
    expect(fresh?.fileLabel).toBe("report.md");
    expect(fresh?.secondary).toBe("新建 · 2 B");
    expect(fresh?.revealPath).toBe("D:/ws/report.md");

    const over = summarizeToolCall("write_file", args, {
      content: '{"bytes_written":2,"backup":"D:/ws/.icepaw-backup/x"}', isError: false,
    });
    expect(over?.secondary).toBe("覆盖 · 2 B");
  });

  it("edit_file：replacements 计数", () => {
    const args = JSON.stringify({ path: "main.rs", old_string: "a", new_string: "b" });
    const s = summarizeToolCall("edit_file", args, {
      content: '{"replacements":3}', isError: false,
    });
    expect(s?.secondary).toBe("替换 3 处");
    expect(s?.fileLabel).toBe("main.rs");
  });

  it("delete_file：文件删除带备份标注 / 目录删除无", () => {
    const args = JSON.stringify({ path: "D:/tmp/cache.txt" });
    expect(summarizeToolCall("delete_file", args, {
      content: '{"backup":"D:/tmp/.icepaw-backup/c"}', isError: false,
    })?.secondary).toBe("已删除 · 已备份");
    expect(summarizeToolCall("delete_file", args, {
      content: '{"backup":null}', isError: false,
    })?.secondary).toBe("已删除");
  });

  it("move_file：纯移动（同名）只显目标名；改名显 旧 → 新；reveal 目标", () => {
    const moved = summarizeToolCall("move_file",
      JSON.stringify({ source: "D:/a.rs", destination: "D:/src/a.rs" }), null);
    expect(moved?.fileLabel).toBe("a.rs");
    expect(moved?.fileTitle).toBe("D:/a.rs → D:/src/a.rs");
    expect(moved?.revealPath).toBe("D:/src/a.rs");
    expect(moved?.secondary).toBe("");

    const renamed = summarizeToolCall("move_file",
      JSON.stringify({ source: "D:/draft.md", destination: "D:/final.md" }), null);
    expect(renamed?.fileLabel).toBe("draft.md → final.md");
  });

  it("copy_file：覆盖时次级信息「覆盖」；新建目标无", () => {
    const args = JSON.stringify({ source: "D:/a.rs", destination: "D:/dist/a.rs" });
    expect(summarizeToolCall("copy_file", args, {
      content: '{"backup":"D:/dist/.icepaw-backup/old"}', isError: false,
    })?.secondary).toBe("覆盖");
    expect(summarizeToolCall("copy_file", args, {
      content: '{"backup":null}', isError: false,
    })?.secondary).toBe("");
  });

  it("read_file：字节数 + 行数（结果非 JSON 时宽容降级为空）", () => {
    const args = JSON.stringify({ path: "config.yaml" });
    expect(summarizeToolCall("read_file", args, {
      content: '{"size":8400,"total_lines":210}', isError: false,
    })?.secondary).toBe("8.2 KB · 210 行");
    // 纯文本结果（如测试工厂/旧形态）：parse 失败 → 次级信息空，文件名位照常
    expect(summarizeToolCall("read_file", args, {
      content: "文件内容", isError: false,
    })?.secondary).toBe("");
  });

  it("inspect_docx：投影中文词表 + 块数（词表外裸透原值）", () => {
    const args = JSON.stringify({ path: "D:/报告.docx", projection: "outline" });
    expect(summarizeToolCall("inspect_docx", args, {
      content: '{"projection":"outline","total_blocks":42}', isError: false,
    })?.secondary).toBe("大纲 · 42 块");
    expect(summarizeToolCall("inspect_docx", args, {
      content: '{"projection":"unknown_proj","total_blocks":1}', isError: false,
    })?.secondary).toBe("unknown_proj · 1 块");
  });

  it("edit_docx / write_docx / validate_docx 计数", () => {
    expect(summarizeToolCall("edit_docx", JSON.stringify({ path: "a.docx", operations: [] }), {
      content: '{"applied":5}', isError: false,
    })?.secondary).toBe("5 处操作");

    // write_docx 新建判据 = created 字段（backup 缺省形态）
    const w = summarizeToolCall("write_docx", JSON.stringify({ path: "a.docx", blocks: [] }), {
      content: '{"created":true,"blocks":18}', isError: false,
    });
    expect(w?.secondary).toBe("新建 · 18 块");
    const w2 = summarizeToolCall("write_docx", JSON.stringify({ path: "a.docx", blocks: [] }), {
      content: '{"created":false,"blocks":18}', isError: false,
    });
    expect(w2?.secondary).toBe("18 块");

    expect(summarizeToolCall("validate_docx", JSON.stringify({ path: "a.docx", assertions: [] }), {
      content: '{"passed":true,"total":6,"failed":0}', isError: false,
    })?.secondary).toBe("6 项 · 全部通过");
    expect(summarizeToolCall("validate_docx", JSON.stringify({ path: "a.docx", assertions: [] }), {
      content: '{"passed":false,"total":6,"failed":2}', isError: false,
    })?.secondary).toBe("6 项 · 2 项未过");
  });
});

describe("summarizeToolCall：执行中与错误态", () => {
  it("result 未到达（执行中）：文件名位先出、次级信息空", () => {
    const s = summarizeToolCall("write_file", JSON.stringify({ path: "D:/x.md", content: "y" }), null);
    expect(s?.fileLabel).toBe("x.md");
    expect(s?.secondary).toBe("");
  });

  it("错误态：次级信息「调用失败」，文件名位照常（文件可能没动过仍可点开看）", () => {
    const s = summarizeToolCall("edit_file",
      JSON.stringify({ path: "main.rs", old_string: "a", new_string: "b" }),
      { content: "edit_file: old_string 不唯一…", isError: true });
    expect(s?.secondary).toBe("调用失败");
    expect(s?.fileLabel).toBe("main.rs");
  });
});
