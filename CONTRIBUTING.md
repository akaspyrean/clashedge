# Contributing to ClashEdge

## Quick start

```powershell
# Clone
git clone https://github.com/akaspyrean/clashedge.git
cd clashedge

# Windows app (Tauri 2: Rust + Vue 3)
cd apps/windows
npm install
cd src-tauri && cargo build
```

## Branch naming

Create branches from `main` using conventional prefixes:

| Prefix | Use |
|--------|-----|
| `feat/` | New feature |
| `fix/` | Bug fix |
| `docs/` | Documentation |
| `refactor/` | Code restructuring |
| `build/` | Build system or dependency change |

## Commit messages

Follow [Conventional Commits](https://www.conventionalcommits.org/):

```
feat(ui): add node latency display
fix(release): tag-driven versioning
docs: update README screenshots
build(deps): bump tauri from 2.8 to 2.9
```

## Pull request process

1. One PR = one logical change. Don't mix unrelated refactors with features.
2. PR title follows conventional commits (English). Human-facing docs use Chinese.
3. Squash merge is the default — your PR will be squashed into one commit.
4. CI must pass before merging (`lint-test` job).
5. If your change touches generated files (`tokens.css`, `Tokens.kt`), run `npm run tokens` first — never hand-edit generated files.

## Local quality check

```powershell
# Fast check (what CI runs on PRs)
pwsh scripts/ci/quality.ps1 -Fast

# Full check (what CI runs on main)
pwsh scripts/ci/quality.ps1
```

## Code style

### Rust (src-tauri)
- `cargo fmt` before committing
- `cargo clippy -- -D warnings` — warnings are errors
- Toolchain pinned in `rust-toolchain.toml` (1.97.1)

### Vue 3 (frontend)
- Views and components may only reference `--ce-*` design tokens — no hex or `rgb()` literals
- `npm test` (Vitest) must pass

### Design tokens
- `design/tokens.json` is the single source
- Run `npm run tokens` to regenerate `tokens.css` and `Tokens.kt`
- Commit generated output alongside token changes

## Security

Do not disclose vulnerabilities in public Issues. Use **Security → Report a vulnerability** (see [SECURITY.md](SECURITY.md)).
