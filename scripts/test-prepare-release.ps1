$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$tempRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("spindle-release-test-" + [guid]::NewGuid().ToString("N"))
$fixture = Join-Path $tempRoot "working"
$bareRemote = Join-Path $tempRoot "origin.git"
$outputFile = Join-Path $tempRoot "github-output.txt"
$summaryFile = Join-Path $tempRoot "github-summary.txt"

$originalLocation = Get-Location
$originalSigningKey = $env:TAURI_SIGNING_PRIVATE_KEY
$originalOutput = $env:GITHUB_OUTPUT
$originalSummary = $env:GITHUB_STEP_SUMMARY

function Invoke-TestGit {
    param([Parameter(Mandatory = $true)][string[]]$GitArgs)

    $output = & git -C $fixture @GitArgs 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "Test git $($GitArgs -join ' ') failed: $($output -join [Environment]::NewLine)"
    }
    return [string]::Join([Environment]::NewLine, @($output)).Trim()
}

try {
    New-Item -ItemType Directory -Path $fixture -Force | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $fixture "scripts") -Force | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $fixture "src-tauri") -Force | Out-Null

    $filesToCopy = @(
        "package.json",
        "package-lock.json",
        "src-tauri/tauri.conf.json",
        "src-tauri/Cargo.toml",
        "src-tauri/Cargo.lock"
    )
    foreach ($relativePath in $filesToCopy) {
        $destination = Join-Path $fixture $relativePath
        Copy-Item -LiteralPath (Join-Path $repoRoot $relativePath) -Destination $destination -Force
    }
    Copy-Item -LiteralPath (Join-Path $repoRoot "scripts/prepare-release.ps1") -Destination (Join-Path $fixture "scripts/prepare-release.ps1") -Force

    & git init --bare $bareRemote | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "Could not initialize the temporary bare Git remote." }
    & git -C $fixture init -b main | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "Could not initialize the temporary main branch." }
    & git -C $fixture config user.name "Release workflow test" | Out-Null
    & git -C $fixture config user.email "release-test@example.invalid" | Out-Null
    & git -C $fixture remote add origin $bareRemote
    if ($LASTEXITCODE -ne 0) { throw "Could not configure the temporary Git remote." }
    & git -C $fixture add -- $filesToCopy "scripts/prepare-release.ps1"
    if ($LASTEXITCODE -ne 0) { throw "Could not stage the test fixture." }
    & git -C $fixture commit -m "test fixture baseline" | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "Could not commit the test fixture." }
    & git -C $fixture push -u origin main | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "Could not seed the temporary Git remote." }

    $env:TAURI_SIGNING_PRIVATE_KEY = "test-only-not-a-real-key"
    $env:GITHUB_OUTPUT = $outputFile
    $env:GITHUB_STEP_SUMMARY = $summaryFile
    $prepareScript = Join-Path $fixture "scripts/prepare-release.ps1"

    & $prepareScript -ReleaseTag "v9.8.7" -EventName "workflow_dispatch" -GitRef "refs/heads/main"

    $package = Get-Content (Join-Path $fixture "package.json") -Raw | ConvertFrom-Json
    $npmLock = Get-Content (Join-Path $fixture "package-lock.json") -Raw | ConvertFrom-Json -AsHashtable
    $tauri = Get-Content (Join-Path $fixture "src-tauri/tauri.conf.json") -Raw | ConvertFrom-Json
    $cargoToml = Get-Content (Join-Path $fixture "src-tauri/Cargo.toml") -Raw
    $cargoLock = Get-Content (Join-Path $fixture "src-tauri/Cargo.lock") -Raw
    $cargoTomlVersion = [regex]::Match($cargoToml, '(?ms)^\[package\].*?^version\s*=\s*"([^"]+)"').Groups[1].Value
    $cargoLockVersion = [regex]::Match($cargoLock, '(?ms)^\[\[package\]\]\r?\nname = "spindle"\r?\nversion = "([^"]+)"').Groups[1].Value
    $versions = @(
        [string]$package.version,
        [string]$npmLock.version,
        [string]$npmLock.packages[""].version,
        [string]$tauri.version,
        [string]$cargoTomlVersion,
        [string]$cargoLockVersion
    )

    if (($versions | Select-Object -Unique).Count -ne 1 -or $versions[0] -ne "0.1.1") {
        throw "Expected every manifest and lockfile to contain 0.1.1; got $($versions -join ', ')."
    }

    $firstCommit = Invoke-TestGit -GitArgs @("rev-parse", "HEAD")
    $remoteLine = Invoke-TestGit -GitArgs @("ls-remote", "origin", "refs/heads/main")
    $remoteCommit = $remoteLine.Split([char]9)[0]
    if ($firstCommit -ne $remoteCommit) {
        throw "The release version commit was not pushed to the temporary main branch."
    }
    if (-not (Select-String -LiteralPath $outputFile -Pattern ("release_commit=" + $firstCommit) -Quiet)) {
        throw "The script did not publish its release commit output."
    }

    & $prepareScript -ReleaseTag "v0.1.1" -EventName "workflow_dispatch" -GitRef "refs/heads/main"
    $retryCommit = Invoke-TestGit -GitArgs @("rev-parse", "HEAD")
    if ($retryCommit -ne $firstCommit) {
        throw "Retrying the same version created an unexpected second version commit."
    }

    $olderVersionRejected = $false
    try {
        & $prepareScript -ReleaseTag "v0.1.0" -EventName "workflow_dispatch" -GitRef "refs/heads/main"
    }
    catch {
        $olderVersionRejected = $_.Exception.Message -like "*older than the current application version*"
    }
    if (-not $olderVersionRejected) {
        throw "The script did not reject an older release version."
    }

    Write-Host "Release preparation integration test passed: version updates, lockfile sync, push, retry, and older-version guard."
}
finally {
    Set-Location $originalLocation
    $env:TAURI_SIGNING_PRIVATE_KEY = $originalSigningKey
    $env:GITHUB_OUTPUT = $originalOutput
    $env:GITHUB_STEP_SUMMARY = $originalSummary
    if (Test-Path -LiteralPath $tempRoot) {
        Remove-Item -LiteralPath $tempRoot -Recurse -Force
    }
}
