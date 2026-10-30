//! 内置 knowledge-graph memory 工具（原生替代 bundled `server-memory`）。
//!
//! 原实现走内置 Node + `@modelcontextprotocol/server-memory`，把实体/关系知识图谱
//! 持久化为 JSONL。本模块用 Rust 原生复刻其 9 个工具，**数据文件格式逐字节兼容**
//! （`<app_data>/mcp-memory/memory.jsonl`，每行 `{"type":"entity",...}` 或
//! `{"type":"relation",...}`），存量知识图谱零迁移、零数据损失。
//!
//! 授权：读/写（read/search/open/create/add）`Always`（本地知识图谱，无会话外副作用）；
//! 删（delete_*）`Confirm`（不可恢复删除，删了 JSONL 重写即丢）。

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

use super::client::{McpClient, ToolContext};
use super::types::AuthorizationLevel;

/// 记忆数据文件相对 app_data_dir 的目录。
const MEMORY_DIR: &str = "mcp-memory";

// =========================================================================
// 数据模型（字段 camelCase 对齐 Node 版 JSONL）
// =========================================================================

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
struct Entity {
    name: String,
    entity_type: String,
    observations: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
struct Relation {
    from: String,
    to: String,
    relation_type: String,
}

#[derive(Default, Serialize)]
struct KnowledgeGraph {
    entities: Vec<Entity>,
    relations: Vec<Relation>,
}

/// JSONL 行（serde tag 按 `type` 字段分派）。
///
/// ⚠️ `rename_all = "camelCase"` 在带 `tag` 的 enum 上不作用于变体 struct 字段，
/// 须在 `entity_type` / `relation_type` 两个字段上显式 rename，否则序列化成
/// snake_case 与 Node 版 JSONL 不兼容。
#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
enum GraphLine {
    #[serde(rename = "entity")]
    Entity {
        name: String,
        #[serde(rename = "entityType")]
        entity_type: String,
        observations: Vec<String>,
    },
    #[serde(rename = "relation")]
    Relation {
        from: String,
        to: String,
        #[serde(rename = "relationType")]
        relation_type: String,
    },
}

// =========================================================================
// 持久化
// =========================================================================

/// memory.jsonl 绝对路径（`<app_data>/mcp-memory/memory.jsonl`）。
/// 目录由 `save_graph` 惰性创建；load 遇 ENOENT 视为空图谱。
fn memory_file_path(ctx: &ToolContext) -> AppResult<PathBuf> {
    let app = ctx
        .app_handle
        .as_ref()
        .ok_or_else(|| AppError::Internal("memory 工具需要 app_handle 上下文".into()))?;
    let dir = crate::logging::data_dir(app)?.join(MEMORY_DIR);
    Ok(dir.join("memory.jsonl"))
}

async fn load_graph(path: &Path) -> AppResult<KnowledgeGraph> {
    let data = match tokio::fs::read_to_string(path).await {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(KnowledgeGraph::default()),
        Err(e) => return Err(AppError::Io(e)),
    };
    let mut graph = KnowledgeGraph::default();
    for line in data.lines().filter(|l| !l.trim().is_empty()) {
        match serde_json::from_str::<GraphLine>(line) {
            Ok(GraphLine::Entity {
                name,
                entity_type,
                observations,
            }) => graph.entities.push(Entity {
                name,
                entity_type,
                observations,
            }),
            Ok(GraphLine::Relation {
                from,
                to,
                relation_type,
            }) => graph.relations.push(Relation {
                from,
                to,
                relation_type,
            }),
            Err(e) => {
                return Err(AppError::Internal(format!("记忆数据损坏: 数据行解析失败: {e}")))
            }
        }
    }
    Ok(graph)
}

async fn save_graph(path: &Path, graph: &KnowledgeGraph) -> AppResult<()> {
    let mut lines: Vec<String> = Vec::with_capacity(graph.entities.len() + graph.relations.len());
    for e in &graph.entities {
        let line = GraphLine::Entity {
            name: e.name.clone(),
            entity_type: e.entity_type.clone(),
            observations: e.observations.clone(),
        };
        lines.push(
            serde_json::to_string(&line)
                .map_err(|e| AppError::Internal(format!("记忆序列化失败: {e}")))?,
        );
    }
    for r in &graph.relations {
        let line = GraphLine::Relation {
            from: r.from.clone(),
            to: r.to.clone(),
            relation_type: r.relation_type.clone(),
        };
        lines.push(
            serde_json::to_string(&line)
                .map_err(|e| AppError::Internal(format!("记忆序列化失败: {e}")))?,
        );
    }
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(AppError::Io)?;
    }
    tokio::fs::write(path, lines.join("\n"))
        .await
        .map_err(AppError::Io)?;
    Ok(())
}

/// 解析工具 args 的统一入口（带稳定家族前缀）。
fn parse_args<T: for<'de> Deserialize<'de>>(tool: &str, args: &str) -> AppResult<T> {
    serde_json::from_str(args).map_err(|e| AppError::Validation(format!("记忆参数解析失败: {tool}: {e}")))
}

// =========================================================================
// 各工具 args
// =========================================================================

#[derive(Deserialize)]
struct CreateEntitiesArgs {
    entities: Vec<Entity>,
}

#[derive(Deserialize)]
struct CreateRelationsArgs {
    relations: Vec<Relation>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddObservationInput {
    entity_name: String,
    contents: Vec<String>,
}

#[derive(Deserialize)]
struct AddObservationsArgs {
    observations: Vec<AddObservationInput>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteEntitiesArgs {
    entity_names: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteObservationInput {
    entity_name: String,
    observations: Vec<String>,
}

#[derive(Deserialize)]
struct DeleteObservationsArgs {
    deletions: Vec<DeleteObservationInput>,
}

#[derive(Deserialize)]
struct DeleteRelationsArgs {
    relations: Vec<Relation>,
}

#[derive(Deserialize)]
struct SearchNodesArgs {
    query: String,
}

#[derive(Deserialize)]
struct OpenNodesArgs {
    names: Vec<String>,
}

// =========================================================================
// 写工具（Always）
// =========================================================================

pub struct CreateEntitiesTool;

#[async_trait]
impl McpClient for CreateEntitiesTool {
    fn name(&self) -> &str {
        "create_entities"
    }
    fn description(&self) -> &str {
        "Create multiple new entities in the knowledge graph"
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "entities": {
                    "type": "array",
                    "description": "Array of entities to create",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string", "description": "The name of the entity" },
                            "entityType": { "type": "string", "description": "The type of the entity" },
                            "observations": { "type": "array", "items": { "type": "string" }, "description": "An array of observation contents associated with the entity" }
                        },
                        "required": ["name", "entityType", "observations"]
                    }
                }
            },
            "required": ["entities"]
        })
    }
    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(
            "该工具必须通过 execute_with_context 调用".into(),
        ))
    }

    async fn execute_with_context(&self, args: &str, ctx: &ToolContext) -> AppResult<String> {
        let parsed: CreateEntitiesArgs = parse_args(self.name(), args)?;
        let path = memory_file_path(ctx)?;
        let mut graph = load_graph(&path).await?;
        let mut new_entities = Vec::new();
        for e in parsed.entities {
            if !graph.entities.iter().any(|x| x.name == e.name) {
                graph.entities.push(e.clone());
                new_entities.push(e);
            }
        }
        save_graph(&path, &graph).await?;
        Ok(serde_json::json!({ "entities": new_entities }).to_string())
    }
}

pub struct CreateRelationsTool;

#[async_trait]
impl McpClient for CreateRelationsTool {
    fn name(&self) -> &str {
        "create_relations"
    }
    fn description(&self) -> &str {
        "Create multiple new relations between entities in the knowledge graph. Relations should be in active voice"
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "relations": {
                    "type": "array",
                    "description": "Array of relations to create",
                    "items": {
                        "type": "object",
                        "properties": {
                            "from": { "type": "string", "description": "The name of the entity where the relation starts" },
                            "to": { "type": "string", "description": "The name of the entity where the relation ends" },
                            "relationType": { "type": "string", "description": "The type of the relation" }
                        },
                        "required": ["from", "to", "relationType"]
                    }
                }
            },
            "required": ["relations"]
        })
    }
    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(
            "该工具必须通过 execute_with_context 调用".into(),
        ))
    }

    async fn execute_with_context(&self, args: &str, ctx: &ToolContext) -> AppResult<String> {
        let parsed: CreateRelationsArgs = parse_args(self.name(), args)?;
        let path = memory_file_path(ctx)?;
        let mut graph = load_graph(&path).await?;
        let mut new_relations = Vec::new();
        for r in parsed.relations {
            let exists = graph.relations.iter().any(|x| {
                x.from == r.from && x.to == r.to && x.relation_type == r.relation_type
            });
            if !exists {
                graph.relations.push(r.clone());
                new_relations.push(r);
            }
        }
        save_graph(&path, &graph).await?;
        Ok(serde_json::json!({ "relations": new_relations }).to_string())
    }
}

pub struct AddObservationsTool;

#[async_trait]
impl McpClient for AddObservationsTool {
    fn name(&self) -> &str {
        "add_observations"
    }
    fn description(&self) -> &str {
        "Add new observations to existing entities in the knowledge graph"
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "observations": {
                    "type": "array",
                    "description": "Array of observations to add",
                    "items": {
                        "type": "object",
                        "properties": {
                            "entityName": { "type": "string", "description": "The name of the entity to add the observations to" },
                            "contents": { "type": "array", "items": { "type": "string" }, "description": "An array of observation contents to add" }
                        },
                        "required": ["entityName", "contents"]
                    }
                }
            },
            "required": ["observations"]
        })
    }
    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(
            "该工具必须通过 execute_with_context 调用".into(),
        ))
    }

    async fn execute_with_context(&self, args: &str, ctx: &ToolContext) -> AppResult<String> {
        let parsed: AddObservationsArgs = parse_args(self.name(), args)?;
        let path = memory_file_path(ctx)?;
        let mut graph = load_graph(&path).await?;
        let mut results = Vec::new();
        for o in parsed.observations {
            let entity = graph
                .entities
                .iter_mut()
                .find(|e| e.name == o.entity_name)
                .ok_or_else(|| {
                    AppError::Validation(format!(
                        "记忆实体不存在: 实体 \"{}\" 不在知识图谱中，请先用 create_entities 创建",
                        o.entity_name
                    ))
                })?;
            let added: Vec<String> = o
                .contents
                .into_iter()
                .filter(|c| !entity.observations.contains(c))
                .collect();
            entity.observations.extend(added.clone());
            results.push(serde_json::json!({
                "entityName": o.entity_name,
                "addedObservations": added,
            }));
        }
        save_graph(&path, &graph).await?;
        Ok(serde_json::json!({ "results": results }).to_string())
    }
}

// =========================================================================
// 删工具（Confirm——不可恢复删除）
// =========================================================================

pub struct DeleteEntitiesTool;

#[async_trait]
impl McpClient for DeleteEntitiesTool {
    fn name(&self) -> &str {
        "delete_entities"
    }
    fn description(&self) -> &str {
        "Delete multiple entities and their associated relations from the knowledge graph"
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "entityNames": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "An array of entity names to delete"
                }
            },
            "required": ["entityNames"]
        })
    }
    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::Confirm
    }
    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(
            "该工具必须通过 execute_with_context 调用".into(),
        ))
    }

    async fn execute_with_context(&self, args: &str, ctx: &ToolContext) -> AppResult<String> {
        let parsed: DeleteEntitiesArgs = parse_args(self.name(), args)?;
        let path = memory_file_path(ctx)?;
        let mut graph = load_graph(&path).await?;
        graph.entities.retain(|e| !parsed.entity_names.contains(&e.name));
        graph.relations.retain(|r| {
            !parsed.entity_names.contains(&r.from) && !parsed.entity_names.contains(&r.to)
        });
        save_graph(&path, &graph).await?;
        Ok("Entities deleted successfully".to_string())
    }
}

pub struct DeleteObservationsTool;

#[async_trait]
impl McpClient for DeleteObservationsTool {
    fn name(&self) -> &str {
        "delete_observations"
    }
    fn description(&self) -> &str {
        "Delete specific observations from entities in the knowledge graph"
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "deletions": {
                    "type": "array",
                    "description": "Array of observations to delete",
                    "items": {
                        "type": "object",
                        "properties": {
                            "entityName": { "type": "string", "description": "The name of the entity containing the observations" },
                            "observations": { "type": "array", "items": { "type": "string" }, "description": "An array of observations to delete" }
                        },
                        "required": ["entityName", "observations"]
                    }
                }
            },
            "required": ["deletions"]
        })
    }
    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::Confirm
    }
    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(
            "该工具必须通过 execute_with_context 调用".into(),
        ))
    }

    async fn execute_with_context(&self, args: &str, ctx: &ToolContext) -> AppResult<String> {
        let parsed: DeleteObservationsArgs = parse_args(self.name(), args)?;
        let path = memory_file_path(ctx)?;
        let mut graph = load_graph(&path).await?;
        for d in parsed.deletions {
            if let Some(entity) = graph.entities.iter_mut().find(|e| e.name == d.entity_name) {
                entity.observations.retain(|o| !d.observations.contains(o));
            }
        }
        save_graph(&path, &graph).await?;
        Ok("Observations deleted successfully".to_string())
    }
}

pub struct DeleteRelationsTool;

#[async_trait]
impl McpClient for DeleteRelationsTool {
    fn name(&self) -> &str {
        "delete_relations"
    }
    fn description(&self) -> &str {
        "Delete multiple relations from the knowledge graph"
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "relations": {
                    "type": "array",
                    "description": "Array of relations to delete",
                    "items": {
                        "type": "object",
                        "properties": {
                            "from": { "type": "string" },
                            "to": { "type": "string" },
                            "relationType": { "type": "string" }
                        },
                        "required": ["from", "to", "relationType"]
                    }
                }
            },
            "required": ["relations"]
        })
    }
    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::Confirm
    }
    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(
            "该工具必须通过 execute_with_context 调用".into(),
        ))
    }

    async fn execute_with_context(&self, args: &str, ctx: &ToolContext) -> AppResult<String> {
        let parsed: DeleteRelationsArgs = parse_args(self.name(), args)?;
        let path = memory_file_path(ctx)?;
        let mut graph = load_graph(&path).await?;
        graph.relations.retain(|r| {
            !parsed.relations.iter().any(|del| {
                r.from == del.from && r.to == del.to && r.relation_type == del.relation_type
            })
        });
        save_graph(&path, &graph).await?;
        Ok("Relations deleted successfully".to_string())
    }
}

// =========================================================================
// 读工具（Always）
// =========================================================================

pub struct ReadGraphTool;

#[async_trait]
impl McpClient for ReadGraphTool {
    fn name(&self) -> &str {
        "read_graph"
    }
    fn description(&self) -> &str {
        "Read the entire knowledge graph"
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object", "properties": {} })
    }
    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(
            "该工具必须通过 execute_with_context 调用".into(),
        ))
    }

    async fn execute_with_context(&self, _args: &str, ctx: &ToolContext) -> AppResult<String> {
        let path = memory_file_path(ctx)?;
        let graph = load_graph(&path).await?;
        Ok(serde_json::json!(graph).to_string())
    }
}

pub struct SearchNodesTool;

#[async_trait]
impl McpClient for SearchNodesTool {
    fn name(&self) -> &str {
        "search_nodes"
    }
    fn description(&self) -> &str {
        "Search for nodes in the knowledge graph based on a query"
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "The search query to match against entity names, types, and observation content" }
            },
            "required": ["query"]
        })
    }
    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(
            "该工具必须通过 execute_with_context 调用".into(),
        ))
    }

    async fn execute_with_context(&self, args: &str, ctx: &ToolContext) -> AppResult<String> {
        let parsed: SearchNodesArgs = parse_args(self.name(), args)?;
        let path = memory_file_path(ctx)?;
        let graph = load_graph(&path).await?;
        let q = parsed.query.to_lowercase();
        let filtered_entities: Vec<Entity> = graph
            .entities
            .into_iter()
            .filter(|e| {
                e.name.to_lowercase().contains(&q)
                    || e.entity_type.to_lowercase().contains(&q)
                    || e.observations.iter().any(|o| o.to_lowercase().contains(&q))
            })
            .collect();
        let names: std::collections::HashSet<String> =
            filtered_entities.iter().map(|e| e.name.clone()).collect();
        let filtered_relations: Vec<Relation> = graph
            .relations
            .into_iter()
            .filter(|r| names.contains(&r.from) || names.contains(&r.to))
            .collect();
        Ok(serde_json::json!({
            "entities": filtered_entities,
            "relations": filtered_relations,
        })
        .to_string())
    }
}

pub struct OpenNodesTool;

#[async_trait]
impl McpClient for OpenNodesTool {
    fn name(&self) -> &str {
        "open_nodes"
    }
    fn description(&self) -> &str {
        "Open specific nodes in the knowledge graph by their names"
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "names": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "An array of entity names to retrieve"
                }
            },
            "required": ["names"]
        })
    }
    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(
            "该工具必须通过 execute_with_context 调用".into(),
        ))
    }

    async fn execute_with_context(&self, args: &str, ctx: &ToolContext) -> AppResult<String> {
        let parsed: OpenNodesArgs = parse_args(self.name(), args)?;
        let path = memory_file_path(ctx)?;
        let graph = load_graph(&path).await?;
        let filtered_entities: Vec<Entity> = graph
            .entities
            .into_iter()
            .filter(|e| parsed.names.contains(&e.name))
            .collect();
        let names: std::collections::HashSet<String> =
            filtered_entities.iter().map(|e| e.name.clone()).collect();
        let filtered_relations: Vec<Relation> = graph
            .relations
            .into_iter()
            .filter(|r| names.contains(&r.from) || names.contains(&r.to))
            .collect();
        Ok(serde_json::json!({
            "entities": filtered_entities,
            "relations": filtered_relations,
        })
        .to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 用临时文件做无 app_handle 的纯持久化逻辑测试。
    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("icepaw-memory-test-{name}.jsonl"))
    }

    fn cleanup(path: &Path) {
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn load_save_roundtrip_preserves_jsonl() {
        let path = temp_path("roundtrip");
        cleanup(&path);
        let mut graph = KnowledgeGraph::default();
        graph.entities.push(Entity {
            name: "Alice".into(),
            entity_type: "person".into(),
            observations: vec!["likes coffee".into()],
        });
        graph.relations.push(Relation {
            from: "Alice".into(),
            to: "Bob".into(),
            relation_type: "knows".into(),
        });
        save_graph(&path, &graph).await.unwrap();

        // 逐字节验证 JSONL 形状（camelCase 字段名对齐 Node 版）
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains(r#""type":"entity""#));
        assert!(raw.contains(r#""entityType":"person""#));
        assert!(raw.contains(r#""type":"relation""#));
        assert!(raw.contains(r#""relationType":"knows""#));

        let loaded = load_graph(&path).await.unwrap();
        assert_eq!(loaded.entities.len(), 1);
        assert_eq!(loaded.entities[0].name, "Alice");
        assert_eq!(loaded.relations.len(), 1);
        cleanup(&path);
    }

    #[tokio::test]
    async fn load_missing_file_is_empty() {
        let path = temp_path("missing");
        cleanup(&path);
        let graph = load_graph(&path).await.unwrap();
        assert!(graph.entities.is_empty());
        assert!(graph.relations.is_empty());
    }

    /// 兼容 Node 版遗留 memory.jsonl 的真实行形状（camelCase 字段 + 裸 type）。
    #[tokio::test]
    async fn parses_legacy_node_jsonl_lines() {
        let path = temp_path("legacy");
        cleanup(&path);
        std::fs::write(
            &path,
            "{\"type\":\"entity\",\"name\":\"Alice\",\"entityType\":\"person\",\"observations\":[\"x\"]}\n\
             {\"type\":\"relation\",\"from\":\"Alice\",\"to\":\"Bob\",\"relationType\":\"knows\"}\n",
        )
        .unwrap();
        let graph = load_graph(&path).await.unwrap();
        assert_eq!(graph.entities.len(), 1);
        assert_eq!(graph.entities[0].entity_type, "person");
        assert_eq!(graph.relations.len(), 1);
        assert_eq!(graph.relations[0].relation_type, "knows");
        cleanup(&path);
    }
}
