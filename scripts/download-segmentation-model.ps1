param(
    [string]$ModelRoot = "D:\AI Models\Keepframe\segmentation\beit-base-ade20k-640",
    [switch]$ConfirmDownload
)
$ErrorActionPreference = "Stop"
if (-not $ModelRoot.StartsWith("D:\AI Models", [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "AI model assets must remain under D:\AI Models."
}
$Revision = "a8b6f5ef4acb2ea55d882989deaa02d39401e2b2"
$ExpectedHash = "e0747360d190bd7c0f53d2fe3b2ed560c304d3eefef94574af9c7e93aaf8e7a9"
$Base = "https://huggingface.co/microsoft/beit-base-finetuned-ade-640-640/resolve/$Revision"
Write-Host "Microsoft BEiT-base ADE20K requires approximately 900 MB."
Write-Host "Licence: Apache-2.0. Source revision: $Revision"
Write-Host "Destination: $ModelRoot"
if (-not $ConfirmDownload) {
    $Confirmation = Read-Host "Type DOWNLOAD to continue"
    if ($Confirmation -cne "DOWNLOAD") { Write-Host "Download cancelled; manual masks remain available."; exit 0 }
}
New-Item -ItemType Directory -Force -Path $ModelRoot | Out-Null
$ExpectedSizes = @{ "config.json" = 6966L; "preprocessor_config.json" = 276L; "pytorch_model.bin" = 899902905L }
foreach ($File in @("config.json", "preprocessor_config.json", "pytorch_model.bin")) {
    $Destination = Join-Path $ModelRoot $File
    $Partial = "$Destination.partial"
    try {
        Invoke-WebRequest -Uri "$Base/$File" -OutFile $Partial
        if ((Get-Item -LiteralPath $Partial).Length -ne $ExpectedSizes[$File]) { throw "The downloaded $File file was truncated or unexpected." }
        if ($File -eq "pytorch_model.bin") {
            $ActualHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $Partial).Hash.ToLowerInvariant()
            if ($ActualHash -ne $ExpectedHash) { throw "The downloaded model failed SHA-256 verification." }
        }
        Move-Item -Force -LiteralPath $Partial -Destination $Destination
    } finally {
        if (Test-Path -LiteralPath $Partial) { Remove-Item -Force -LiteralPath $Partial }
    }
}
[System.IO.File]::WriteAllText((Join-Path $ModelRoot "MODEL_SHA256.txt"), "$ExpectedHash`n", [System.Text.Encoding]::ASCII)
Write-Host "Intelligent masking model installed and verified."
