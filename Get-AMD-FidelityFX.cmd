@echo off
rem Downloads the official signed AMD FidelityFX SDK 2.x DLLs needed for FSR 3 Frame Generation
rem (works on RTX 20/30/40/50 and AMD/Intel DX12 GPUs, about 30 MB) directly from AMD GPUOpen GitHub
rem into amd-sdk\FidelityFX\bin in this folder. Run it once.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\get-amd-fidelityfx.ps1" %*
echo.
pause
