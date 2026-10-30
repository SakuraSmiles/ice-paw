use super::*;

/// docx-rs 造最小真实包（zip 容器 + document.xml）。
fn docx_bytes() -> Vec<u8> {
    use docx_rs::{Document, Docx, Paragraph, Run};
    let document =
        Document::new().add_paragraph(Paragraph::new().add_run(Run::new().add_text("正文段")));
    let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
    Docx::new()
        .document(document)
        .build()
        .pack(&mut cursor)
        .unwrap();
    cursor.into_inner()
}

#[tokio::test]
async fn happy_path_outline() {
    let dir = std::env::temp_dir().join("icepaw_inspect_docx_test");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("样本.docx");
    std::fs::write(&file, docx_bytes()).unwrap();

    let tool = InspectDocxTool;
    let args = serde_json::json!({ "path": file.to_string_lossy() }).to_string();
    let out = tool.execute(&args).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["projection"], "outline");
    assert_eq!(v["total_blocks"], 1);
    assert!(v["content"].as_str().unwrap().contains("正文段"));
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn rejects_non_docx_and_bad_projection() {
    let tool = InspectDocxTool;
    // 非 docx 扩展名
    let dir = std::env::temp_dir().join("icepaw_inspect_docx_test2");
    std::fs::create_dir_all(&dir).unwrap();
    let txt = dir.join("note.txt");
    std::fs::write(&txt, b"hello").unwrap();
    let args = serde_json::json!({ "path": txt.to_string_lossy() }).to_string();
    let err = tool.execute(&args).await.unwrap_err().to_string();
    assert!(err.contains("read_file"), "应指向 read_file: {err}");
    // 未知投影
    let args = serde_json::json!({
        "path": txt.to_string_lossy(),
        "projection": "deep"
    })
    .to_string();
    let err = tool.execute(&args).await.unwrap_err().to_string();
    assert!(err.contains("未知投影档位"), "实际: {err}");
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn missing_file_gives_suggestions() {
    let tool = InspectDocxTool;
    let args = serde_json::json!({ "path": "Z:/不存在的/文档.docx" }).to_string();
    let err = tool.execute(&args).await.unwrap_err().to_string();
    assert!(err.contains("文件不存在"), "实际: {err}");
}

// ---- edit_docx ----

/// 三段正文的最小真实包。
fn three_para_docx() -> Vec<u8> {
    use docx_rs::{Document, Docx, Paragraph, Run};
    let document = Document::new()
        .add_paragraph(Paragraph::new().add_run(Run::new().add_text("第一段")))
        .add_paragraph(Paragraph::new().add_run(Run::new().add_text("第二段")))
        .add_paragraph(Paragraph::new().add_run(Run::new().add_text("第三段")));
    let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
    Docx::new()
        .document(document)
        .build()
        .pack(&mut cursor)
        .unwrap();
    cursor.into_inner()
}

#[tokio::test]
async fn edit_batch_applies_and_backs_up() {
    let dir = std::env::temp_dir().join("icepaw_edit_docx_test");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("样本.docx");
    std::fs::write(&file, three_para_docx()).unwrap();

    let tool = EditDocxTool;
    let args = serde_json::json!({
            "path": file.to_string_lossy(),
            "operations": [
                { "op": "replace_text", "block": 2, "expect_prefix": "第二段", "new_text": "改写段" },
                { "op": "insert_paragraph_after", "block": 1, "expect_prefix": "第一段", "text": "新插段" },
                { "op": "delete_block", "block": 3, "expect_prefix": "第三段" }
            ]
        })
        .to_string();
    let out = tool.execute(&args).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["applied"], 3);
    assert!(v["backup"].as_str().is_some(), "应已备份: {out}");
    assert_eq!(v["operations"].as_array().unwrap().len(), 3);

    // 读回验证：全文 = 第一段/新插段/改写段（第三段已删）
    let bytes = std::fs::read(&file).unwrap();
    let inspect = crate::harness::doc::inspect_document(
        &bytes,
        &InspectRequest {
            projection: InspectProjection::Text,
            start: None,
            end: None,
            row: None,
            cell: None,
            style: None,
            num_id: None,
            level: None,
        },
    )
    .unwrap();
    assert!(inspect.content.contains("新插段"));
    assert!(inspect.content.contains("改写段"));
    assert!(!inspect.content.contains("第三段"));
    assert_eq!(inspect.total_blocks, 3, "3 块 +1 插 -1 删");

    // 备份内容 = 原始字节
    let backup_path = v["backup"].as_str().unwrap();
    assert_eq!(std::fs::read(backup_path).unwrap(), three_para_docx());
    // tmp 不残留
    assert!(!file.with_file_name("样本.docx.icepaw-tmp").exists());
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn fingerprint_mismatch_leaves_file_untouched() {
    let dir = std::env::temp_dir().join("icepaw_edit_docx_test2");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("样本.docx");
    std::fs::write(&file, three_para_docx()).unwrap();
    let before = std::fs::read(&file).unwrap();

    let tool = EditDocxTool;
    let args = serde_json::json!({
        "path": file.to_string_lossy(),
        "operations": [
            { "op": "replace_text", "block": 1, "expect_prefix": "第一段", "new_text": "x" },
            { "op": "delete_block", "block": 2, "expect_prefix": "过时指纹" }
        ]
    })
    .to_string();
    let err = tool.execute(&args).await.unwrap_err().to_string();
    assert!(err.contains("指纹不符"), "实际: {err}");
    // 全有或全无：合法的第 1 条也未生效，文件字节原样
    assert_eq!(std::fs::read(&file).unwrap(), before);
    // 无备份产生（未动刀）
    assert!(!dir.join(".icepaw-backup").exists());
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn edit_rejects_non_docx_and_missing() {
    let tool = EditDocxTool;
    let dir = std::env::temp_dir().join("icepaw_edit_docx_test3");
    std::fs::create_dir_all(&dir).unwrap();
    let txt = dir.join("note.txt");
    std::fs::write(&txt, b"hello").unwrap();
    let args = serde_json::json!({
        "path": txt.to_string_lossy(),
        "operations": [{ "op": "delete_block", "block": 1, "expect_prefix": "" }]
    })
    .to_string();
    let err = tool.execute(&args).await.unwrap_err().to_string();
    assert!(err.contains("read_file"), "应指向 read_file: {err}");

    let args = serde_json::json!({
        "path": "Z:/不存在的/文档.docx",
        "operations": [{ "op": "delete_block", "block": 1, "expect_prefix": "" }]
    })
    .to_string();
    let err = tool.execute(&args).await.unwrap_err().to_string();
    assert!(err.contains("文件不存在"), "实际: {err}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn edit_auth_level_is_path_whitelist() {
    assert_eq!(
        EditDocxTool.authorization_level(),
        AuthorizationLevel::PathWhitelist
    );
}

/// 定义族走工具真入口：create→set 同批组合 + styledef 投影读回 + 部件互斥拒。
#[tokio::test]
async fn style_family_edits_and_exclusive_batches() {
    let dir = std::env::temp_dir().join("icepaw_edit_docx_def_test");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("样本.docx");
    std::fs::write(&file, three_para_docx()).unwrap();

    // styles 族 happy path：create 最小出生 → 同批 set 补字符格式（寻址放应用期）
    let tool = EditDocxTool;
    let args = serde_json::json!({
        "path": file.to_string_lossy(),
        "operations": [
            { "op": "create_style", "style_type": "paragraph", "name": "交付标题" },
            { "op": "set_style_element", "style": "交付标题", "container": "rPr",
              "element": "sz", "xml": "<w:sz w:val=\"32\"/>" }
        ]
    })
    .to_string();
    let out = tool.execute(&args).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["applied"], 2, "{out}");

    // styledef 投影经工具入口读回（定义手术的验证视图闭环）
    let args = serde_json::json!({
        "path": file.to_string_lossy(),
        "projection": "styledef",
        "style": "交付标题"
    })
    .to_string();
    let out = InspectDocxTool.execute(&args).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let content = v["content"].as_str().unwrap();
    assert!(content.contains("交付标题"), "{content}");
    assert!(content.contains(r#"<w:sz w:val="32"/>"#), "{content}");

    // 部件互斥：正文操作 + 定义操作混批拒，文件字节原样
    let before = std::fs::read(&file).unwrap();
    let args = serde_json::json!({
        "path": file.to_string_lossy(),
        "operations": [
            { "op": "replace_text", "block": 1, "expect_prefix": "第一段", "new_text": "x" },
            { "op": "create_style", "style_type": "paragraph", "name": "另一个样式" }
        ]
    })
    .to_string();
    let err = tool.execute(&args).await.unwrap_err().to_string();
    assert!(err.contains("部件互斥"), "实际: {err}");
    assert_eq!(std::fs::read(&file).unwrap(), before);
    std::fs::remove_dir_all(&dir).ok();
}

// ---- validate_docx（D15 八波①）----

#[tokio::test]
async fn validate_reports_failures_as_data_not_error() {
    let dir = std::env::temp_dir().join("icepaw_validate_docx_test");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("样本.docx");
    std::fs::write(&file, three_para_docx()).unwrap();

    let tool = ValidateDocxTool;
    // 混合断言：2 过 2 败——失败是正常输出（passed=false），不是工具 Err
    let args = serde_json::json!({
        "path": file.to_string_lossy(),
        "assertions": [
            { "kind": "block_count", "equals": 3 },
            { "kind": "block_text", "block": 2, "contains": "第二" },
            { "kind": "block_text", "block": 1, "equals": "错文本" },
            { "kind": "cell_text", "block": 1, "row": 1, "cell": 1, "contains": "x" }
        ]
    })
    .to_string();
    let out = tool.execute(&args).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["passed"], false, "{out}");
    assert_eq!(v["total"], 4);
    assert_eq!(v["failed"], 2);
    let kinds: Vec<&str> = v["failures"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["kind"].as_str().unwrap())
        .collect();
    // 独立评估不短路：两条失败都报（段落块打 cell 断言 → fail 指路，非 Err）
    assert_eq!(kinds, ["block_text", "cell_text"], "{out}");

    // 全过形态：failures 空、passed_kinds 摘要在场
    let args = serde_json::json!({
        "path": file.to_string_lossy(),
        "assertions": [
            { "kind": "block_count", "equals": 3 },
            { "kind": "block_style", "block": 1, "equals": "(无样式)" }
        ]
    })
    .to_string();
    let out = tool.execute(&args).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["passed"], true, "{out}");
    assert_eq!(v["failed"], 0);
    assert!(v["failures"].as_array().unwrap().is_empty());
    assert!(!v["passed_kinds"].as_array().unwrap().is_empty(), "{out}");
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn validate_rejects_over_limit_and_missing_file() {
    let dir = std::env::temp_dir().join("icepaw_validate_docx_test2");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("样本.docx");
    std::fs::write(&file, three_para_docx()).unwrap();

    let tool = ValidateDocxTool;
    // 断言数超限 = 参数错 → Err（区别于断言失败）
    let assertions: Vec<_> = (0..51)
        .map(|i| serde_json::json!({ "kind": "block_count", "equals": i }))
        .collect();
    let args =
        serde_json::json!({ "path": file.to_string_lossy(), "assertions": assertions }).to_string();
    let err = tool.execute(&args).await.unwrap_err().to_string();
    assert!(err.contains("断言数超限"), "实际: {err}");

    // 文件不存在：did-you-mean 契约同 inspect/edit
    let args = serde_json::json!({
        "path": "Z:/不存在的/文档.docx",
        "assertions": [{ "kind": "block_count", "equals": 1 }]
    })
    .to_string();
    let err = tool.execute(&args).await.unwrap_err().to_string();
    assert!(err.contains("文件不存在"), "实际: {err}");
    std::fs::remove_dir_all(&dir).ok();
}

// ---- allowed_blocks 区间锁（D15 八波②）----

#[tokio::test]
async fn range_lock_allows_in_range_and_rejects_out_of_range() {
    let dir = std::env::temp_dir().join("icepaw_edit_docx_lock_test");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("样本.docx");
    std::fs::write(&file, three_para_docx()).unwrap();

    let tool = EditDocxTool;
    // 区间内：lock [1,2] 改块 2 → 通过
    let args = serde_json::json!({
        "path": file.to_string_lossy(),
        "allowed_blocks": [1, 2],
        "operations": [
            { "op": "replace_text", "block": 2, "expect_prefix": "第二段", "new_text": "改写段" }
        ]
    })
    .to_string();
    let out = tool.execute(&args).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["applied"], 1, "{out}");

    // 区间外：lock [1,2] 动块 3 → 整批拒（家族前缀），文件字节原样
    let before = std::fs::read(&file).unwrap();
    let args = serde_json::json!({
        "path": file.to_string_lossy(),
        "allowed_blocks": [1, 2],
        "operations": [
            { "op": "replace_text", "block": 3, "expect_prefix": "第三段", "new_text": "越界段" }
        ]
    })
    .to_string();
    let err = tool.execute(&args).await.unwrap_err().to_string();
    assert!(err.contains("区间外块"), "实际: {err}");
    assert!(err.contains("1..=2"), "区间要在文案里: {err}");
    assert_eq!(
        std::fs::read(&file).unwrap(),
        before,
        "拒批文件逐字节 untouched"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn range_lock_rejects_clear_body_and_bad_bounds() {
    let dir = std::env::temp_dir().join("icepaw_edit_docx_lock_test2");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("样本.docx");
    std::fs::write(&file, three_para_docx()).unwrap();
    let before = std::fs::read(&file).unwrap();

    let tool = EditDocxTool;
    // clear_body 清空全文，与任何区间语义冲突 → 拒
    let args = serde_json::json!({
        "path": file.to_string_lossy(),
        "allowed_blocks": [1, 3],
        "operations": [ { "op": "clear_body", "expect_blocks": 3 } ]
    })
    .to_string();
    let err = tool.execute(&args).await.unwrap_err().to_string();
    assert!(err.contains("区间外块"), "实际: {err}");
    assert!(err.contains("clear_body"), "实际: {err}");
    assert_eq!(std::fs::read(&file).unwrap(), before);

    // 非法区间 lo>hi → 工具层拒
    let args = serde_json::json!({
        "path": file.to_string_lossy(),
        "allowed_blocks": [3, 2],
        "operations": [ { "op": "delete_block", "block": 2, "expect_prefix": "第二段" } ]
    })
    .to_string();
    let err = tool.execute(&args).await.unwrap_err().to_string();
    assert!(err.contains("区间锁无效"), "实际: {err}");

    // lo=0（块号 1-based）→ 拒
    let args = serde_json::json!({
        "path": file.to_string_lossy(),
        "allowed_blocks": [0, 2],
        "operations": [ { "op": "delete_block", "block": 2, "expect_prefix": "第二段" } ]
    })
    .to_string();
    let err = tool.execute(&args).await.unwrap_err().to_string();
    assert!(err.contains("区间锁无效"), "实际: {err}");

    // 样式族无块号概念，与锁互斥 → 拒
    let args = serde_json::json!({
        "path": file.to_string_lossy(),
        "allowed_blocks": [1, 3],
        "operations": [ { "op": "create_style", "style_type": "paragraph", "name": "某样式" } ]
    })
    .to_string();
    let err = tool.execute(&args).await.unwrap_err().to_string();
    assert!(err.contains("区间锁无效"), "实际: {err}");
    assert_eq!(std::fs::read(&file).unwrap(), before);
    std::fs::remove_dir_all(&dir).ok();
}

// ---- insert_table_after rows_text（D15 八波③）----

/// 表格工具测试用锚段包。
fn anchored_docx() -> Vec<u8> {
    use docx_rs::{Document, Docx, Paragraph, Run};
    let document =
        Document::new().add_paragraph(Paragraph::new().add_run(Run::new().add_text("锚段")));
    let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
    Docx::new()
        .document(document)
        .build()
        .pack(&mut cursor)
        .unwrap();
    cursor.into_inner()
}

#[tokio::test]
async fn rows_text_parses_markdown_and_matches_rows_form() {
    let dir = std::env::temp_dir().join("icepaw_rows_text_test");
    std::fs::create_dir_all(&dir).unwrap();
    let tool = EditDocxTool;

    // 同一表格两种形态：rows_text（Markdown 含分隔行 + \| 转义）vs rows 矩阵。
    // 模板字节取一次写两份（docx-rs 可能嵌时间戳，两次 build 字节不保证相同）
    let template = anchored_docx();
    let md = "| 列1 | 列2 |\n| --- | :---: |\n| 甲 | a\\|b |\n\n";
    let cases: [(&str, serde_json::Value); 2] = [
        ("rows_text", serde_json::json!(md)),
        ("rows", serde_json::json!([["列1", "列2"], ["甲", "a|b"]])),
    ];
    let mut summaries = Vec::new();
    let mut file_bytes = Vec::new();
    for (form, data) in cases {
        let file = dir.join(format!("{form}.docx"));
        std::fs::write(&file, &template).unwrap();
        let mut op = serde_json::json!({
            "op": "insert_table_after", "block": 1, "expect_prefix": "锚段"
        });
        op[form] = data;
        let args =
            serde_json::json!({ "path": file.to_string_lossy(), "operations": [op] }).to_string();
        let out = tool.execute(&args).await.unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["applied"], 1, "{form}: {out}");
        summaries.push(v["operations"].to_string());
        file_bytes.push(std::fs::read(&file).unwrap());
    }
    // 值优先与结构化等价：applied 摘要相等 + 产物字节逐字节相等
    assert_eq!(summaries[0], summaries[1], "两种形态 applied 摘要须一致");
    assert_eq!(file_bytes[0], file_bytes[1], "两种形态产物须一致");

    // 读回：表格内容正确落格（含 \| 还原的字面竖线）
    let inspect = crate::harness::doc::inspect_document(
        &file_bytes[0],
        &InspectRequest {
            projection: InspectProjection::Table,
            start: None,
            end: None,
            row: None,
            cell: None,
            style: None,
            num_id: None,
            level: None,
        },
    )
    .unwrap();
    assert!(inspect.content.contains("a|b"), "{}", inspect.content);
    assert!(inspect.content.contains("列1"), "{}", inspect.content);
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn rows_text_tsv_and_error_paths() {
    let dir = std::env::temp_dir().join("icepaw_rows_text_test2");
    std::fs::create_dir_all(&dir).unwrap();
    let tool = EditDocxTool;
    let file = dir.join("t.docx");
    std::fs::write(&file, anchored_docx()).unwrap();

    // TSV：制表符分列
    let args = serde_json::json!({
        "path": file.to_string_lossy(),
        "operations": [ {
            "op": "insert_table_after", "block": 1, "expect_prefix": "锚段",
            "rows_text": "甲\t乙\n丙\t丁"
        } ]
    })
    .to_string();
    let out = tool.execute(&args).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["applied"], 1, "{out}");

    // rows 与 rows_text 同时给 → 互斥拒
    let args = serde_json::json!({
        "path": file.to_string_lossy(),
        "operations": [ {
            "op": "insert_table_after", "block": 1, "expect_prefix": "锚段",
            "rows": [["x"]], "rows_text": "| x |"
        } ]
    })
    .to_string();
    let err = tool.execute(&args).await.unwrap_err().to_string();
    assert!(err.contains("表格文本无效"), "实际: {err}");

    // 都不给 → 拒（必填二选一）
    let args = serde_json::json!({
        "path": file.to_string_lossy(),
        "operations": [ { "op": "insert_table_after", "block": 1, "expect_prefix": "锚段" } ]
    })
    .to_string();
    let err = tool.execute(&args).await.unwrap_err().to_string();
    assert!(err.contains("表格文本无效"), "实际: {err}");

    // 既无 | 也无制表符 → 无法切列
    let args = serde_json::json!({
        "path": file.to_string_lossy(),
        "operations": [ {
            "op": "insert_table_after", "block": 1, "expect_prefix": "锚段",
            "rows_text": "就一行没有分隔符"
        } ]
    })
    .to_string();
    let err = tool.execute(&args).await.unwrap_err().to_string();
    assert!(err.contains("无法切列"), "实际: {err}");
    std::fs::remove_dir_all(&dir).ok();
}

/// parse_rows_text 边缘（同步直调）：首尾包裹竖线剥离 / 空行忽略 / 空文本拒。
#[test]
fn parse_rows_text_edges() {
    let rows = parse_rows_text("| a | b |\n| c | d |").unwrap();
    assert_eq!(rows, [["a", "b"], ["c", "d"]]);

    // 无包裹竖线的 Markdown 行同样合法；空行忽略
    let rows = parse_rows_text("a | b\n\nc | d").unwrap();
    assert_eq!(rows, [["a", "b"], ["c", "d"]]);

    // 只有分隔行 → 空，拒
    let err = parse_rows_text("| --- | --- |").unwrap_err().to_string();
    assert!(err.contains("解析后为空"), "实际: {err}");
    let err = parse_rows_text("").unwrap_err().to_string();
    assert!(err.contains("解析后为空"), "实际: {err}");
}

// =========================================================================
// write_docx 工具壳（D16 九波）：三层模板 / 备份覆盖 / 参数守卫 / 读回
// =========================================================================

/// 造带 workspace 的 ToolContext（write_docx 的 L2 相对模板路径需要它）。
async fn write_ctx(workspace: Option<String>) -> ToolContext {
    ToolContext {
        tool_use_id: None,
        conv_id: "c1".into(),
        agent_id: "a1".into(),
        project_id: None,
        workspace,
        pool: sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap(),
        api_key: None,
        app_handle: None,
        proposal_registry: None,
        turn_id: None,
        cancel: None,
    }
}

/// text 投影读回产物（带块号正文，验内容与块序）。
fn read_back_text(bytes: &[u8]) -> String {
    crate::harness::doc::inspect_document(
        bytes,
        &InspectRequest {
            projection: InspectProjection::Text,
            start: None,
            end: None,
            row: None,
            cell: None,
            style: None,
            num_id: None,
            level: None,
        },
    )
    .unwrap()
    .content
}

#[tokio::test]
async fn write_docx_builtin_creates_ordered_document() {
    let dir = std::env::temp_dir().join("icepaw_write_docx_test");
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("报告.docx");

    // L1 缺省模板（= report）+ 混合块序：heading / paragraph / table(rows_text)
    let args = serde_json::json!({
        "path": file.to_string_lossy(),
        "blocks": [
            { "type": "heading", "level": 1, "text": "季度报告" },
            { "type": "paragraph", "text": "本季度进展顺利。" },
            { "type": "table", "rows_text": "| 指标 | 数值 |\n| --- | --- |\n| 交付 | 3 |" },
            { "type": "heading", "level": 2, "text": "下季度计划" }
        ]
    })
    .to_string();
    let ctx = write_ctx(None).await;
    let out = WriteDocxTool
        .execute_with_context(&args, &ctx)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["created"], true, "{out}");
    assert_eq!(v["template"], "report", "{out}");
    assert_eq!(v["paragraphs"], 3, "{out}"); // 两 heading + 一段
    assert_eq!(v["tables"], 1, "{out}");
    assert_eq!(v["check"], "passed", "{out}");

    // 读回：四类内容全在且块序正确（text 投影带块号顺序输出）
    let text = read_back_text(&std::fs::read(&file).unwrap());
    let pos = |needle: &str| {
        text.find(needle)
            .unwrap_or_else(|| panic!("缺 {needle}: {text}"))
    };
    assert!(pos("季度报告") < pos("本季度进展顺利。"));
    assert!(pos("本季度进展顺利。") < pos("交付"));
    assert!(pos("交付") < pos("下季度计划"));
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn write_docx_template_three_layers() {
    let dir = std::env::temp_dir().join("icepaw_write_docx_test2");
    std::fs::remove_dir_all(&dir).ok();
    let ws = dir.join("ws");
    std::fs::create_dir_all(ws.join("templates")).unwrap();
    // 「家模板」= 内置模板字节落盘占位（真模板用户后续用 Word 造）
    let house = ws.join("templates").join("memo.docx");
    std::fs::write(
        &house,
        crate::harness::doc::build_builtin_template("report").unwrap(),
    )
    .unwrap();

    let blocks = r#"[{"type":"paragraph","text":"内容"}]"#.to_string();
    let ctx_ws = write_ctx(Some(ws.to_string_lossy().to_string())).await;

    // L2 相对路径：workspace/templates/ 下命中
    let file = dir.join("l2.docx");
    let args = serde_json::json!({
        "path": file.to_string_lossy(),
        "template": "memo.docx",
        "blocks": serde_json::from_str::<serde_json::Value>(&blocks).unwrap()
    })
    .to_string();
    let out = WriteDocxTool
        .execute_with_context(&args, &ctx_ws)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["template"], "memo.docx", "{out}");
    assert!(file.exists(), "目标应已生成");

    // L3 绝对路径：任意 docx
    let file = dir.join("l3.docx");
    let args = serde_json::json!({
        "path": file.to_string_lossy(),
        "template": house.to_string_lossy(),
        "blocks": serde_json::from_str::<serde_json::Value>(&blocks).unwrap()
    })
    .to_string();
    let out = WriteDocxTool
        .execute_with_context(&args, &ctx_ws)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["created"], true, "{out}");

    // 相对路径但无 workspace → 拒 + 三层规则文案（含档位名）
    let ctx_none = write_ctx(None).await;
    let args = serde_json::json!({
        "path": dir.join("no.docx").to_string_lossy(),
        "template": "memo.docx",
        "blocks": serde_json::from_str::<serde_json::Value>(&blocks).unwrap()
    })
    .to_string();
    let err = WriteDocxTool
        .execute_with_context(&args, &ctx_none)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("模板无效"), "实际: {err}");
    assert!(err.contains("report"), "应列内置档位名: {err}");

    // workspace 有但模板文件缺失 → not-found + did-you-mean
    let args = serde_json::json!({
        "path": dir.join("no.docx").to_string_lossy(),
        "template": "memoo.docx",
        "blocks": serde_json::from_str::<serde_json::Value>(&blocks).unwrap()
    })
    .to_string();
    let err = WriteDocxTool
        .execute_with_context(&args, &ctx_ws)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("模板无效"), "实际: {err}");
    assert!(err.contains("不存在"), "实际: {err}");
    std::fs::remove_dir_all(&dir).ok();
}

// =========================================================================
// 模板四层解析（D17 共享模板目录）——顺序 / 内置兜底 / 可用清单提示
// =========================================================================

#[tokio::test]
async fn resolve_template_shared_dir_fallback_and_order() {
    let dir = std::env::temp_dir().join("icepaw_resolve_tpl_test");
    std::fs::remove_dir_all(&dir).ok();
    let ws = dir.join("ws");
    let shared = dir.join("shared");
    std::fs::create_dir_all(ws.join("templates")).unwrap();
    std::fs::create_dir_all(&shared).unwrap();
    let (seed_name, seed_bytes) = crate::harness::doc::shared_templates::SHARED_TEMPLATE_SEEDS[0];

    // 共享目录命中：无 workspace 也能用（共享目录不依赖 workspace）
    std::fs::write(shared.join(seed_name), seed_bytes).unwrap();
    let bytes = resolve_template(seed_name, None, Some(&shared))
        .await
        .unwrap();
    assert_eq!(bytes, seed_bytes.to_vec());

    // workspace 优先于共享目录（更具体者赢）：同名时取 workspace 字节。
    // 合成 docx 只取一次样——docx_rs 每次打包带时间戳，两次调用字节必异。
    let synthetic = docx_bytes();
    std::fs::write(ws.join("templates").join(seed_name), &synthetic).unwrap();
    let bytes = resolve_template(seed_name, Some(ws.to_str().unwrap()), Some(&shared))
        .await
        .unwrap();
    assert_eq!(bytes, synthetic);

    // 内置档位兜底：两处目录都无该文件时回落 BUILTIN_TEMPLATES
    let bytes = resolve_template("report", Some(ws.to_str().unwrap()), Some(&shared))
        .await
        .unwrap();
    assert!(!bytes.is_empty(), "内置档位应兜底");

    // 同名文件可覆盖内置档位：workspace 放 report.docx → 取文件
    std::fs::write(ws.join("templates").join("report.docx"), &synthetic).unwrap();
    let bytes = resolve_template("report", Some(ws.to_str().unwrap()), Some(&shared))
        .await
        .unwrap();
    assert_eq!(bytes, synthetic);

    // miss 报错即行为契约：列两处目录清单 + 内置档位
    let err = resolve_template("zzz.docx", Some(ws.to_str().unwrap()), Some(&shared))
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("模板无效"), "实际: {err}");
    assert!(err.contains("共享模板目录"), "应列共享目录: {err}");
    assert!(err.contains(seed_name), "应列共享目录现有模板: {err}");
    assert!(err.contains("report"), "应列内置档位: {err}");
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn write_docx_overwrite_existing_backs_up() {
    let dir = std::env::temp_dir().join("icepaw_write_docx_test3");
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("old.docx");
    std::fs::write(&file, docx_bytes()).unwrap(); // 旧内容：单段「正文段」

    let args = serde_json::json!({
        "path": file.to_string_lossy(),
        "blocks": [ { "type": "heading", "level": 1, "text": "新标题" } ]
    })
    .to_string();
    let ctx = write_ctx(None).await;
    let out = WriteDocxTool
        .execute_with_context(&args, &ctx)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["created"], false, "{out}");
    let backup = v["backup"].as_str().unwrap().to_string();
    assert!(!backup.is_empty() && Path::new(&backup).exists(), "{out}");

    // 覆盖后 = 新文档（新标题在、旧正文不在）；备份里是旧内容
    let text = read_back_text(&std::fs::read(&file).unwrap());
    assert!(text.contains("新标题"), "{text}");
    assert!(!text.contains("正文段"), "旧内容应已被替换: {text}");
    let backup_text = read_back_text(&std::fs::read(&backup).unwrap());
    assert!(
        backup_text.contains("正文段"),
        "备份应是旧文件: {backup_text}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn write_docx_rejects_bad_input() {
    let dir = std::env::temp_dir().join("icepaw_write_docx_test4");
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    let ctx = write_ctx(None).await;

    // blocks 空 → 生成块无效
    let args = serde_json::json!({
        "path": dir.join("a.docx").to_string_lossy(),
        "blocks": []
    })
    .to_string();
    let err = WriteDocxTool
        .execute_with_context(&args, &ctx)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("生成块无效"), "实际: {err}");

    // blocks 超上限（MAX_WRITE_BLOCKS=300）→ 生成块无效
    let overflow: Vec<serde_json::Value> = (0..=300)
        .map(|i| serde_json::json!({ "type": "paragraph", "text": format!("段{i}") }))
        .collect();
    let args = serde_json::json!({
        "path": dir.join("b.docx").to_string_lossy(),
        "blocks": overflow
    })
    .to_string();
    let err = WriteDocxTool
        .execute_with_context(&args, &ctx)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("超过上限"), "实际: {err}");

    // 目标非 .docx → 指路 write_file
    let args = serde_json::json!({
        "path": dir.join("c.txt").to_string_lossy(),
        "blocks": [ { "type": "paragraph", "text": "x" } ]
    })
    .to_string();
    let err = WriteDocxTool
        .execute_with_context(&args, &ctx)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("write_file"), "实际: {err}");

    // 未知块型（serde tag 拒）→ 参数解析失败
    let args = serde_json::json!({
        "path": dir.join("d.docx").to_string_lossy(),
        "blocks": [ { "type": "video", "text": "x" } ]
    })
    .to_string();
    let err = WriteDocxTool
        .execute_with_context(&args, &ctx)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("参数解析失败"), "实际: {err}");
    std::fs::remove_dir_all(&dir).ok();
}

// =========================================================================
// toc/image 块 + insert 两操作（D18 十波③ 工具层）
// =========================================================================

/// 造一张真实 PNG（image crate 编码，10×6 纯色）——装载层格式/尺寸探测实弹。
fn tiny_png() -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(10, 6, image::Rgba([180, 60, 60, 255]));
    let mut buf = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut buf, image::ImageFormat::Png)
        .unwrap();
    buf.into_inner()
}

/// 读 zip 内某文本部件（缺失返 None）。
fn zip_part(bytes: &[u8], name: &str) -> Option<String> {
    use std::io::Read;
    let mut a = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut f = match a.by_name(name) {
        Ok(f) => f,
        Err(_) => return None,
    };
    let mut s = String::new();
    f.read_to_string(&mut s).unwrap();
    Some(s)
}

/// zip 内部件存在性（二进制件如 media 只探不读）。
fn zip_has(bytes: &[u8], name: &str) -> bool {
    zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .unwrap()
        .by_name(name)
        .is_ok()
}

#[tokio::test]
async fn write_docx_toc_and_image_blocks() {
    let dir = std::env::temp_dir().join("icepaw_write_docx_tocimg");
    std::fs::remove_dir_all(&dir).ok();
    let ws = dir.join("ws");
    std::fs::create_dir_all(ws.join("figs")).unwrap();
    let png = tiny_png();
    std::fs::write(ws.join("figs").join("图1.png"), &png).unwrap();
    let file = dir.join("报告.docx");

    // image 相对路径挂 workspace；toc 缺省 levels=3/hyperlink=true
    let args = serde_json::json!({
        "path": file.to_string_lossy(),
        "blocks": [
            { "type": "heading", "level": 1, "text": "系统报告" },
            { "type": "toc", "levels": 3 },
            { "type": "image", "path": "figs/图1.png", "width_mm": 120 },
            { "type": "paragraph", "text": "部署完成。" }
        ]
    })
    .to_string();
    let ctx = write_ctx(Some(ws.to_string_lossy().to_string())).await;
    let out = WriteDocxTool
        .execute_with_context(&args, &ctx)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["images"], 1, "{out}");
    assert_eq!(v["tocs"], 1, "{out}");
    assert_eq!(v["check"], "passed", "{out}");

    // 产物：media 字节全等 + settings updateFields（内置 report 无 settings →
    // 新建路径）+ 读回投影域/图片标记可见（治「空段」盲区）
    let bytes = std::fs::read(&file).unwrap();
    let media = {
        use std::io::Read;
        let mut a = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
        let mut m = Vec::new();
        a.by_name("word/media/image1.png")
            .unwrap()
            .read_to_end(&mut m)
            .unwrap();
        m
    };
    assert_eq!(media, png, "media 部件应与源文件字节全等");
    assert!(
        zip_part(&bytes, "word/settings.xml")
            .unwrap()
            .contains("updateFields"),
        "settings 应自动置 updateFields"
    );
    let text = read_back_text(&bytes);
    assert!(text.contains("[域:TOC]"), "{text}");
    assert!(text.contains("[图片×1]"), "{text}");
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn edit_docx_insert_image_and_toc() {
    let dir = std::env::temp_dir().join("icepaw_edit_tocimg");
    std::fs::remove_dir_all(&dir).ok();
    let ws = dir.join("ws");
    std::fs::create_dir_all(ws.join("figs")).unwrap();
    let png = tiny_png();
    std::fs::write(ws.join("figs").join("fig.png"), &png).unwrap();
    let file = dir.join("t.docx");
    std::fs::write(&file, anchored_docx()).unwrap();

    // 同批图 + TOC（doc 族混排合法）；image 相对路径挂 ctx.workspace
    let args = serde_json::json!({
        "path": file.to_string_lossy(),
        "operations": [
            { "op": "insert_image_after", "block": 1, "expect_prefix": "锚段",
              "image_path": "figs/fig.png" },
            { "op": "insert_toc_after", "block": 1, "expect_prefix": "锚段", "levels": 2 }
        ]
    })
    .to_string();
    let ctx = write_ctx(Some(ws.to_string_lossy().to_string())).await;
    let out = EditDocxTool
        .execute_with_context(&args, &ctx)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["applied"], 2, "{out}");

    let bytes = std::fs::read(&file).unwrap();
    assert!(zip_has(&bytes, "word/media/image1.png"), "media 应入包");
    assert!(zip_part(&bytes, "word/settings.xml")
        .unwrap()
        .contains("updateFields"));
    let text = read_back_text(&bytes);
    assert!(text.contains("[域:TOC]"), "{text}");
    assert!(text.contains("[图片×1]"), "{text}");

    // 直调 execute（无 workspace）+ 相对路径 → 图片无效 + 指路绝对路径
    let file2 = dir.join("t2.docx");
    std::fs::write(&file2, anchored_docx()).unwrap();
    let args = serde_json::json!({
        "path": file2.to_string_lossy(),
        "operations": [ { "op": "insert_image_after", "block": 1,
                          "expect_prefix": "锚段", "image_path": "figs/fig.png" } ]
    })
    .to_string();
    let err = EditDocxTool.execute(&args).await.unwrap_err().to_string();
    assert!(err.contains("图片无效"), "实际: {err}");
    assert!(err.contains("绝对路径"), "应指路绝对路径: {err}");

    // schema 披露抽查：两操作进 edit schema、两块型进 write schema
    let p = EditDocxTool.parameters().to_string();
    assert!(p.contains("insert_image_after"), "edit schema 缺插图操作");
    assert!(p.contains("insert_toc_after"), "edit schema 缺 TOC 操作");
    let p = WriteDocxTool.parameters().to_string();
    assert!(p.contains(r#""const":"toc""#), "write schema 缺 toc 块");
    assert!(p.contains(r#""const":"image""#), "write schema 缺 image 块");
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn validate_docx_image_and_field_assertions() {
    // write_docx 全链造带域带图的文档（heading=1 toc=2 image=3）
    let dir = std::env::temp_dir().join("icepaw_validate_tocimg");
    std::fs::remove_dir_all(&dir).ok();
    let ws = dir.join("ws");
    std::fs::create_dir_all(ws.join("figs")).unwrap();
    std::fs::write(ws.join("figs").join("f.png"), tiny_png()).unwrap();
    let file = dir.join("v.docx");
    let args = serde_json::json!({
        "path": file.to_string_lossy(),
        "blocks": [
            { "type": "heading", "level": 1, "text": "标题" },
            { "type": "toc" },
            { "type": "image", "path": "figs/f.png" }
        ]
    })
    .to_string();
    let ctx = write_ctx(Some(ws.to_string_lossy().to_string())).await;
    WriteDocxTool
        .execute_with_context(&args, &ctx)
        .await
        .unwrap();

    let tool = ValidateDocxTool;
    let args = serde_json::json!({
        "path": file.to_string_lossy(),
        "assertions": [
            { "kind": "block_image", "block": 3, "equals": 1 },
            { "kind": "block_field", "block": 2, "instr_contains": "TOC" },
            { "kind": "block_field", "block": 3, "instr_contains": "TOC" },
            { "kind": "block_image", "block": 1 },
            { "kind": "block_image", "block": 99 }
        ]
    })
    .to_string();
    let out = tool.execute(&args).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    // 失败 = 正常数据：块 3 是图非域、块 1 标题段无图、块 99 越界
    assert_eq!(v["passed"], false, "{out}");
    assert_eq!(v["total"], 5);
    assert_eq!(v["failed"], 3, "{out}");
    let kinds: Vec<&str> = v["failures"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        ["block_field", "block_image", "block_image"],
        "{out}"
    );

    // 全过形态：存在性（省 equals）+ 子串两 kind 进摘要
    let args = serde_json::json!({
        "path": file.to_string_lossy(),
        "assertions": [
            { "kind": "block_image", "block": 3 },
            { "kind": "block_field", "block": 2, "instr_contains": "TOC" }
        ]
    })
    .to_string();
    let out = tool.execute(&args).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["passed"], true, "{out}");
    let kinds: Vec<&str> = v["passed_kinds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k.as_str().unwrap())
        .collect();
    let kinds = kinds.join(",");
    assert!(
        kinds.contains("block_image") && kinds.contains("block_field"),
        "{out}"
    );
    std::fs::remove_dir_all(&dir).ok();
}
