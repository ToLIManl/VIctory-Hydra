# Fetches the official signed AMD FidelityFX SDK 2.x DLLs needed for AMD FSR 3 Frame Generation
# (works on RTX 20/30 cards that lack DLSS Frame Generation, as well as RTX 40/50 and AMD/Intel GPUs)
# directly from github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK into amd-sdk\FidelityFX\bin (git-ignored).
param(
    [string]$Tag = 'v2.4.0',
    [string]$To = ''
)
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
$sdk = if ($To) { $To } else { Join-Path (Resolve-Path (Join-Path $PSScriptRoot '..')).Path 'amd-sdk' }
$bin = Join-Path $sdk 'FidelityFX\bin'
$files = @(
    @{ Remote = "Kits/FidelityFX/signedbin/amd_fidelityfx_loader_dx12.dll"; Local = "amd_fidelityfx_loader_dx12.dll"; Optional = $false },
    @{ Remote = "Kits/FidelityFX/signedbin/amd_fidelityfx_framegeneration_dx12.dll"; Local = "amd_fidelityfx_framegeneration_dx12.dll"; Optional = $false },
    @{ Remote = "Kits/FidelityFX/LICENSE.txt"; Local = "LICENSE.txt"; Optional = $true }
)

function Get-File([string]$Url, [string]$ToPath) {
    New-Item -ItemType Directory -Force (Split-Path $ToPath) | Out-Null
    $part = "$ToPath.part"
    if (Get-Command curl.exe -ErrorAction SilentlyContinue) {
        & curl.exe --location --fail --silent --show-error --progress-bar --output $part $Url
        if ($LASTEXITCODE -ne 0) { Remove-Item $part -ErrorAction SilentlyContinue; throw "could not get $Url" }
    } else {
        Invoke-WebRequest -Uri $Url -OutFile $part -UseBasicParsing -ErrorAction Stop
    }
    Move-Item $part $ToPath -Force
}

Write-Host "AMD FidelityFX SDK $Tag signed DLLs for FSR 3 Frame Generation (about 30 MB):"
foreach ($f in $files) {
    $dest = Join-Path $bin $f.Local
    if (Test-Path $dest) {
        Write-Host "  already present: $($f.Local)"
        continue
    }
    $url = "https://raw.githubusercontent.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK/$Tag/$($f.Remote)"
    Write-Host "  downloading $($f.Local) ..."
    try {
        Get-File $url $dest
    } catch {
        if (-not $f.Optional) { throw }
    }
}
Write-Host "Done: $bin"
