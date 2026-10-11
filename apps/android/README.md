# ClashEdge for Android

Lightweight / Simple / Mihomo-based Clash client for Android.

## Status: experimental / planned (NOT released)

Android is **experimental / planned work**. It is **not part of the release
chain** and must not be advertised as a shipping platform until every
prerequisite below is done.

Current state (as of 2026-10-12):

- **Gradle wrapper checked in** (pinned 8.7): `assembleRelease` /
  `assembleDebug` build from a clean checkout after staging the rule assets.
- **No real Mihomo integration**: the Mihomo Android core (AAR / JNI) is a
  placeholder — `mihomo/MihomoCoreImpl.kt` is a no-op stub and the dependency
  coordinate in `gradle/libs.versions.toml` is still commented out. The app
  cannot provide a real VPN.
- **No signing config**: no release keystore / `signingConfig` exists.
- **Subscription import is a skeleton**: `SubscriptionRepository` keeps only `name/type/server`
  (credentials are dropped), so generated nodes cannot connect. Real node retention must reuse
  the Windows normalizer rules before this platform is shipped.
- **Release-chain guard**: CI fails if any release workflow references `apps/android`.

Prerequisites before Android can enter the release scope:

1. Reproducible build — **DONE**: the pinned gradle wrapper (`gradlew`,
   `gradlew.bat`, `gradle-wrapper.jar`, `gradle-wrapper.properties`) is
   checked in and verified against a clean toolchain (JDK 17, compileSdk 34).
2. Real Mihomo integration: pin the AAR/JNI artifact, replace the
   `MihomoCoreImpl` placeholder with real JNI calls, and verify the tunnel
   actually passes traffic.
3. Release signing: keystore + `signingConfig` (secrets only, never committed).
4. ProGuard / R8: rules verified against the real Mihomo integration (the
   current `app/proguard-rules.pro` only keeps the placeholder package).
5. Data / backup strategy: decide and document backup / uninstall behavior
   for profiles and settings.
6. Device testing: real-device matrix (Android 8+, OEM background-kill
   behavior, VpnService permission flows).

- **UI**: Kotlin + Jetpack Compose (Material 3)
- **Network**: Android `VpnService` → TUN → Mihomo core (AAR / JNI)
- **Rules**: same default rule set as Windows (pinned in `assets.lock.json`), same proxy-group names

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

> Version control matches Windows (`src-tauri/src/update/mod.rs::current_version`):
> the release tag is the single version source — no source file is bumped. The
> version is injected at build time via `CLASHEDGE_RELEASE_VERSION`; without it
> the build reports `dev`. `versionCode` derives deterministically from
> `x.y.z` as `x*1_000_000 + y*1_000 + z` (`dev` builds use 1).

```powershell
# 1) Stage the pinned rule set into assets (runs scripts/assets/prepare.ps1 first)
.\scripts\android\sync-rules.ps1

# 2) Build a debug APK (from apps/android)
.\gradlew.bat :app:assembleDebug

# 3) Release-stamped build (e.g. tag v0.1.0 -> inject 0.1.0)
$env:CLASHEDGE_RELEASE_VERSION = '0.1.0'
.\gradlew.bat :app:assembleRelease
```

Prerequisites: JDK 17, Android SDK (compileSdk 34), and a configured
`ANDROID_HOME` / `local.properties`.

## Mihomo core wiring

The Mihomo Android core (AAR / JNI) is not a standard Maven artifact and must be
pinned. The Android-only `mihomo/` module keeps the integration behind the
`MihomoCore` interface:

- `mihomo/MihomoCore.kt` — interface (startTun / start / stop / setMode / selectProxy).
- the no-op stub implementation in `mihomo/MihomoCore.kt` — replace with real JNI calls to the pinned AAR.
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
