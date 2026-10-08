# Third-Party Notices

ClashEdge itself is licensed under the MIT License (see `LICENSE`). The
following third-party components are bundled in the portable distribution
and/or referenced by the source tree. Each component retains its own license
and copyright. This notice does not change or relicense any third-party
component.

| Component | Role | License | Upstream |
| --- | --- | --- | --- |
| mihomo (`App/ClashEdge/sidecar/mihomo-win64.exe`) | Proxy core | GPL-3.0 | https://github.com/MetaCubeX/mihomo |
| wintun.dll | TUN driver | Wintun prebuilt-binaries license (signed by WireGuard LLC) | https://www.wintun.net/ |
| GeoIP.dat / GeoSite.dat | Rule-set GeoData | See meta-rules-dat notices | https://github.com/MetaCubeX/meta-rules-dat |
| Country.mmdb | MaxMind GeoLite2 mirror | GeoLite2 EULA / CC BY-SA 4.0 | https://www.maxmind.com/en/geolite2/eula |
| Built-in rule sets (direct/proxy/media/ai/ad) | Default rules | Derived from rule-set projects; see data file notices | e.g. Loyalsoldier/clash-rules |
| Tauri 2 (Rust + JS) | App shell framework (Windows) | MIT / Apache-2.0 (dual) | https://github.com/tauri-apps/tauri |
| Vue 3, Pinia, Vue Router, Vite, Element Plus, vue-i18n | Frontend libraries (Windows) | MIT (each project's own license) | https://vuejs.org/ etc. |
| Mihomo Android core (AAR / JNI) | Proxy core + TUN adapter (Android, not released) | GPL-3.0 | https://github.com/MetaCubeX/mihomo |
| Kotlin, Jetpack Compose, AndroidX | Android UI / runtime (Android, not released) | Apache-2.0 (each project's own license) | https://developer.android.com/ |

## GPL-3.0 component: mihomo

The portable package contains the **unmodified** official mihomo release binary
(version and download URL are pinned in `assets.lock.json` and recorded in the
package as `Other/Licenses/MIHOMO-SOURCE.txt`). The complete corresponding source
code of exactly that version is available at
`https://github.com/MetaCubeX/mihomo/tree/v<version>` (the tag named in
`MIHOMO-SOURCE.txt`), and on request from the ClashEdge maintainers for at least
three years after the release. The full GPL-3.0 text is in
`Other/Licenses/GPL-3.0.txt`. ClashEdge's own code (MIT) runs mihomo as a separate
process and does not link against it.

## Notes

- The frontend npm packages ship their own licenses inside `node_modules`
  (not committed to this repository).
- GeoData files are generated rule/geolocation data with per-data licenses.
  Refer to the respective upstream repositories for full license texts.
- `Country.mmdb` is a GeoLite2 mirror: keep the attribution above, and re-check the
  current GeoLite2 EULA redistribution terms before each release.
