[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$ReleaseTag,
    [Parameter(Mandatory = $true)]
    [string]$EventName,
    [Parameter(Mandatory = $true)]
    [string]$GitRef
)

$ErrorActionPreference = "Stop"
$utf8NoBom = [System.Text.UTF8Encoding]::new($false)

function Invoke-Git {
    param([Parameter(Mandatory = $true)][string[]]$GitArgs)

    $output = & git @GitArgs 2>&1
    $exitCode = $LASTEXITCODE
    if ($exitCode -ne 0) {
        throw "git $($GitArgs -join ' ') failed with exit code $exitCode. $($output -join [Environment]::NewLine)"
    }

    return [string]::Join([Environment]::NewLine, @($output)).Trim()
}

function Write-JsonFile {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][object]$Value
    )

    $json = ConvertTo-Json -InputObject $Value -Depth 100
    [System.IO.File]::WriteAllText((Join-Path $repoRoot $Path), $json + [Environment]::NewLine, $utf8NoBom)
}

function Get-VersionParts {
    param([Parameter(Mandatory = $true)][string]$Value)

    $match = [regex]::Match(
        $Value,
        '^(?<major>0|[1-9]\d*)\.(?<minor>0|[1-9]\d*)\.(?<patch>0|[1-9]\d*)(?:-(?<pre>[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$'
    )
    if (-not $match.Success) {
        throw "Invalid semantic application version: $Value"
    }

    return [pscustomobject]@{
        Core = [version]::new(
            [int]$match.Groups["major"].Value,
            [int]$match.Groups["minor"].Value,
            [int]$match.Groups["patch"].Value
        )
        PreRelease = $match.Groups["pre"].Value
    }
}

function Compare-AppVersions {
    param(
        [Parameter(Mandatory = $true)][string]$Left,
        [Parameter(Mandatory = $true)][string]$Right
    )

    $leftParts = Get-VersionParts $Left
    $rightParts = Get-VersionParts $Right
    $coreComparison = $leftParts.Core.CompareTo($rightParts.Core)
    if ($coreComparison -ne 0) {
        return $coreComparison
    }

    if ($leftParts.PreRelease -eq $rightParts.PreRelease) {
        return 0
    }
    if ([string]::IsNullOrEmpty($leftParts.PreRelease)) {
        return 1
    }
    if ([string]::IsNullOrEmpty($rightParts.PreRelease)) {
        return -1
    }

    return [string]::CompareOrdinal($leftParts.PreRelease, $rightParts.PreRelease)
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Set-Location $repoRoot

if ($ReleaseTag -notmatch '^v(?<version>(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?)$') {
    throw "Invalid release tag '$ReleaseTag'. Use vMAJOR.MINOR.PATCH, optionally followed by a prerelease suffix."
}
$releaseVersion = $Matches["version"]
$null = Get-VersionParts $releaseVersion

if ($EventName -eq "workflow_dispatch") {
    if ($GitRef -ne "refs/heads/main") {
        throw "Manual releases must be dispatched from main. Select main in the workflow branch selector."
    }
}
elseif ($EventName -eq "push") {
    if ($GitRef -ne "refs/tags/$ReleaseTag") {
        throw "Release tag '$ReleaseTag' does not match triggering ref '$GitRef'."
    }
}
else {
    throw "Unsupported release event: $EventName"
}

$package = Get-Content "package.json" -Raw | ConvertFrom-Json -AsHashtable
$npmLock = Get-Content "package-lock.json" -Raw | ConvertFrom-Json -AsHashtable
$tauri = Get-Content "src-tauri/tauri.conf.json" -Raw | ConvertFrom-Json -AsHashtable
$cargoToml = Get-Content "src-tauri/Cargo.toml" -Raw
$cargoLock = Get-Content "src-tauri/Cargo.lock" -Raw

$cargoVersionMatch = [regex]::Match($cargoToml, '(?ms)^\[package\].*?^version\s*=\s*"([^"]+)"')
if (-not $cargoVersionMatch.Success) {
    throw "Could not read the Cargo package version."
}
$cargoLockPattern = '(?ms)(^\[\[package\]\]\r?\nname = "spindle"\r?\nversion = ")[^"]+(")'
$cargoLockMatches = [regex]::Matches($cargoLock, $cargoLockPattern)
if ($cargoLockMatches.Count -ne 1) {
    throw "Expected exactly one spindle package entry in Cargo.lock; found $($cargoLockMatches.Count)."
}

$npmRoot = $npmLock.packages[""]
if ($null -eq $npmRoot) {
    throw "package-lock.json is missing its root package entry."
}
$cargoLockVersion = [regex]::Match($cargoLockMatches[0].Value, 'version = "([^"]+)"').Groups[1].Value
$currentVersions = @(
    [string]$package.version,
    [string]$npmLock.version,
    [string]$npmRoot.version,
    [string]$tauri.version,
    [string]$cargoVersionMatch.Groups[1].Value,
    [string]$cargoLockVersion
)

if (($currentVersions | Select-Object -Unique).Count -ne 1) {
    throw "Application versions are inconsistent: $($currentVersions -join ', '). Reconcile them before releasing."
}
$currentVersion = [string]$package.version
$null = Get-VersionParts $currentVersion

if ($tauri.bundle.windows.nsis.installMode -ne "perMachine") {
    throw "NSIS installMode must be perMachine so installs and updates target Program Files."
}
if ($tauri.plugins.updater.windows.installMode -ne "passive") {
    throw "Updater installMode must be passive so machine-wide updates can request elevation."
}
if ([string]::IsNullOrWhiteSpace([string]$tauri.plugins.updater.pubkey) -or
    $tauri.plugins.updater.pubkey -eq "REPLACE_WITH_GENERATED_TAURI_PUBLIC_KEY") {
    throw "A real Tauri updater public key must be configured before releasing."
}
if ([string]::IsNullOrWhiteSpace($env:TAURI_SIGNING_PRIVATE_KEY)) {
    throw "TAURI_SIGNING_PRIVATE_KEY is missing. Signed updater artifacts are mandatory."
}

$null = Invoke-Git @("fetch", "--force", "--tags", "origin")
$remoteTag = Invoke-Git @("ls-remote", "--tags", "origin", "refs/tags/$ReleaseTag")
$remoteTagExists = -not [string]::IsNullOrWhiteSpace($remoteTag)

if ($EventName -eq "workflow_dispatch") {
    if ((Invoke-Git @("status", "--porcelain")) -ne "") {
        throw "The release checkout is not clean. Refusing to modify a working tree with unexpected changes."
    }

    $comparison = Compare-AppVersions -Left $releaseVersion -Right $currentVersion
    if ($comparison -lt 0) {
        throw "Requested version $releaseVersion is older than the current application version $currentVersion."
    }

    if ($comparison -gt 0) {
        if ($remoteTagExists) {
            throw "Tag $ReleaseTag already exists. Refusing to move or overwrite an existing release tag."
        }

        $package.version = $releaseVersion
        $npmLock.version = $releaseVersion
        $npmLock.packages[""].version = $releaseVersion
        $tauri.version = $releaseVersion

        Write-JsonFile -Path "package.json" -Value $package
        Write-JsonFile -Path "package-lock.json" -Value $npmLock
        Write-JsonFile -Path "src-tauri/tauri.conf.json" -Value $tauri

        $cargoPattern = '(?ms)(^\[package\].*?^version\s*=\s*")[^"]+(")'
        $cargoMatches = [regex]::Matches($cargoToml, $cargoPattern)
        if ($cargoMatches.Count -ne 1) {
            throw "Expected exactly one Cargo.toml [package] version; found $($cargoMatches.Count)."
        }
        $cargoEvaluator = [System.Text.RegularExpressions.MatchEvaluator]{
            param($match)
            return $match.Groups[1].Value + $releaseVersion + $match.Groups[2].Value
        }
        $cargoToml = [regex]::Replace($cargoToml, $cargoPattern, $cargoEvaluator, 1)

        $cargoLockEvaluator = [System.Text.RegularExpressions.MatchEvaluator]{
            param($match)
            return $match.Groups[1].Value + $releaseVersion + $match.Groups[2].Value
        }
        $cargoLock = [regex]::Replace($cargoLock, $cargoLockPattern, $cargoLockEvaluator, 1)
        [System.IO.File]::WriteAllText((Join-Path $repoRoot "src-tauri/Cargo.toml"), $cargoToml, $utf8NoBom)
        [System.IO.File]::WriteAllText((Join-Path $repoRoot "src-tauri/Cargo.lock"), $cargoLock, $utf8NoBom)

        $null = Invoke-Git @("add", "--", "package.json", "package-lock.json", "src-tauri/tauri.conf.json", "src-tauri/Cargo.toml", "src-tauri/Cargo.lock")
        $null = Invoke-Git @("diff", "--cached", "--check")
        $null = Invoke-Git @("config", "user.name", "github-actions[bot]")
        $null = Invoke-Git @("config", "user.email", "41898282+github-actions[bot]@users.noreply.github.com")
        $null = Invoke-Git @("commit", "-m", "chore(release): prepare $ReleaseTag")
        $null = Invoke-Git @("push", "origin", "HEAD:refs/heads/main")
        $releaseCommit = Invoke-Git @("rev-parse", "HEAD")

        Write-Host "::notice::Updated all Spindle version declarations to $releaseVersion and committed $releaseCommit to main."
    }
    elseif ($remoteTagExists) {
        $releaseCommit = Invoke-Git @("rev-parse", "refs/tags/$ReleaseTag^{commit}")
        $null = Invoke-Git @("checkout", "--detach", $releaseCommit)
        Write-Host "::notice::Reusing the existing $ReleaseTag commit $releaseCommit for a release retry."
    }
    else {
        $releaseCommit = Invoke-Git @("rev-parse", "HEAD")
        Write-Host "::notice::Version $releaseVersion is already committed; continuing without another version commit."
    }
}
else {
    if ($releaseVersion -ne $currentVersion) {
        throw "Tag $ReleaseTag contains application version $currentVersion, not $releaseVersion."
    }
    $releaseCommit = Invoke-Git @("rev-parse", "HEAD")
}

# Re-read the final worktree after any checkout of an existing tag. Every manifest and
# lockfile must describe the same version before a release build is allowed to start.
$finalPackage = Get-Content "package.json" -Raw | ConvertFrom-Json
$finalNpmLock = Get-Content "package-lock.json" -Raw | ConvertFrom-Json
$finalTauri = Get-Content "src-tauri/tauri.conf.json" -Raw | ConvertFrom-Json
$finalCargoToml = Get-Content "src-tauri/Cargo.toml" -Raw
$finalCargoLock = Get-Content "src-tauri/Cargo.lock" -Raw
$finalCargoVersion = [regex]::Match($finalCargoToml, '(?ms)^\[package\].*?^version\s*=\s*"([^"]+)"')
$finalCargoLockMatch = [regex]::Match($finalCargoLock, $cargoLockPattern)
$finalVersions = @(
    [string]$finalPackage.version,
    [string]$finalNpmLock.version,
    [string]$finalNpmLock.packages[""].version,
    [string]$finalTauri.version,
    [string]$finalCargoVersion.Groups[1].Value,
    [string]([regex]::Match($finalCargoLockMatch.Value, 'version = "([^"]+)"').Groups[1].Value)
)
if (-not $finalCargoVersion.Success -or -not $finalCargoLockMatch.Success -or
    ($finalVersions | Select-Object -Unique).Count -ne 1 -or $finalVersions[0] -ne $releaseVersion) {
    throw "The selected release commit does not have consistent version $releaseVersion across all manifests and lockfiles."
}

"release_commit=$releaseCommit" | Out-File -FilePath $env:GITHUB_OUTPUT -Encoding utf8 -Append
"release_version=$releaseVersion" | Out-File -FilePath $env:GITHUB_OUTPUT -Encoding utf8 -Append
"Prepared Spindle $ReleaseTag from commit $releaseCommit." | Out-File -FilePath $env:GITHUB_STEP_SUMMARY -Encoding utf8 -Append
