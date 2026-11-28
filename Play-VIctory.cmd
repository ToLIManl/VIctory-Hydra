@echo off
setlocal EnableExtensions DisableDelayedExpansion

:: VIctory-Hydra portable launcher (verified RTX / D3D12 performance + visual baseline).
:: Override any setting by defining VICTORY_* in your shell before running this script.

if not defined VICTORY_GAME_DIR (
  if exist "%~dp0..\Game\game_scarlett_master.exe" (
    set "VICTORY_GAME_DIR=%~dp0..\Game"
  ) else (
    set "VICTORY_GAME_DIR=%~dp0..\GTA6.Xbox.v1.0.2.1"
  )
)
if not exist "%VICTORY_GAME_DIR%\game_scarlett_master.exe" (
  echo Set VICTORY_GAME_DIR to your own dump folder containing game_scarlett_master.exe. 1>&2
  echo Example: set VICTORY_GAME_DIR=D:\Games\MyDump 1>&2
  exit /b 2
)

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

if not defined VICTORY_CACHE_DIR set "VICTORY_CACHE_DIR=%LOCALAPPDATA%\VIctory\cache-pareto"
if not defined VICTORY_CONSOLE set "VICTORY_CONSOLE=series-s"

:: Renderer Ports & Async Pipeline (PORT_CONSTANTS=0 keeps constants in GPU VRAM, avoiding PCIe/order stalls on RTX)
if not defined VICTORY_PORT_CONSTANTS set "VICTORY_PORT_CONSTANTS=0"
if not defined VICTORY_PORT_DESCRIPTORS set "VICTORY_PORT_DESCRIPTORS=1"
if not defined VICTORY_PORT_CBV set "VICTORY_PORT_CBV=0"
if not defined VICTORY_PORT_GUARD set "VICTORY_PORT_GUARD=1"
if not defined VICTORY_PORT_JOURNAL set "VICTORY_PORT_JOURNAL=1"
if not defined VICTORY_AUTO_A set "VICTORY_AUTO_A=90"
if not defined VICTORY_UP_QUEUE set "VICTORY_UP_QUEUE=1"
if not defined VICTORY_JOIN_BARRIERS set "VICTORY_JOIN_BARRIERS=1"
if not defined VICTORY_READBACK_POOL set "VICTORY_READBACK_POOL=1"
if not defined VICTORY_TEXBACK_ASYNC set "VICTORY_TEXBACK_ASYNC=1"
if not defined VICTORY_LIBRARY_STORE_LATER set "VICTORY_LIBRARY_STORE_LATER=1"
if not defined VICTORY_LIBRARY_SAVE_S set "VICTORY_LIBRARY_SAVE_S=120"

:: Lighting, Reflection & Texture Stability (Directional SH GI + Specular Reflections + Crisp Textures)
if not defined VICTORY_AMBIENT set "VICTORY_AMBIENT=0.8"
if not defined VICTORY_AMBIENT_SPECULAR set "VICTORY_AMBIENT_SPECULAR=1"
if not defined VICTORY_AMBIENT_SPECULAR_ALPHA set "VICTORY_AMBIENT_SPECULAR_ALPHA=0"
if not defined VICTORY_AMBIENT_ONCE set "VICTORY_AMBIENT_ONCE=0"
if not defined VICTORY_AMBIENT_SHADE set "VICTORY_AMBIENT_SHADE=1.0,0.6"
if not defined VICTORY_AMBIENT_AIR set "VICTORY_AMBIENT_AIR=1.5"
if not defined VICTORY_AMBIENT_REFLECT set "VICTORY_AMBIENT_REFLECT=0"
if not defined VICTORY_KEEP_SHOWN set "VICTORY_KEEP_SHOWN=1"
if not defined VICTORY_STALE_KEEP_MS set "VICTORY_STALE_KEEP_MS=10000"
if not defined VICTORY_STALE_KEEP_MAX_MS set "VICTORY_STALE_KEEP_MAX_MS=120000"
if not defined VICTORY_TEXTURE_BACK_MODES set "VICTORY_TEXTURE_BACK_MODES=0"
if not defined VICTORY_TEXTURE_NO_WAIT set "VICTORY_TEXTURE_NO_WAIT=1"
if not defined VICTORY_GPU_DETILE set "VICTORY_GPU_DETILE=1"
if not defined VICTORY_CARD_TOUCHED set "VICTORY_CARD_TOUCHED=1"
if not defined VICTORY_CARD_COMPARE_EVERY set "VICTORY_CARD_COMPARE_EVERY=0"
if not defined VICTORY_SHADER_OPT set "VICTORY_SHADER_OPT=191"
if exist "%~dp0mods" if not defined VICTORY_MODS_DIR set "VICTORY_MODS_DIR=%~dp0mods"

:: Multithreaded Submission & Memory Residency
if not defined VICTORY_RELAY set "VICTORY_RELAY=2"
if not defined VICTORY_WIDE_BACK set "VICTORY_WIDE_BACK=1"
if not defined VICTORY_CARD_REHASH_RUNS set "VICTORY_CARD_REHASH_RUNS=1"
if not defined VICTORY_JOIN_COPIES set "VICTORY_JOIN_COPIES=1"
if not defined VICTORY_INDIRECT_WINDOW set "VICTORY_INDIRECT_WINDOW=1"
if not defined VICTORY_RELAY_COPY set "VICTORY_RELAY_COPY=1"
if not defined VICTORY_RELAY_COPY_RUNS set "VICTORY_RELAY_COPY_RUNS=1"
if not defined VICTORY_CARD_STEPS set "VICTORY_CARD_STEPS=2"
if not defined VICTORY_PLACED_ORDER set "VICTORY_PLACED_ORDER=1"
if not defined VICTORY_FILL_TITLE set "VICTORY_FILL_TITLE=2"
if not defined VICTORY_INDIRECT_BATCH set "VICTORY_INDIRECT_BATCH=2"
if not defined VICTORY_CARD_AWAKE set "VICTORY_CARD_AWAKE=1"
if not defined VICTORY_SHOW_AFTER set "VICTORY_SHOW_AFTER=1"
if not defined VICTORY_THIN set "VICTORY_THIN=1"
if not defined VICTORY_THIN_THREADS set "VICTORY_THIN_THREADS=4"
if not defined VICTORY_LAND_THREADS set "VICTORY_LAND_THREADS=3"
if not defined VICTORY_MAP_THREADS set "VICTORY_MAP_THREADS=1"
if not defined VICTORY_WIDE_MERGE set "VICTORY_WIDE_MERGE=1"
if not defined VICTORY_UPLOAD_AHEAD set "VICTORY_UPLOAD_AHEAD=1"
if not defined VICTORY_LEVELS_ON_DEMAND set "VICTORY_LEVELS_ON_DEMAND=0"
if not defined VICTORY_ASYNC_SUBMIT set "VICTORY_ASYNC_SUBMIT=1"
if not defined VICTORY_SUBMIT_FRAMES set "VICTORY_SUBMIT_FRAMES=3"
if not defined VICTORY_JOIN_PARTS set "VICTORY_JOIN_PARTS=24"
if not defined VICTORY_JOIN_LISTS set "VICTORY_JOIN_LISTS=128"
if not defined VICTORY_CARD_BUFFERS_MIN_KB set "VICTORY_CARD_BUFFERS_MIN_KB=128"
if not defined VICTORY_CARD_BUFFERS_MB set "VICTORY_CARD_BUFFERS_MB=1536"
if not defined VICTORY_CARD_WINDOWS_MB set "VICTORY_CARD_WINDOWS_MB=1024"
if not defined VICTORY_CARD_RESERVE_BUDGETS set "VICTORY_CARD_RESERVE_BUDGETS=1"
if not defined VICTORY_SETTLE_ROOM set "VICTORY_SETTLE_ROOM=1"
if not defined VICTORY_WAIT_RESIDENT set "VICTORY_WAIT_RESIDENT=0"

:: Unified Multi-Fork Optimizations
if not defined VICTORY_MASKED_PATCH set "VICTORY_MASKED_PATCH=1"
if not defined VICTORY_PATCH_LEAN set "VICTORY_PATCH_LEAN=1"
if not defined VICTORY_PATCH_PAGE_RUNS set "VICTORY_PATCH_PAGE_RUNS=8"
if not defined VICTORY_HEAP_SYNC_EARLY set "VICTORY_HEAP_SYNC_EARLY=1"
if not defined VICTORY_VIEW_THREADS set "VICTORY_VIEW_THREADS=2"
if not defined VICTORY_UNDER_THREADS set "VICTORY_UNDER_THREADS=2"
if not defined VICTORY_MESH_LAUNCH set "VICTORY_MESH_LAUNCH=1"
if not defined VICTORY_PORT_CONSTANTS_ORDER set "VICTORY_PORT_CONSTANTS_ORDER=0"
if not defined VICTORY_PORT_CONSTANTS_ORDER_ALONE set "VICTORY_PORT_CONSTANTS_ORDER_ALONE=0"
if not defined VICTORY_SCRATCH_ANY_SIZE set "VICTORY_SCRATCH_ANY_SIZE=1"
if not defined VICTORY_SCRATCH_SPARE_MB set "VICTORY_SCRATCH_SPARE_MB=256"
if not defined VICTORY_SHADER_FAST_BITS set "VICTORY_SHADER_FAST_BITS=24"
if not defined VICTORY_SCOPED_CARD_BARRIERS set "VICTORY_SCOPED_CARD_BARRIERS=1"
if not defined VICTORY_NAN_SCRUB set "VICTORY_NAN_SCRUB=1"
if not defined VICTORY_CARD_BIG_ROOM set "VICTORY_CARD_BIG_ROOM=256"
if not defined VICTORY_UM_TEXLOOK set "VICTORY_UM_TEXLOOK=1"
if not defined VICTORY_UM_COALESCE_KB set "VICTORY_UM_COALESCE_KB=0"
if not defined VICTORY_UM_WATCH set "VICTORY_UM_WATCH=0"
if not defined VICTORY_TABLES_BY_CONTENT set "VICTORY_TABLES_BY_CONTENT=0"
if not defined VICTORY_REMAP_SAME set "VICTORY_REMAP_SAME=1"
if not defined VICTORY_ALIAS_RUNS set "VICTORY_ALIAS_RUNS=1"
if not defined VICTORY_ASK_EARLY set "VICTORY_ASK_EARLY=2"
if not defined VICTORY_LAND_SKIP set "VICTORY_LAND_SKIP=1"
if not defined VICTORY_PLACED_FORWARD set "VICTORY_PLACED_FORWARD=1"
if not defined VICTORY_DYNAMIC_SLAB set "VICTORY_DYNAMIC_SLAB=1"
if not defined VICTORY_ASYNC_RECYCLE set "VICTORY_ASYNC_RECYCLE=1"
if not defined VICTORY_HEAP_ROUND set "VICTORY_HEAP_ROUND=32"
if not defined VICTORY_WINDOWS_BAR_MB set "VICTORY_WINDOWS_BAR_MB=0"
if not defined VICTORY_EARLY_DEPTH set "VICTORY_EARLY_DEPTH=1"
if not defined VICTORY_WC_NO_BACK set "VICTORY_WC_NO_BACK=1"
if not defined VICTORY_WC_NO_BACK_MIN_KB set "VICTORY_WC_NO_BACK_MIN_KB=1024"
if not defined VICTORY_SPANS_QUICK set "VICTORY_SPANS_QUICK=1"
if not defined VICTORY_ROOT_BIND_CACHE set "VICTORY_ROOT_BIND_CACHE=1"
if not defined VICTORY_WAVE_ZERO_MASK set "VICTORY_WAVE_ZERO_MASK=1"
if not defined VICTORY_RESINFO_MIP_BOUNDS set "VICTORY_RESINFO_MIP_BOUNDS=1"
if not defined VICTORY_RESINFO_SHAPE set "VICTORY_RESINFO_SHAPE=1"
if not defined VICTORY_RESINFO_MSAA set "VICTORY_RESINFO_MSAA=1"
if not defined VICTORY_RESINFO_D16 set "VICTORY_RESINFO_D16=1"
if not defined VICTORY_COMPUTE_WINDOW_COHERENCE set "VICTORY_COMPUTE_WINDOW_COHERENCE=1"
if not defined VICTORY_WATER_LIGHT set "VICTORY_WATER_LIGHT=0"
if not defined VICTORY_LEAVE_OUT set "VICTORY_LEAVE_OUT=02384a60fda6f2d0,731a39171b568298"
if not defined VICTORY_SKIP_RT_TRACE set "VICTORY_SKIP_RT_TRACE=1"
if not defined VICTORY_POOL_GATE set "VICTORY_POOL_GATE=1"
if not defined VICTORY_PRELUDE_POOL set "VICTORY_PRELUDE_POOL=1"
if not defined VICTORY_DYNAMIC_WAIT set "VICTORY_DYNAMIC_WAIT=1"
if not defined VICTORY_THIN_BAKE set "VICTORY_THIN_BAKE=2"
if not defined VICTORY_DRIVER_WATCH set "VICTORY_DRIVER_WATCH=1"

:: DirectStorage & Pipeline Compilation
if not defined VICTORY_DSTORAGE_IO set "VICTORY_DSTORAGE_IO=handles"
if not defined VICTORY_DSTORAGE_SORT set "VICTORY_DSTORAGE_SORT=1"
if not defined VICTORY_DSTORAGE_WORKERS set "VICTORY_DSTORAGE_WORKERS=3"
if not defined VICTORY_DSTORAGE_DECODE_WORKERS set "VICTORY_DSTORAGE_DECODE_WORKERS=4"
if not defined VICTORY_DSTORAGE_CANCEL set "VICTORY_DSTORAGE_CANCEL=1"
if not defined VICTORY_DSTORAGE_ERROR_RECORD set "VICTORY_DSTORAGE_ERROR_RECORD=1"
if not defined VICTORY_DSTORAGE_PAGE_LIST set "VICTORY_DSTORAGE_PAGE_LIST=1"
if not defined VICTORY_DSTORAGE_FILE_SOURCE set "VICTORY_DSTORAGE_FILE_SOURCE=1"
if not defined VICTORY_PIPELINE_LOWEST set "VICTORY_PIPELINE_LOWEST=0"
if not defined VICTORY_PIPELINE_WORKERS set "VICTORY_PIPELINE_WORKERS=4"
if not defined VICTORY_LIBRARY_STATIC_SAMPLERS set "VICTORY_LIBRARY_STATIC_SAMPLERS=1"

:: NVIDIA DLSS Super Resolution & AMD FSR 3 Frame Generation (auto-enabled when SDK DLLs are fetched)
if not defined VICTORY_STREAMLINE_DIR (
  if exist "%~dp0nvidia-sdk\Streamline\bin\x64\sl.interposer.dll" set "VICTORY_STREAMLINE_DIR=%~dp0nvidia-sdk\Streamline\bin\x64"
)
if defined VICTORY_STREAMLINE_DIR if exist "%VICTORY_STREAMLINE_DIR%\sl.interposer.dll" (
  if not defined VICTORY_DLSS set "VICTORY_DLSS=1"
  if not defined VICTORY_DLSS_MODE set "VICTORY_DLSS_MODE=quality"
  if not defined VICTORY_DLSS_PRESET set "VICTORY_DLSS_PRESET=k"
)
if not defined VICTORY_FIDELITYFX_DIR (
  if exist "%~dp0amd-sdk\FidelityFX\bin\amd_fidelityfx_framegeneration_dx12.dll" set "VICTORY_FIDELITYFX_DIR=%~dp0amd-sdk\FidelityFX\bin"
)
if defined VICTORY_FIDELITYFX_DIR if exist "%VICTORY_FIDELITYFX_DIR%\amd_fidelityfx_framegeneration_dx12.dll" (
  if not defined VICTORY_FSR_FRAMEGEN set "VICTORY_FSR_FRAMEGEN=1"
  if not defined VICTORY_FRAMEGEN_X set "VICTORY_FRAMEGEN_X=2"
  if not defined VICTORY_TEXTURE_ROOM_PERCENT set "VICTORY_TEXTURE_ROOM_PERCENT=88"
)

:: F1 Settings Menu & Top-Left FPS/FG Counter
if not defined VICTORY_FPS_LINE set "VICTORY_FPS_LINE=1"
if not defined VICTORY_MENU_PLAY set "VICTORY_MENU_PLAY=%~f0"
if not defined VICTORY_MENU_FILE set "VICTORY_MENU_FILE=%~dp0victory-settings.txt"

:: Output & Pacing (1080p + DLSS 4 Preset K + FSR 3 FG 2x default; change live/restart via F1)
if not defined VICTORY_OUTPUT set "VICTORY_OUTPUT=1920x1080"
if not defined VICTORY_DLSS if not defined VICTORY_FSR_FRAMEGEN (
  if not defined VICTORY_UPSCALER set "VICTORY_UPSCALER=6"
  if not defined VICTORY_RENDER_SCALE set "VICTORY_RENDER_SCALE=0.667"
)
if not defined VICTORY_FRAME_INTERVAL_MS set "VICTORY_FRAME_INTERVAL_MS=0"
if not defined VICTORY_LOG if not defined RUST_LOG set "VICTORY_LOG=warn"

"%runner%" run "%VICTORY_GAME_DIR%\game_scarlett_master.exe" --console %VICTORY_CONSOLE% --execute %*
exit /b %errorlevel%
