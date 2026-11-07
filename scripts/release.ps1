<#
.SYNOPSIS
  Builds a release of AI SubHub into dist\ and, with -Publish, publishes it on GitHub.

.DESCRIPTION
  1. Checks that CHANGELOG.md has a "## X.Y.Z" section with bullets for the version.
  2. Sets the version in [workspace.package] of Cargo.toml.
  3. Runs cargo test --workspace and cargo build --release.
  4. Copies target\release\ai-subhub.exe to dist\ and writes dist\ai-subhub.exe.sha256.
  5. Only with -Publish: git tag vX.Y.Z, git push of that tag, and
     gh release create vX.Y.Z dist\* with the CHANGELOG section as the notes.

  The app updates itself from the release assets "ai-subhub.exe" and
  "ai-subhub.exe.sha256" (docs/features/app.md, "Updates"); keep those names.

  Versioning: patch bump per release; the minor version only when the owner says a feature is
  complete (CLAUDE.md, "Releasing").

  Typical flow:
    scripts\release.ps1 -Version 0.1.1            # bump, test, build, dist\
    git commit -am "Release 0.1.1"                # the tag must point at the bump
    scripts\release.ps1 -Version 0.1.1 -Publish   # same version again: nothing to bump, then publish

.EXAMPLE
  scripts\release.ps1 -Version 0.1.1
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)]
  [ValidatePattern('^\d+\.\d+\.\d+$')]
  [string]$Version,

  # Tag and publish on GitHub (needs a clean, committed tree and the gh CLI, signed in).
  [switch]$Publish
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$exeName = 'ai-subhub.exe'
$tag = "v$Version"

function Invoke-Native([string]$What, [scriptblock]$Command) {
  & $Command
  if ($LASTEXITCODE -ne 0) { throw "$What failed (exit code $LASTEXITCODE)." }
}

# --- 1. CHANGELOG section for this version -------------------------------------------------
$changelogPath = Join-Path $root 'CHANGELOG.md'
if (-not (Test-Path $changelogPath)) { throw 'CHANGELOG.md not found.' }
$lines = Get-Content -LiteralPath $changelogPath -Encoding UTF8
$start = -1
for ($i = 0; $i -lt $lines.Count; $i++) {
  # "## 0.1.1 – 2026-11-20" (en dash, em dash or hyphen; the date is optional)
  if ($lines[$i] -match "^##\s+v?$([regex]::Escape($Version))(\s|$)") { $start = $i; break }
}
if ($start -lt 0) {
  throw "CHANGELOG.md has no '## $Version - <date>' section. Rename '## Unreleased' (or add the section) with 3-6 short bullets first."
}
$notes = New-Object System.Collections.Generic.List[string]
for ($i = $start + 1; $i -lt $lines.Count; $i++) {
  if ($lines[$i] -match '^#{1,2}\s') { break }
  $notes.Add($lines[$i])
}
$bullets = @($notes | Where-Object { $_ -match '^\s*[-*]\s+\S' })
if ($bullets.Count -eq 0) { throw "The '## $Version' section in CHANGELOG.md has no bullets." }
$notesText = ($notes -join "`n").Trim()
Write-Host "CHANGELOG: $($bullets.Count) bullet(s) for $Version."

# --- 2. Version in Cargo.toml --------------------------------------------------------------
$cargoPath = Join-Path $root 'Cargo.toml'
$cargo = Get-Content -LiteralPath $cargoPath -Encoding UTF8
$inSection = $false
$current = $null
$changed = $false
for ($i = 0; $i -lt $cargo.Count; $i++) {
  if ($cargo[$i] -match '^\s*\[') { $inSection = ($cargo[$i].Trim() -eq '[workspace.package]') ; continue }
  if ($inSection -and $cargo[$i] -match '^\s*version\s*=\s*"([^"]*)"') {
    $current = $Matches[1]
    if ($current -ne $Version) {
      $cargo[$i] = $cargo[$i] -replace '"[^"]*"', "`"$Version`""
      $changed = $true
    }
    break
  }
}
if ($null -eq $current) { throw 'No version found in [workspace.package] of Cargo.toml.' }
if ([version]$Version -lt [version]$current) {
  throw "Version $Version is lower than the current $current (no downgrades)."
}
if ($changed) {
  # Write without a BOM, keep the file's line endings.
  $eol = if ((Get-Content -LiteralPath $cargoPath -Raw) -match "`r`n") { "`r`n" } else { "`n" }
  [System.IO.File]::WriteAllText($cargoPath, (($cargo -join $eol) + $eol), (New-Object System.Text.UTF8Encoding($false)))
  Write-Host "Cargo.toml: $current -> $Version"
} else {
  Write-Host "Cargo.toml already at $Version."
}

# --- 3. Test and build ---------------------------------------------------------------------
Invoke-Native 'cargo test' { cargo test --workspace }
Invoke-Native 'cargo build --release' { cargo build --release }

# --- 4. dist\ ------------------------------------------------------------------------------
$built = Join-Path $root "target\release\$exeName"
if (-not (Test-Path $built)) { throw "Build output not found: $built" }
$dist = Join-Path $root 'dist'
if (Test-Path $dist) { Remove-Item -LiteralPath $dist -Recurse -Force }
New-Item -ItemType Directory -Path $dist | Out-Null
$exe = Join-Path $dist $exeName
Copy-Item -LiteralPath $built -Destination $exe
$hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $exe).Hash.ToLower()
# "<hex>  <file name>", ASCII, no BOM: the app reads the first token.
[System.IO.File]::WriteAllText("$exe.sha256", "$hash  $exeName`n", (New-Object System.Text.UTF8Encoding($false)))
Write-Host "dist\: $exeName ($([math]::Round((Get-Item $exe).Length / 1MB, 1)) MB), sha256 $hash"

if (-not $Publish) {
  Write-Host "Built $tag. Commit the version bump, then run again with -Publish to tag and publish."
  return
}

# --- 5. Publish (only with -Publish) -------------------------------------------------------
$dirty = git status --porcelain
if ($LASTEXITCODE -ne 0) { throw 'git status failed.' }
if ($dirty) {
  throw "The working tree has uncommitted changes. Commit the release (Cargo.toml, Cargo.lock, CHANGELOG.md) so the tag points at it, then run again with -Publish.`n$($dirty -join "`n")"
}
git rev-parse -q --verify "refs/tags/$tag" *> $null
if ($LASTEXITCODE -eq 0) { throw "Tag $tag already exists." }
if (-not (Get-Command gh -ErrorAction SilentlyContinue)) { throw 'The gh CLI is not installed.' }

$notesFile = Join-Path ([System.IO.Path]::GetTempPath()) "subhub-release-notes-$Version.md"
[System.IO.File]::WriteAllText($notesFile, $notesText + "`n", (New-Object System.Text.UTF8Encoding($false)))
try {
  Invoke-Native 'git tag' { git tag $tag }
  # The release must use this exact commit, so the tag goes to the remote first.
  Invoke-Native 'git push (tag)' { git push origin $tag }
  Invoke-Native 'gh release create' {
    # Listed one by one: PowerShell does not expand wildcards for native commands.
    $assets = @(Get-ChildItem -LiteralPath $dist -File | ForEach-Object { $_.FullName })
    gh release create $tag @assets --title $tag --notes-file $notesFile
  }
} finally {
  Remove-Item -LiteralPath $notesFile -Force -ErrorAction SilentlyContinue
}
Write-Host "Published $tag."
