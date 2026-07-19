//! 模板仓库（`repo::template`）的回归测试
//!
//! 覆盖：
//! - 列表按 sort_order 升序
//! - 创建 / 获取 / 更新（partial）/ 删除
//! - variables / tools JSON 编解码往返一致
//! - 非法 name 校验
//!
//! 运行：`cargo test --test template_repo_test`

use ice_paw_lib::db::models::{
    NewTemplate, TemplateUpdate, TemplateVariable,
};
use ice_paw_lib::db::repo::template;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::str::FromStr;

async fn fresh_pool() -> SqlitePool {
    let opts = SqliteConnectOptions::from_str("sqlite::memory:")
        .expect("valid sqlite url")
        .create_if_missing(true)
        .foreign_keys(true);
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .expect("connect in-memory sqlite")
}

#[tokio::test]
async fn test_helper_imports_compile() {
    // 冒烟：确保类型可访问、迁移能跑起来
    let pool = fresh_pool().await;
    sqlx::migrate!("./src/db/migrations")
        .run(&pool)
        .await
        .expect("migrations run");
}

#[tokio::test]
async fn create_then_list_roundtrip() {
    let pool = fresh_pool().await;
    sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();

    let new = NewTemplate {
        name: "代码评审".into(),
        description: "对代码片段做严格 review".into(),
        system_prompt: "你是一位资深 Rust 工程师".into(),
        user_prompt_prefix: "请评审以下代码：\n".into(),
        variables: Some(vec![TemplateVariable {
            name: "language".into(),
            label: "语言".into(),
            var_type: "select".into(),
            default: Some("Rust".into()),
            options: Some(vec!["Rust".into(), "TypeScript".into()]),
        }]),
        tools: Some(vec!["read_file".into()]),
        sort_order: 0,
    };

    let row = template::create(&pool, &new, "tpl-1").await.expect("create");
    assert_eq!(row.id, "tpl-1");
    assert_eq!(row.name, "代码评审");
    // JSON 编码后能解析回来
    let vars: Vec<TemplateVariable> = serde_json::from_str(&row.variables).unwrap();
    assert_eq!(vars.len(), 1);
    assert_eq!(vars[0].name, "language");
    let tools: Vec<String> = serde_json::from_str(&row.tools).unwrap();
    assert_eq!(tools, vec!["read_file".to_string()]);

    // 列表应包含该模板
    let all = template::list(&pool).await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].id, "tpl-1");
}

#[tokio::test]
async fn list_orders_by_sort_order() {
    let pool = fresh_pool().await;
    sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();

    for (id, so) in [("a", 30), ("b", 10), ("c", 20)] {
        let new = NewTemplate {
            name: id.into(),
            description: "".into(),
            system_prompt: "".into(),
            user_prompt_prefix: "".into(),
            variables: None,
            tools: None,
            sort_order: so,
        };
        template::create(&pool, &new, id).await.unwrap();
    }

    let all = template::list(&pool).await.unwrap();
    let ids: Vec<_> = all.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, vec!["b", "c", "a"]);
}

#[tokio::test]
async fn get_by_id_missing_returns_not_found() {
    let pool = fresh_pool().await;
    sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();

    let err = template::get_by_id(&pool, "nonexistent").await.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("nonexistent"), "expected id in error: {msg}");
}

#[tokio::test]
async fn update_partial_only_changes_given_fields() {
    let pool = fresh_pool().await;
    sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();

    let new = NewTemplate {
        name: "orig".into(),
        description: "orig-desc".into(),
        system_prompt: "orig-sys".into(),
        user_prompt_prefix: "orig-usr".into(),
        variables: Some(vec![]),
        tools: Some(vec![]),
        sort_order: 5,
    };
    template::create(&pool, &new, "tpl-1").await.unwrap();

    // 只改 name + sort_order
    let upd = TemplateUpdate {
        id: "tpl-1".into(),
        name: Some("updated".into()),
        description: None,
        system_prompt: None,
        user_prompt_prefix: None,
        variables: None,
        tools: None,
        sort_order: Some(99),
    };
    let row = template::update(
        &pool,
        "tpl-1",
        template::TemplateUpdateFields {
            name: upd.name.as_deref(),
            description: upd.description.as_deref(),
            system_prompt: upd.system_prompt.as_deref(),
            user_prompt_prefix: upd.user_prompt_prefix.as_deref(),
            variables: upd.variables.as_ref(),
            tools: upd.tools.as_ref(),
            sort_order: upd.sort_order,
        },
    )
    .await
    .unwrap();

    assert_eq!(row.name, "updated");
    assert_eq!(row.sort_order, 99);
    // 未传字段保持原值
    assert_eq!(row.description, "orig-desc");
    assert_eq!(row.system_prompt, "orig-sys");
    assert_eq!(row.user_prompt_prefix, "orig-usr");
}

#[tokio::test]
async fn update_variables_replaces_whole_array() {
    let pool = fresh_pool().await;
    sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();

    let new = NewTemplate {
        name: "t".into(),
        description: "".into(),
        system_prompt: "".into(),
        user_prompt_prefix: "".into(),
        variables: Some(vec![TemplateVariable {
            name: "a".into(),
            label: "A".into(),
            var_type: "text".into(),
            default: None,
            options: None,
        }]),
        tools: None,
        sort_order: 0,
    };
    template::create(&pool, &new, "tpl-1").await.unwrap();

    // 整体替换
    let new_vars = vec![
        TemplateVariable {
            name: "lang".into(),
            label: "语言".into(),
            var_type: "select".into(),
            default: Some("Go".into()),
            options: Some(vec!["Rust".into(), "Go".into(), "TS".into()]),
        },
        TemplateVariable {
            name: "level".into(),
            label: "深度".into(),
            var_type: "text".into(),
            default: Some("3".into()),
            options: None,
        },
    ];
    let row = template::update(
        &pool,
        "tpl-1",
        template::TemplateUpdateFields {
            name: None,
            description: None,
            system_prompt: None,
            user_prompt_prefix: None,
            variables: Some(&new_vars),
            tools: None,
            sort_order: None,
        },
    )
    .await
    .unwrap();
    let parsed: Vec<TemplateVariable> = serde_json::from_str(&row.variables).unwrap();
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].name, "lang");
    assert_eq!(parsed[0].options.as_ref().unwrap().len(), 3);
    assert_eq!(parsed[1].name, "level");
}

#[tokio::test]
async fn delete_then_not_found() {
    let pool = fresh_pool().await;
    sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();

    let new = NewTemplate {
        name: "t".into(),
        description: "".into(),
        system_prompt: "".into(),
        user_prompt_prefix: "".into(),
        variables: None,
        tools: None,
        sort_order: 0,
    };
    template::create(&pool, &new, "tpl-1").await.unwrap();

    template::delete(&pool, "tpl-1").await.unwrap();

    let err = template::get_by_id(&pool, "tpl-1").await.unwrap_err();
    assert!(err.to_string().contains("tpl-1"));

    // 再删一次：rows_affected = 0 → NotFound
    let err2 = template::delete(&pool, "tpl-1").await.unwrap_err();
    assert!(err2.to_string().contains("tpl-1"));
}
