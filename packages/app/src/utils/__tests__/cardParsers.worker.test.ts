// Router 化批 P1-5：spawn_worker 卡片取数层的解析器回归锁
// （worker 与 delegate 字段族不同名——worker_conversation_id/worker_name）。
import { describe, it, expect } from "vitest";
import { parseSpawnWorkerInput, parseSpawnWorkerResult } from "../cardParsers";

describe("parseSpawnWorkerInput", () => {
  it("完整参数解析（purpose/task）", () => {
    const r = parseSpawnWorkerInput(JSON.stringify({ purpose: "统计文档", task: "数 md 文件" }));
    expect(r).toEqual({ purpose: "统计文档", task: "数 md 文件" });
  });
  it("缺 purpose 判非（流式参数未到齐 → null，卡降级通用行）", () => {
    expect(parseSpawnWorkerInput('{"task":"x"')).toBeNull();
    expect(parseSpawnWorkerInput('{"task":"x"}')).toBeNull();
  });
  it("task 缺省宽容（purpose 到即可出卡，task 空串）", () => {
    expect(parseSpawnWorkerInput('{"purpose":"p"}')).toEqual({ purpose: "p", task: "" });
  });
});

describe("parseSpawnWorkerResult", () => {
  it("完整结果解析（批 P1-5 起 tool_result 带 worker_name 供卡显示）", () => {
    const r = parseSpawnWorkerResult(
      JSON.stringify({
        worker_conversation_id: "c-1",
        worker_agent_id: "w-1",
        worker_name: "临时·统计文档·a9a1",
        finish_reason: "end_turn",
        rounds: 3,
      }),
    );
    expect(r).toEqual({
      childConvId: "c-1",
      workerName: "临时·统计文档·a9a1",
      finishReason: "end_turn",
      rounds: 3,
    });
  });
  it("Err 路径 content 非本族 JSON → null（状态走 isError）", () => {
    expect(parseSpawnWorkerResult("参数校验失败: …")).toBeNull();
    // delegate 字段族不误判（child_conversation_id 不是 worker 键）
    expect(parseSpawnWorkerResult('{"child_conversation_id":"c-2","agent_name":"x"}')).toBeNull();
  });
});
