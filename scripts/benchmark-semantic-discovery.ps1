$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
Push-Location $repo
try {
  Write-Host 'Keepframe Milestone 14 semantic discovery benchmark'
  cargo test --manifest-path src-tauri/Cargo.toml semantic_retrieval_twenty_thousand_sources --release -- --ignored --nocapture
  if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
} finally {
  Pop-Location
}
