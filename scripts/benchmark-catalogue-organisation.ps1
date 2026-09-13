$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
Push-Location $repo
try {
  Write-Host 'Keepframe Milestone 13 catalogue organisation benchmark'
  pnpm exec vitest run src/lib/selection.test.ts src/views/LibraryProductivityView.test.tsx --reporter=verbose
  if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
  cargo test --manifest-path src-tauri/Cargo.toml catalogue_organisation_performance_checkpoint -- --ignored --nocapture
  if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
} finally {
  Pop-Location
}
