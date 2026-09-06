$ErrorActionPreference = "Stop"

$projectRoot = Split-Path -Parent $PSScriptRoot
$bundleDirectory = Join-Path $projectRoot "src-tauri\target\release\bundle\nsis"
$releaseDirectory = Join-Path $projectRoot "release"

function Invoke-Checked {
    param([string]$Command, [string[]]$Arguments)
    & $Command @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$Command failed with exit code $LASTEXITCODE" }
}

Push-Location $projectRoot
try {
    $revision = (& git -C $projectRoot rev-parse HEAD).Trim()
    $dirty = [bool]((& git -C $projectRoot status --porcelain))
    Invoke-Checked "pnpm" @("install", "--frozen-lockfile")
    Invoke-Checked "pnpm" @("tauri", "build", "--bundles", "nsis")

    $installer = Get-ChildItem -LiteralPath $bundleDirectory -Filter "Keepframe_*_x64-setup.exe" -File |
        Sort-Object LastWriteTimeUtc -Descending |
        Select-Object -First 1
    if (-not $installer) { throw "Tauri completed without producing an NSIS installer." }

    New-Item -ItemType Directory -Force -Path $releaseDirectory | Out-Null
    $publishedInstaller = Join-Path $releaseDirectory $installer.Name
    Copy-Item -LiteralPath $installer.FullName -Destination $publishedInstaller -Force
    $stream = [System.IO.File]::OpenRead($publishedInstaller)
    try {
        $sha256 = [System.Security.Cryptography.SHA256]::Create()
        try { $hashValue = ([System.BitConverter]::ToString($sha256.ComputeHash($stream))).Replace("-", "") }
        finally { $sha256.Dispose() }
    }
    finally { $stream.Dispose() }
    Set-Content -LiteralPath (Join-Path $releaseDirectory "SHA256SUMS.txt") -NoNewline -Value "$hashValue  $($installer.Name)`n"

    $provenance = [ordered]@{
        product = "Keepframe"
        installer = $installer.Name
        sha256 = $hashValue
        builtAtUtc = (Get-Date).ToUniversalTime().ToString("o")
        sourceRevision = $revision
        sourceTreeDirty = $dirty
        command = "pnpm tauri build --bundles nsis"
    } | ConvertTo-Json
    Set-Content -LiteralPath (Join-Path $releaseDirectory "BUILD_PROVENANCE.json") -Value $provenance
    Write-Output "Installer: $publishedInstaller"
    Write-Output "SHA-256: $hashValue"
}
finally {
    Pop-Location
}
