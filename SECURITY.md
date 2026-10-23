# Security Policy

## Reporting a Vulnerability

Please **do not** open a public Issue for security vulnerabilities.

Use GitHub's private vulnerability reporting instead:

**Security tab → Report a vulnerability**

Or email: security@clashedge.local (replace with your actual address)

## Response Time

| Step | Target |
|------|--------|
| Acknowledge report | 48 hours |
| Initial assessment | 5 business days |
| Fix or mitigation | 30 days (severity dependent) |

## Scope

- ClashEdge application code (`apps/`, `scripts/`, `packaging/`)
- CI/CD pipeline (`.github/workflows/`)
- Bundled rule sets and configuration (`external/` repo)

## Out of scope

- Mihomo core vulnerabilities (report to [MetaCubeX/mihomo](https://github.com/MetaCubeX/mihomo))
- Third-party dependency CVEs (handled via Dependabot + `cargo audit`)

## Build provenance

Every release artifact includes:

- SHA256 checksums (`.sha256` files)
- Minisign signature (`.minisig` file)
- Build provenance attestation (GitHub `attest-build-provenance`)
- Authenticode signature (Windows binaries)

Verify a downloaded release:

```powershell
# Check SHA256
Get-FileHash ClashEdge-portable-win64.zip -Algorithm SHA256
# Compare with ClashEdge-portable-win64.zip.sha256
```
