# Protect-ReleaseTag.ps1 - Immutable release guard (Release workflow safety gate).
#
# Goal: a re-run of the Release workflow against an existing tag must never
#   1. delete or overwrite an already PUBLISHED (non-draft) release,
#   2. move/reuse a tag that points at a different commit,
#   3. delete/recreate a draft that does not belong to the exact commit being built.
#
# Decision matrix (exit code 1 on every REJECT):
#   tag commit != build commit                    -> REJECT (tag moved/reused)
#   no existing release                           -> CREATE
#   release exists, published (not a draft)       -> REJECT
#   release exists, draft, targetCommitish != sha -> REJECT
#   release exists, draft, targetCommitish == sha -> DELETE_RECREATE_DRAFT
#     (tag commit has already been verified == build commit by Check 1)
#
# Side effects: read-only by default. In non-WhatIfOnly mode the only write
#   action is deleting a draft whose commit was proven identical (safe recovery
#   of a failed run). The Release workflow calls this script with -WhatIfOnly
#   -SetGitHubOutput and performs the actual delete/recreate in its
#   "Create draft release" step, so the guard step itself stays side-effect free.
#
# Test seams (unit-tested by scripts/release/test-release-guards.ps1; when a
#   seam is provided, the corresponding gh/git call is never made):
#   -ReleaseViewJson : canned `gh release view <tag> --json isDraft,targetCommitish`
#                      output. Empty string means "no such release".
#   -TagCommitJson   : canned `gh api repos/<repo>/git/ref/tags/<tag>` output.
#                      object.sha is interpreted as the DEREFERENCED commit the
#                      tag ultimately points to (lightweight or annotated tag).
#   -MockGh          : scriptblock standing in for every gh invocation; receives
#                      the gh argument array, returns raw output (or $null on
#                      failure). Takes precedence over the JSON seams.
#
# Shallow clones: the Release workflow checks out with fetch-depth: 0, so the
#   full tag object graph is present locally and `git rev-parse` resolves the
#   tag directly. As a safety net for shallow clones (running this script
#   outside the workflow) we fetch the single tag ref on demand, then fall back
#   to the GitHub API (requires -Repo and GH_TOKEN).
#
# ASCII-only; Windows PowerShell 5.1 compatible.

[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)]
  [ValidatePattern('^v')]
  [string]$Tag,

  [Parameter(Mandatory = $true)]
  [ValidatePattern('^[0-9a-fA-F]{40}$')]
  [string]$Sha,

  [string]$Repo,

  [AllowEmptyString()]
  [string]$ReleaseViewJson,

  [AllowEmptyString()]
  [string]$TagCommitJson,

  [scriptblock]$MockGh,

  [switch]$WhatIfOnly,

  [switch]$SetGitHubOutput
)

function Set-Decision {
  param([string]$Value)
  # Machine-readable decision line (parsed by tests and by the workflow).
  Write-Output ("GUARD_DECISION={0}" -f $Value)
  if ($SetGitHubOutput -and $env:GITHUB_OUTPUT) {
    Add-Content -Path $env:GITHUB_OUTPUT -Value ("decision={0}" -f $Value)
  }
}

function Reject {
  param([string]$Reason)
  Set-Decision 'REJECT'
  Write-Host ("::error::{0}" -f $Reason)
  exit 1
}

# NOTE: the seam checks below must read SCRIPT-scope variables, not
# $PSBoundParameters: inside a function, $PSBoundParameters is the function's
# own (empty) parameter table, which silently disabled every test seam and made
# the guard read the real git refs instead of the injected JSON.
$script:HasReleaseViewSeam = $PSBoundParameters.ContainsKey('ReleaseViewJson')
$script:HasTagCommitSeam = $PSBoundParameters.ContainsKey('TagCommitJson')
$script:MockGhSeam = $MockGh
$script:ReleaseViewJsonSeam = $ReleaseViewJson
$script:TagCommitJsonSeam = $TagCommitJson

function Invoke-Gh {
  param([string[]]$GhArgs)
  if ($script:MockGhSeam) {
    $out = & $script:MockGhSeam $GhArgs
    if ($null -eq $out) { return $null }
    return (@($out) -join "`n")
  }
  $out = & gh @GhArgs 2>$null
  if ($LASTEXITCODE -ne 0 -or $null -eq $out) { return $null }
  return (@($out) -join "`n")
}

function Get-ReleaseViewRaw {
  if ($script:MockGhSeam) {
    return (Invoke-Gh @('release', 'view', $Tag, '--json', 'isDraft,targetCommitish'))
  }
  if ($script:HasReleaseViewSeam) { return $script:ReleaseViewJsonSeam }
  return (Invoke-Gh @('release', 'view', $Tag, '--json', 'isDraft,targetCommitish'))
}

function Get-TagCommitRaw {
  if ($script:MockGhSeam) {
    return (Invoke-Gh @('api', ('repos/{0}/git/ref/tags/{1}' -f $Repo, $Tag)))
  }
  if ($script:HasTagCommitSeam) { return $script:TagCommitJsonSeam }

  # Real mode, path 1: local git object database. The Release workflow checks
  # out with fetch-depth: 0 so refs/tags/<tag> and its target commit are
  # present locally; `^{commit}` also peels annotated tags.
  $rev = & git rev-parse --verify ('refs/tags/{0}^{{commit}}' -f $Tag) 2>$null
  if ($LASTEXITCODE -eq 0 -and $rev) { return ([string](@($rev)[0])).Trim() }

  # Real mode, path 2 (shallow-clone safety net): fetch exactly this tag ref
  # on demand, then retry the local resolution.
  Write-Host "Tag ref not resolvable locally; fetching refs/tags/$Tag on demand ..."
  & git fetch origin ('+refs/tags/{0}:refs/tags/{1}' -f $Tag, $Tag) --no-tags 2>$null | Out-Null
  $rev = & git rev-parse --verify ('refs/tags/{0}^{{commit}}' -f $Tag) 2>$null
  if ($LASTEXITCODE -eq 0 -and $rev) { return ([string](@($rev)[0])).Trim() }

  # Real mode, path 3: GitHub API. For annotated tags the ref points at the tag
  # object, so dereference once more through the tag-object endpoint.
  if ($Repo) {
    Write-Host "Falling back to GitHub API to resolve tag commit ..."
    $raw = Invoke-Gh @('api', ('repos/{0}/git/ref/tags/{1}' -f $Repo, $Tag))
    if ($raw) {
      $refObj = $null
      try { $refObj = $raw | ConvertFrom-Json } catch { $refObj = $null }
      if ($refObj -and $refObj.object -and $refObj.object.sha) {
        if ($refObj.object.type -eq 'tag') {
          $rawTag = Invoke-Gh @('api', ('repos/{0}/git/tags/{1}' -f $Repo, $refObj.object.sha))
          if ($rawTag) {
            $tagObj = $null
            try { $tagObj = $rawTag | ConvertFrom-Json } catch { $tagObj = $null }
            if ($tagObj -and $tagObj.object -and $tagObj.object.sha) {
              return ([string]$tagObj.object.sha)
            }
          }
          return $null
        }
        return ([string]$refObj.object.sha)
      }
    }
  }
  return $null
}

function Resolve-CommitFromRaw {
  param([string]$Raw)
  if ([string]::IsNullOrWhiteSpace($Raw)) { return $null }
  $text = $Raw.Trim()
  if ($text.StartsWith('{')) {
    $obj = $null
    try { $obj = $text | ConvertFrom-Json } catch { $obj = $null }
    if ($obj -and $obj.object -and $obj.object.sha) {
      return ([string]$obj.object.sha).ToLowerInvariant()
    }
    return $null
  }
  return $text.ToLowerInvariant()
}

# --------------------------------------------------------------- checks

if ([string]::IsNullOrWhiteSpace($Tag)) { Reject "Parameter -Tag must not be empty." }

$buildCommit = $Sha.Trim().ToLowerInvariant()

# --- Check 1: the tag must point at exactly the commit being built. ---
$tagCommit = Resolve-CommitFromRaw (Get-TagCommitRaw)
if (-not $tagCommit) {
  Reject ("Tag '{0}' could not be resolved to a commit (missing or unreachable). Refusing to release: the same tag must always point at the same commit." -f $Tag)
}
if ($tagCommit -ne $buildCommit) {
  Reject ("IMMUTABLE TAG VIOLATION: tag '{0}' points at commit {1}, but this build is {2}. Moving/reusing an existing tag is forbidden - cut a new version tag instead." -f $Tag, $tagCommit, $buildCommit)
}

# --- Check 2: never touch an already published release. ---
$viewRaw = Get-ReleaseViewRaw
$release = $null
if (-not [string]::IsNullOrWhiteSpace($viewRaw)) {
  $release = $null
  try { $release = $viewRaw | ConvertFrom-Json } catch { $release = $null }
}

if (-not $release) {
  Set-Decision 'CREATE'
  Write-Host ("Guard OK: no existing release for '{0}' and tag commit == build commit ({1}). Proceeding to create the draft." -f $Tag, $buildCommit)
  exit 0
}

$isDraft = $false
if ($release.PSObject.Properties['isDraft'] -and $release.isDraft) { $isDraft = $true }

if (-not $isDraft) {
  Reject ("Release '{0}' already exists and is PUBLISHED (not a draft). Published releases are immutable; this workflow must never delete or overwrite them. Re-run with a new version tag instead." -f $Tag)
}

$target = ''
if ($release.PSObject.Properties['targetCommitish'] -and $release.targetCommitish) {
  $target = ([string]$release.targetCommitish).Trim()
}
if ($target.ToLowerInvariant() -ne $buildCommit) {
  Reject ("DRAFT COMMIT MISMATCH: existing draft '{0}' targets '{1}', but this build is {2}. Only a draft of the exact same commit may be deleted and recreated." -f $Tag, $target, $buildCommit)
}

# --- Unpublished draft of the exact build commit: safe recovery allowed. ---
Set-Decision 'DELETE_RECREATE_DRAFT'
if ($WhatIfOnly) {
  Write-Host ("Guard OK: '{0}' is an unpublished draft of the exact build commit {1}. Delete+recreate is allowed (WhatIfOnly: nothing was deleted)." -f $Tag, $buildCommit)
  exit 0
}

Write-Host ("Deleting stale draft '{0}' (same commit {1}) ..." -f $Tag, $buildCommit)
& gh release delete $Tag --yes
if ($LASTEXITCODE -ne 0) {
  Write-Host ("::error::gh release delete failed for '{0}'" -f $Tag)
  exit 1
}
Write-Host "Stale draft deleted; recreate may proceed."
exit 0
