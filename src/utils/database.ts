// 数据库工具封装：单例 Database 实例
// 注意：@tauri-apps/plugin-sql 仅在 Tauri 原生窗口中可用，
// 在纯浏览器中调用会抛出错误，请使用 try-catch 包裹。

import Database from "@tauri-apps/plugin-sql";

/**
 * 数据库工具类
 * - init()      初始化连接（懒加载）
 * - execute()   执行写操作 SQL
 * - select()    执行查询 SQL 并返回结果数组
 * - close()     关闭连接
 *
 * 整个应用通过 getDatabase() 共享同一实例，避免重复打开。
 */
class DatabaseManager {
  private db: Database | null = null;
  private currentPath: string | null = null;

  /**
   * 初始化数据库连接
   * @param path sqlite 连接串，例如 "sqlite:icepaw.db"
   */
  async init(path: string = "sqlite:icepaw.db"): Promise<Database> {
    // 如果已经初始化为相同路径，则直接复用
    if (this.db && this.currentPath === path) {
      return this.db;
    }
    // 如果切换了路径，先关闭旧连接
    if (this.db && this.currentPath !== path) {
      await this.close();
    }
    const db = await Database.load(path);
    this.db = db;
    this.currentPath = path;
    return db;
  }

  /**
   * 执行写操作 SQL（INSERT / UPDATE / DELETE 等）
   * @returns 受影响行数等信息
   */
  async execute(sql: string, bind?: unknown[]): Promise<{ rowsAffected: number; lastInsertId?: number }> {
    const db = await this.ensureDb();
    return await db.execute(sql, bind);
  }

  /**
   * 执行查询 SQL 并返回结果数组
   * 注意：@tauri-apps/plugin-sql 的 select<T> 泛型期望 T 本身为数组类型（单行类型 + 数组），
   * 这里我们将「行类型 R」转换为内部调用的「数组类型 R[]」，对调用方更友好。
   */
  async select<R = Record<string, unknown>>(sql: string, bind?: unknown[]): Promise<R[]> {
    const db = await this.ensureDb();
    // 内部调用时把 R[] 作为泛型传入，与原生签名保持一致
    return (await db.select<R[]>(sql, bind)) as R[];
  }

  /**
   * 关闭数据库连接
   */
  async close(): Promise<boolean> {
    if (!this.db) {
      return true;
    }
    const ok = await this.db.close();
    this.db = null;
    this.currentPath = null;
    return ok;
  }

  /**
   * 获取当前已初始化的 Database 实例（若未初始化则抛错）
   */
  private async ensureDb(): Promise<Database> {
    if (!this.db) {
      throw new Error("数据库尚未初始化，请先调用 database.init()");
    }
    return this.db;
  }
}

// 单例：整个应用共享一个数据库管理器
const database = new DatabaseManager();

/** 获取全局共享的数据库管理器 */
export function getDatabase(): DatabaseManager {
  return database;
}

export default database;