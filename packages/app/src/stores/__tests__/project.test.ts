import { describe, it, expect, beforeEach, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useProjectStore } from "../project";
import { invoke } from "@tauri-apps/api/core";

const mockInvoke = vi.mocked(invoke);

// 构造一个最小 Project 列表项
function mockProject(id: string, name: string, archived = false) {
  return {
    id,
    name,
    description: "",
    icon: "folder",
    sort_order: 0,
    workspace_path: null,
    theme_color: null,
    archived,
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
    agents: [],
  };
}

describe("projectStore", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    mockInvoke.mockReset();
  });

  describe("load", () => {
    it("fetches projects and sets loaded=true", async () => {
      mockInvoke.mockResolvedValueOnce([mockProject("p1", "项目A")]);

      const store = useProjectStore();
      await store.load();

      expect(store.list).toHaveLength(1);
      expect(store.list[0].name).toBe("项目A");
      expect(store.loaded).toBe(true);
    });

    it("does not refetch if already loaded", async () => {
      mockInvoke.mockResolvedValueOnce([mockProject("p1", "A")]);

      const store = useProjectStore();
      await store.load();
      await store.load(); // 第二次调用——应该被 loaded 守卫跳过

      // invoke 只被调用一次（第二次 load 被跳过）
      expect(mockInvoke).toHaveBeenCalledTimes(1);
    });

    it("concurrent loads share the in-flight promise (no duplicate invoke)", async () => {
      // 第一次调用 pending，第二次共享同一个 Promise 等待（不再「跳过即返回」）
      let resolveFirst: (v: unknown) => void;
      const firstPromise = new Promise((r) => { resolveFirst = r; });
      mockInvoke.mockReturnValueOnce(firstPromise);

      const store = useProjectStore();
      const p1 = store.load(true);
      const p2 = store.load(true);

      resolveFirst!([mockProject("p1", "A")]);
      await p1;
      await p2;

      expect(mockInvoke).toHaveBeenCalledTimes(1);
      // 关键：后到的调用方等到数据落地（刷新时详情页 load(true) 不再误判空列表）
      expect(store.loaded).toBe(true);
      expect(store.getById("p1")?.name).toBe("A");
    });

    it("refresh race: detail page load(true) awaits sidebar's in-flight load and sees data", async () => {
      // 复刻刷新时序：Sidebar load() 先发起（pending）→ 详情页 onMounted
      // load(true)。旧实现 force 被「loading 中 return」短路立即返回，
      // getById 仍 null → 误报「项目不存在或已删除」。
      let resolveFirst: (v: unknown) => void;
      const firstPromise = new Promise((r) => { resolveFirst = r; });
      mockInvoke.mockReturnValueOnce(firstPromise);

      const store = useProjectStore();
      const sidebarLoad = store.load();      // Sidebar onMounted
      const detailLoad = store.load(true);   // 详情页 onMounted（force）

      // 详情页的检查在 await 之后才发生
      resolveFirst!([mockProject("p1", "真实项目")]);
      await detailLoad;
      expect(store.getById("p1")?.name).toBe("真实项目");
      await sidebarLoad;
    });
  });

  describe("setActiveProject", () => {
    it("accepts null", () => {
      const store = useProjectStore();
      store.setActiveProject(null);
      expect(store.activeProjectId).toBeNull();
    });

    it("rejects invalid ID when list is loaded", async () => {
      mockInvoke.mockResolvedValueOnce([mockProject("p1", "A")]);

      const store = useProjectStore();
      await store.load();

      const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
      store.setActiveProject("nonexistent");
      expect(store.activeProjectId).toBeNull();

      warnSpy.mockRestore();
    });

    it("accepts valid ID when list is loaded", async () => {
      mockInvoke.mockResolvedValueOnce([mockProject("p1", "A")]);

      const store = useProjectStore();
      await store.load();

      store.setActiveProject("p1");
      expect(store.activeProjectId).toBe("p1");
    });

    it("optimistically sets ID before load completes", () => {
      const store = useProjectStore();
      // loaded=false, list=[]
      store.setActiveProject("future-id");
      expect(store.activeProjectId).toBe("future-id");
      // load() 完成后会自动校验并清空无效 ID
    });
  });

  describe("create", () => {
    it("calls bridge and reloads list", async () => {
      mockInvoke
        .mockResolvedValueOnce({ id: "new-id", name: "新项目" }) // create 返回
        .mockResolvedValueOnce([mockProject("new-id", "新项目")]); // load 返回

      const store = useProjectStore();
      await store.create({ name: "新项目" });

      expect(store.list).toHaveLength(1);
      expect(store.list[0].id).toBe("new-id");
    });
  });

  describe("context（project.md 项目上下文）", () => {
    const mockCtx = {
      available: true,
      dir: "C:/ws/projects/p1",
      project_md: "# 说明",
      conventions_md: "",
    };

    it("loadContext fetches and caches by pid", async () => {
      mockInvoke.mockResolvedValueOnce(mockCtx);

      const store = useProjectStore();
      await store.loadContext("p1");

      expect(mockInvoke).toHaveBeenCalledWith("get_project_context", { projectId: "p1" });
      expect(store.context?.pid).toBe("p1");
      expect(store.context?.project_md).toBe("# 说明");

      // 同 pid 再拉 → 走缓存不再 invoke
      await store.loadContext("p1");
      expect(mockInvoke).toHaveBeenCalledTimes(1);
    });

    it("loadContext force=true bypasses cache; different pid refetches", async () => {
      mockInvoke
        .mockResolvedValueOnce(mockCtx)
        .mockResolvedValueOnce({ ...mockCtx, project_md: "# 外部改过" });

      const store = useProjectStore();
      await store.loadContext("p1");
      await store.loadContext("p1", true); // force 绕过缓存（编辑区展开防陈旧）
      await store.loadContext("p2", true);

      expect(mockInvoke).toHaveBeenCalledTimes(3);
      expect(store.context?.pid).toBe("p2");
    });

    it("saveContext writes file and syncs cache", async () => {
      mockInvoke.mockResolvedValueOnce(mockCtx);

      const store = useProjectStore();
      await store.loadContext("p1");

      mockInvoke.mockClear();
      await store.saveContext("p1", "project.md", "# 新内容");

      expect(mockInvoke).toHaveBeenCalledWith("set_project_context", {
        projectId: "p1",
        file: "project.md",
        content: "# 新内容",
      });
      // 缓存同步（保存后不再拉一次也能反映新内容）
      expect(store.context?.project_md).toBe("# 新内容");
      expect(store.context?.conventions_md).toBe("");
    });

    it("saveContext does not touch cache of other pid", async () => {
      mockInvoke.mockResolvedValueOnce(mockCtx);
      const store = useProjectStore();
      await store.loadContext("p1");

      await store.saveContext("other", "project.md", "x");
      expect(store.context?.pid).toBe("p1");
      expect(store.context?.project_md).toBe("# 说明");
    });
  });
});
