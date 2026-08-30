# ClashEdge for Android

Lightweight / Simple / Mihomo-based Clash client for Android.

## Status: experimental / planned (NOT released)

Android is **experimental / planned work**. It is **not part of the release
chain** and must not be advertised as a shipping platform until every
prerequisite below is done.

Current state (as of 2026-08-28):

- **No gradle wrapper**: `gradlew` / `gradlew.bat` are not checked in, so the
  build documented below is not reproducible from a clean checkout.
- **No real Mihomo integration**: the Mihomo Android core (AAR / JNI) is a
  placeholder — `mihomo/MihomoCoreImpl.kt` is a no-op stub and the dependency
  coordinate in `gradle/libs.versions.toml` is still commented out. The app
  cannot provide a real VPN.
- **No signing config**: no release keystore / `signingConfig` exists.

Prerequisites before Android can enter the release scope:

1. Reproducible build: check in a pinned gradle wrapper (`gradlew`,
   `gradlew.bat`, `gradle-wrapper.jar` / `gradle-wrapper.properties`) and
   verify a clean-checkout build.
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

> The gradle wrapper is **not checked in yet** — `gradlew.bat` below cannot
> run until prerequisite 1 (above) is done. Generate it locally with
> `gradle wrapper` after installing a Gradle distribution.

```powershell
# 1) Materialize the shared rule set into assets
.\scripts\android\sync-rules.ps1

# 2) Build a debug APK (from apps/android; requires the wrapper, see above)
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
