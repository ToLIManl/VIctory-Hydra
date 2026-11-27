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

echo ============================================================================
echo VIctory-Hydra Offline Shader ^& Pipeline Pre-Compiler
echo Cache dir: %VICTORY_CACHE_DIR%
echo Workers  : %VICTORY_PIPELINE_WORKERS% processes (resume-safe)
echo ============================================================================

"%runner%" warm --used --workers %VICTORY_PIPELINE_WORKERS% %*
exit /b %errorlevel%
