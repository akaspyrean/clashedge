// build.rs — minimal build script for Tauri 2.
// Sidecar binaries (clash-edge-core.exe, go-tun2socks.exe, etc.) are NOT embedded in
// the Tauri binary; they are manually copied into App/ by scripts/windows/build-portable.ps1.
// create-tauri-app scaffolds an empty build.rs; Tauri's own resource embedding is handled
// via tauri.conf.json `bundle.resources`, so this file stays minimal.

fn main() {
    tauri_build::build();
    // tauri-build 只 rerun-if-changed tauri.conf.json / resources，不感知图标文件。
    // 若换图标（icons/icon.ico）不触发 build script 重跑，exe 会继续嵌旧图标。
    // 显式声明，让图标变更能重新嵌入 Windows 资源。
    println!("cargo:rerun-if-changed=icons/icon.ico");
    println!("cargo:rerun-if-changed=icons/icon.icns");
    println!("cargo:rerun-if-changed=icons/32x32.png");
    // Desktop/taskbar cat icons (tray icon 32x32.png 保持不变)
    println!("cargo:rerun-if-changed=icons/cat-32x32.png");
    println!("cargo:rerun-if-changed=icons/cat-128x128.png");
    println!("cargo:rerun-if-changed=icons/cat-256x256.png");
    println!("cargo:rerun-if-changed=icons/cat.icns");
    println!("cargo:rerun-if-changed=icons/cat.ico");
}
