# Release Tag Protection — GitHub Repository Ruleset Configuration

> This file documents the exact GitHub Repository Ruleset settings needed to
> enforce immutable release tags. If you have repository admin access, create
> this ruleset manually; if not, this file serves as the configuration checklist
> for "repo settings pending manual completion."

## Ruleset: Immutable Release Tags

### Settings

| Field | Value |
|-------|-------|
| Ruleset name | `Immutable release tags` |
| Enforcement status | Active |
| Target | Tags |
| Tag name pattern | `v*` |

### Restrictions

- [x] **Restrict deletions**: Enabled — no one can delete a `v*` tag
- [x] **Restrict updates**: Enabled — no one can move/force-push a `v*` tag
- [x] **Restrict who can create tags matching this pattern**: Admin only

### Bypass list

- Repository admins only (for emergency tag repair, e.g., the v1.0.8 tag
  restoration on 2026-08-29). All bypasses are auditable via the ruleset
  activity log.

### Second line of defense

The existing `scripts/release/Protect-ReleaseTag.ps1` (called by
`.github/workflows/release.yml`) remains as the CI-level guard. It enforces:

1. Tag commit must equal build commit (tag moved → FAIL)
2. Published releases are never deleted or overwritten
3. Draft recovery only for the exact same commit

The GitHub Ruleset is the **first** line of defense (server-side); the guard
script is the **second** (CI-side). Both must remain active.

## Creation Steps (GitHub UI)

1. Go to: Repository Settings → Rules → Rulesets → New ruleset
2. Set name: `Immutable release tags`
3. Enforcement: Active
4. Target: Tags (not Branches)
5. Tag pattern: `v*` (glob)
6. Enable "Restrict deletions"
7. Enable "Restrict updates"
8. Save

## Creation Steps (gh CLI, if available)

```bash
gh api repos/akaspyrean/clashedge/rulesets \
  -X POST \
  -f name="Immutable release tags" \
  -f target="tags" \
  -f enforcement="active" \
  -F conditions[ref_name][include][]="v*" \
  -F restrictions[delete]=true \
  -F restrictions[update]=true
```

If `gh` is not available or the API returns 403, log this as
"Repository settings pending manual completion" and proceed with code work.
