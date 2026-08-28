# Third-Party Notices

ClashEdge itself is licensed under the MIT License (see `LICENSE`). The
following third-party components are bundled in the portable distribution
and/or referenced by the source tree. Each component retains its own license
and copyright. This notice does not change or relicense any third-party
component.

| Component | Role | License | Upstream |
| --- | --- | --- | --- |
| mihomo (clash-edge-core.exe / mihomo-win64.exe) | Proxy core | GPL-3.0 | https://github.com/MetaCubeX/mihomo |
| go-tun2socks | TUN mode helper | Apache-2.0 | https://github.com/xjasonlyu/tun2socks |
| wintun.dll | TUN driver | See Wintun license | https://www.wintun.net/ |
| EnableLoopback.exe | Loopback enabler for UWP loopback exemption | See upstream distribution | Distributed with the proxy core package |
| GeoIP.dat / GeoSite.dat | Rule-set GeoData | See meta-rules-dat notices | https://github.com/MetaCubeX/meta-rules-dat |
| Country.mmdb | MaxMind GeoLite2 mirror | GeoLite2 EULA / CC BY-SA 4.0 | https://www.maxmind.com/en/geolite2/eula |
| Built-in rule sets (direct/proxy/media/ai/ad) | Default rules | Derived from rule-set projects; see data file notices | e.g. Loyalsoldier/clash-rules |
| Tauri 2 (Rust + JS) | App shell framework (Windows) | MIT / Apache-2.0 (dual) | https://github.com/tauri-apps/tauri |
| Vue 3, Pinia, Vue Router, Vite, Element Plus, vue-i18n | Frontend libraries (Windows) | MIT (each project's own license) | https://vuejs.org/ etc. |
| Mihomo Android core (AAR / JNI) | Proxy core + TUN adapter (Android) | GPL-3.0 | https://github.com/MetaCubeX/mihomo |
| Kotlin, Jetpack Compose, AndroidX | Android UI / runtime (Android) | Apache-2.0 (each project's own license) | https://developer.android.com/ |

Notes:

- The mihomo binary is distributed unmodified as a GPL-3.0 component. ClashEdge
  does not distribute modified GPL source; its own code is MIT.
- The frontend npm packages ship their own licenses inside `node_modules`
  (not committed to this repository).
- GeoData files are generated rule/geolocation data with per-data licenses.
  Refer to the respective upstream repositories for full license texts.