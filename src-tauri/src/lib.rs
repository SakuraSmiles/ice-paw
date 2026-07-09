// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        // 注册 SQL 插件：使用 sqlite 特性，支持前端调用 Database.load("sqlite:...")
        .plugin(tauri_plugin_sql::Builder::new().build())
        // 注册 Store 插件：基于 AES-256-GCM 加密的本地键值存储，
        // 用于安全保存用户的 LLM API Key 等敏感信息。
        .plugin(tauri_plugin_store::Builder::new().build())
        .invoke_handler(tauri::generate_handler![greet])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}