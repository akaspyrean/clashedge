# ClashEdge for Android

Lightweight / Simple / Mihomo-based Clash client for Android.

- **UI**: Kotlin + Jetpack Compose (Material 3)
- **Network**: Android `VpnService` → TUN → Mihomo core (AAR / JNI)
- **Rules**: same default rule set as Windows (`shared/rules`), same proxy-group names

## Architecture

```
UI (Compose)
  ↓
Application Service (ProxyCoordinator — single source of truth)
  ↓
VpnService / Mihomo Adapter (MihomoCore)
  ↓
Mihomo Core (rules, DNS, groups, nodes)
```

The UI never talks to the core directly; it renders `ProxyCoordinator`'s
`StateFlow` (the real runtime state). Any abnormal exit / killed service flips the
state to `STOPPED`/`ERROR` — the UI can never show a fake "connected".

## Build

```powershell
# 1) Materialize the shared rule set into assets
.\scripts\android\sync-rules.ps1

# 2) Build a debug APK (from apps/android)
.\gradlew.bat :app:assembleDebug
```

Prerequisites: JDK 17, Android SDK (compileSdk 34), and a configured
`ANDROID_HOME` / `local.properties`.

## Mihomo core wiring

The Mihomo Android core (AAR / JNI) is not a standard Maven artifact and must be
pinned. The Android-only `mihomo/` module keeps the integration behind the
`MihomoCore` interface:

- `mihomo/MihomoCore.kt` — interface (startTun / start / stop / setMode / selectProxy).
- `mihomo/MihomoCoreImpl.kt` — placeholder; wire real JNI calls to the pinned AAR.
- The dependency coordinate in `gradle/libs.versions.toml` is commented out until
  the artifact is pinned (JitPack / custom maven).

Until the AAR is wired, the app runs the state machine with a no-op core so the
VPN flow (permission → tunnel → logs → UI state) is fully testable.

## MVP scope includes

Import / refresh subscription, start / stop proxy, VpnService, Rule / Global /
Direct, node & group selection, auto-select (url-test), traffic state wiring,
basic logs, Chinese / English, light / dark.

Not (yet): Root mode, TProxy, BPF, scripting (JS/Lua), WebDAV, Sub-Store, cloud
sync, multi-core switching, complex rule editor.
