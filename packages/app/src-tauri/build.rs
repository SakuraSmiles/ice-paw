fn main() {
    // 关闭 Tauri 默认 app manifest，改由下方 embed_manifest() 统一注入。
    //
    // 为什么必须关掉 Tauri 的 manifest 再自己来：
    // tauri-winres 经 embed-resource 的 `cargo:rustc-link-arg-bins`（embed-resource 3.x
    // 的默认 compile()，仅 rustc≥1.50 + 有 bin 时）把 manifest .res **只链进 bin 目标**
    // （main exe）。lib 的 `#[test]` harness 拿不到 → loader 用 comctl32 v5 → 静态导入的
    // `TaskDialogIndirect` 解析不到 → STATUS_ENTRYPOINT_NOT_FOUND (0xC0000139)，binary
    // 在进入 main 前即被 loader 终止（长期被误记为「sodium DLL」问题，实则 sodium 是静态
    // 链接，binary 导入表里根本没有 libsodium.dll）。
    //
    // 能给 lib test harness 注 manifest 的 link-arg 只有「无后缀的全局 rustc-link-arg」
    // （-tests/-lib 都不覆盖 lib 的 #[test] harness，已 cargo clean 后实测确认）。但全局
    // 会同时命中 main exe；若 main exe 已有 Tauri 注入的 manifest，再嵌一份 RT_MANIFEST
    // 会触发链接器 CVT1100「资源重复 MANIFEST ID 1」。Tauri 默认 manifest 的内容恰是
    // Common-Controls v6（见下方常量，与 tauri-build 的 windows-app-manifest.xml 完全一致），
    // 故关掉它、让全局 link-arg 成为唯一 manifest 来源：main exe 与 test harness 都拿同一份，
    // main exe 只剩一份 RT_MANIFEST 不冲突；icon/version 仍由 tauri-winres 正常链入。
    let attrs = tauri_build::Attributes::new()
        .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
    tauri_build::try_build(attrs).expect("tauri-build failed");
    embed_manifest();
}

// Windows MSVC：用全局 link-arg 嵌入 Common-Controls v6 manifest。
//
// 适用对象：main exe（用 TaskDialogIndirect）与 lib `#[test]` harness（链接同一份 Tauri
// 运行时）都依赖它。外部 side-by-side `<exe>.manifest` 在 Win10/11 常被 loader 忽略，必须
// embedded。非 Windows / 非 MSVC 目标直接跳过（Linux/Mac 的 CI 测试不需要它）。
fn embed_manifest() {
    if !std::env::var("TARGET")
        .map(|t| t.contains("msvc"))
        .unwrap_or(false)
    {
        return;
    }
    let out_dir = match std::env::var("OUT_DIR") {
        Ok(d) => std::path::PathBuf::from(d),
        Err(_) => return,
    };
    let manifest_path = out_dir.join("ice-paw-common-controls-v6.manifest");
    if let Err(e) = std::fs::write(&manifest_path, COMMON_CONTROLS_V6_MANIFEST) {
        println!(
            "cargo:warning=写入 manifest 失败，Windows 下 binary 可能 0xC0000139: {e}"
        );
        return;
    }
    // 全局 rustc-link-arg：唯一能覆盖 lib `#[test]` harness 的 link-arg 形式。
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!(
        "cargo:rustc-link-arg=/MANIFESTINPUT:{}",
        manifest_path.display()
    );
}

const COMMON_CONTROLS_V6_MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity
        type="win32"
        name="Microsoft.Windows.Common-Controls"
        version="6.0.0.0"
        processorArchitecture="*"
        publicKeyToken="6595b64144ccf1df"
        language="*"/>
    </dependentAssembly>
  </dependency>
</assembly>
"#;
