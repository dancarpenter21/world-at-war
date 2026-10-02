[CmdletBinding()]
param(
    [switch]$VerifyOnly
)

$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
$dependencyPath = [System.IO.Path]::GetFullPath((Join-Path $repositoryRoot '../c3mesh'))
$revision = (Get-Content -LiteralPath (Join-Path $repositoryRoot 'c3mesh-revision.txt') -Raw).Trim()
if ($revision -notmatch '^[0-9a-f]{40}$') {
    throw 'c3mesh-revision.txt must contain one complete Git commit SHA.'
}

function Invoke-DependencyGit {
    param([string[]]$GitArguments)
    & git -C $dependencyPath @GitArguments
    if ($LASTEXITCODE -ne 0) {
        throw "c3mesh Git command failed (exit $LASTEXITCODE)."
    }
}

if (-not (Test-Path -LiteralPath $dependencyPath)) {
    if ($VerifyOnly) { throw "Missing dependency: $dependencyPath. Run scripts/setup-c3mesh.ps1." }
    & git clone 'https://github.com/dancarpenter21/c3mesh.git' $dependencyPath
    if ($LASTEXITCODE -ne 0) { throw 'Could not clone c3mesh. Check your GitHub read access.' }
}
$repositoryTop = (Invoke-DependencyGit -GitArguments @('rev-parse', '--show-toplevel')).Trim()
if ([System.IO.Path]::GetFullPath($repositoryTop) -ne $dependencyPath) {
    throw "The dependency path must be the c3mesh repository root: $dependencyPath."
}

$currentRevision = Invoke-DependencyGit -GitArguments @('rev-parse', '--verify', 'HEAD')
if ($VerifyOnly) {
    if ($currentRevision.Trim() -ne $revision) {
        throw "c3mesh is at $currentRevision; this checkpoint expects $revision."
    }
    if (Invoke-DependencyGit -GitArguments @('status', '--porcelain')) {
        throw 'c3mesh has local changes. The pinned dependency must be clean for verification.'
    }
    Write-Host "Verified c3mesh $revision at $dependencyPath"
    exit 0
}

if ($currentRevision.Trim() -ne $revision) {
    if (Invoke-DependencyGit -GitArguments @('status', '--porcelain')) {
        throw 'c3mesh has local changes; commit or stash them before selecting the pinned revision.'
    }
    Invoke-DependencyGit -GitArguments @('fetch', 'origin', $revision)
    Invoke-DependencyGit -GitArguments @('checkout', '--detach', $revision)
}
Write-Host "c3mesh $revision is available at $dependencyPath"
