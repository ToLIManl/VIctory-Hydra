@echo off
setlocal EnableExtensions DisableDelayedExpansion

:: Offline shader & D3D12 pipeline pre-compiler (resume-safe via warm-done.txt).
:: Run after updating your GPU driver or rebuilding the runtime to avoid in-game compile hitches.

if not defined VICTORY_CACHE_DIR set "VICTORY_CACHE_DIR=%LOCALAPPDATA%\VIctory\cache-pareto"
if not defined VICTORY_PIPELINE_WORKERS set "VICTORY_PIPELINE_WORKERS=6"
if not defined VICTORY_LOG set "VICTORY_LOG=warn"

set "runner=%~dp0target\release\victory.exe"
if not exist "%runner%" set "runner=%~dp0victory.exe"
if not exist "%runner%" set "runner=%~dp0victory-rtx-unified.exe"
if not exist "%runner%" (
  echo Build the runtime first: cargo build --release --locked -p victory 1>&2
  exit /b 2
)

for %%I in ("%runner%") do set "runner_dir=%%~dpI"
if not exist "%runner_dir%D3D12\D3D12Core.dll" (
  if exist "%~dp0D3D12\D3D12Core.dll" (
    if not exist "%runner_dir%D3D12" mkdir "%runner_dir%D3D12" >nul 2>&1
    copy /Y "%~dp0D3D12\D3D12Core.dll" "%runner_dir%D3D12\D3D12Core.dll" >nul
  )
)
if not exist "%runner_dir%D3D12\D3D12Core.dll" (
  echo ERROR: Missing DirectX 12 Agility SDK 1.620.2 ^(x64^): "%runner_dir%D3D12\D3D12Core.dll" 1>&2
  echo Without it, D3D12 fails with 0x887E0003. See README.md ^(Requirements ^& Dependencies^). 1>&2
  exit /b 2
)

echo ============================================================================
echo VIctory-Hydra Offline Shader ^& Pipeline Pre-Compiler
echo Cache dir: %VICTORY_CACHE_DIR%
echo Workers  : %VICTORY_PIPELINE_WORKERS% processes (resume-safe)
echo ============================================================================

"%runner%" warm --used --workers %VICTORY_PIPELINE_WORKERS% %*
exit /b %errorlevel%
