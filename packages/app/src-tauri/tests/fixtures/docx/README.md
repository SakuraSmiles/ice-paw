# docx 真实语料（🚫 严禁版本控制 / 上传）

本目录放三份真实工程文档语料（代号 SDP / SRS / INSTALL），供
`src/harness/doc/corpus_tests.rs` 运行时读取。

- **文件不入库**：`.gitignore` 已排除 `*.docx`；语料含真实单位/项目信息，
  禁止 commit / push / 任何形式上传（用户拍板 2026-08-24，详见
  docs/word-capability-roadmap.md 决策 D7）。
- **内容字符串不进代码**：任何来自语料的文本（文档标题/正文词/样式名）
  不得写进代码、注释或文档；测试断言只用结构性锚点（规模/块号/首行派生
  关系/golden 逐字节对比）。
- **缺失自动 skip**：文件不在时 corpus 系列测试打印 skip 说明后跳过，
  CI / 无语料机器不失败；本机放置后自动生效。
- 文件名约定（corpus_tests 读取）：`sdp.docx` / `srs.docx` / `install.docx`
  （源目录 `D:\wcb\test`）。
