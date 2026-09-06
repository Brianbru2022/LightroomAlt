param(
    [Parameter(Mandatory = $true)]
    [string]$FixtureRoot
)

$ErrorActionPreference = "Stop"
$projectRoot = Split-Path -Parent $PSScriptRoot
$exiftool = Join-Path $projectRoot "src-tauri\resources\exiftool\exiftool-13.59_64\exiftool.exe"
$dcraw = Join-Path $projectRoot "src-tauri\resources\libraw\libraw-0.22.2-win64\dcraw_emu.exe"
if (-not (Test-Path -LiteralPath $FixtureRoot)) { throw "Fixture directory does not exist: $FixtureRoot" }
if (-not (Test-Path -LiteralPath $exiftool) -or -not (Test-Path -LiteralPath $dcraw)) { throw "Bundled RAW tooling is unavailable; build the desktop dependencies first." }

$fixtures = Get-ChildItem -LiteralPath $FixtureRoot -Recurse -File |
    Where-Object { $_.Extension.ToLowerInvariant() -in ".cr2", ".cr3", ".nef", ".arw" }
if (-not $fixtures) { throw "No CR2, CR3, NEF or ARW fixtures were found." }

foreach ($fixture in $fixtures) {
    $metadataJson = & $exiftool -json -n -ImageWidth -ImageHeight -Model $fixture.FullName
    if ($LASTEXITCODE -ne 0) { throw "ExifTool could not read $($fixture.FullName)" }
    $metadata = $metadataJson | ConvertFrom-Json | Select-Object -First 1
    if (-not $metadata.ImageWidth -or -not $metadata.ImageHeight) { throw "No image dimensions were extracted from $($fixture.FullName)" }
    & $dcraw -i -v $fixture.FullName | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "LibRaw could not inspect $($fixture.FullName)" }
    Write-Output "RAW fixture passed: $($fixture.Name) ($($metadata.Model), $($metadata.ImageWidth)x$($metadata.ImageHeight))"
}
