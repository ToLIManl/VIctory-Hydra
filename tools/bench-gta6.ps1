param(
    [ValidateSet("live", "ab", "record", "replay")]
    [string]$Mode = "live",
    [switch]$Diag,
    [switch]$Screenshot,
    [int]$WorldS = 20,
    [int]$WarmupS = 6,
    [int]$PhaseS = 10,
    [string]$AbSwitches = "",
    [string]$Output = "1280x720",
    [string]$RenderScale = "0.667",
    [string]$Ambient = "constant:0.8",
    [string]$AmbientReflect = "0.5",
    [string]$ExtraEnv = "",
    [string]$Tag = "run",
    [string]$LauncherDir = "",
    [string]$GameExe = "",
    [string]$CaptureDir = "",
    [string[]]$ReplayArgs = @("--loops", "5", "--awake")
)

$ErrorActionPreference = "Stop"
$RepoDir = Split-Path $PSScriptRoot -Parent
$ParentDir = Split-Path $RepoDir -Parent

if ([string]::IsNullOrWhiteSpace($LauncherDir)) {
    if ((Test-Path (Join-Path $ParentDir "victory-rtx-unified.exe")) -or (Test-Path (Join-Path $ParentDir "victory-rtx-diag.exe"))) {
        $LauncherDir = $ParentDir
    } else {
        $LauncherDir = $RepoDir
    }
}

if ([string]::IsNullOrWhiteSpace($GameExe)) {
    $candidates = @()
    if (-not [string]::IsNullOrWhiteSpace($env:VICTORY_GAME_DIR)) {
        $candidates += Join-Path $env:VICTORY_GAME_DIR "game_scarlett_master.exe"
    }
    $candidates += Join-Path (Split-Path $LauncherDir -Parent) "Game\game_scarlett_master.exe"
    $candidates += Join-Path $ParentDir "Game\game_scarlett_master.exe"
    $candidates += Join-Path $ParentDir "GTA6.Xbox.v1.0.2.1\game_scarlett_master.exe"
    $GameExe = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
}

$BenchDir = Join-Path $LauncherDir "bench"
New-Item -ItemType Directory -Force -Path $BenchDir | Out-Null
if ([string]::IsNullOrWhiteSpace($CaptureDir)) {
    $CaptureDir = Join-Path $BenchDir "capture"
}

function Resolve-Runner([bool]$wantDiag) {
    $names = if ($wantDiag) {
        @("victory-rtx-diag.exe", "target\diag\victory.exe", "victory-diag.exe")
    } else {
        @("victory-rtx-unified.exe", "target\release\victory.exe", "victory.exe")
    }
    foreach ($base in @($LauncherDir, $RepoDir)) {
        foreach ($n in $names) {
            $p = Join-Path $base $n
            if (Test-Path $p) { return $p }
        }
    }
    return $null
}

if ($Mode -eq "replay") {
    $diagExe = Resolve-Runner $true
    if (-not $diagExe) { throw "Missing diagnostics runner (build with: cargo build --locked --profile diag -p victory --features diagnostics)" }
    Write-Host "=== REPLAY BENCHMARK ($CaptureDir) ==="
    Push-Location $LauncherDir
    try {
        & $diagExe replay $CaptureDir @ReplayArgs
        exit $LASTEXITCODE
    } finally {
        Pop-Location
    }
}

$useDiag = $Diag -or ($Mode -eq "ab") -or ($Mode -eq "record")
$runner = Resolve-Runner $useDiag
if (-not $runner) { throw "Missing runner binary (build release or diag profile first)" }
if ([string]::IsNullOrWhiteSpace($GameExe) -or -not (Test-Path $GameExe)) {
    throw "Missing game executable. Set VICTORY_GAME_DIR or pass -GameExe <path\to\game_scarlett_master.exe>"
}

# Clean leftover VICTORY_* env vars in this process
Get-ChildItem Env:VICTORY_* | Remove-Item -ErrorAction SilentlyContinue

# Baseline Stage 6.1 + Stage 7 runtime configuration (matches Play-GTA6-RTX.cmd)
$env:VICTORY_GAME_DIR = Split-Path $GameExe -Parent
$env:VICTORY_CACHE_DIR = "$env:LOCALAPPDATA\VIctory\cache-pareto"
$env:VICTORY_PORT_CONSTANTS = "1"
$env:VICTORY_PORT_DESCRIPTORS = "1"
$env:VICTORY_PORT_CBV = "0"
$env:VICTORY_PORT_GUARD = "1"
$env:VICTORY_PORT_JOURNAL = "1"
$env:VICTORY_UP_QUEUE = "1"
$env:VICTORY_JOIN_BARRIERS = "1"
$env:VICTORY_READBACK_POOL = "1"
$env:VICTORY_TEXBACK_ASYNC = "1"
$env:VICTORY_LIBRARY_STORE_LATER = "1"
$env:VICTORY_LIBRARY_SAVE_S = "120"

$env:VICTORY_AMBIENT = $Ambient
$env:VICTORY_AMBIENT_SHADE = "1.0,0.6"
$env:VICTORY_AMBIENT_AIR = "1.5"
$env:VICTORY_AMBIENT_REFLECT = $AmbientReflect
$env:VICTORY_KEEP_SHOWN = "2"
$env:VICTORY_STALE_KEEP_MS = "10000"
$env:VICTORY_STALE_KEEP_MAX_MS = "120000"
$env:VICTORY_TEXTURE_BACK_MODES = "0"
$env:VICTORY_TEXTURE_NO_WAIT = "1"

$env:VICTORY_RELAY = "2"
$env:VICTORY_WIDE_BACK = "1"
$env:VICTORY_CARD_REHASH_RUNS = "1"
$env:VICTORY_JOIN_COPIES = "1"
$env:VICTORY_INDIRECT_WINDOW = "1"
$env:VICTORY_RELAY_COPY = "1"
$env:VICTORY_CARD_STEPS = "2"
$env:VICTORY_PLACED_ORDER = "1"
$env:VICTORY_FILL_TITLE = "2"
$env:VICTORY_INDIRECT_BATCH = "2"
$env:VICTORY_CARD_AWAKE = "1"
$env:VICTORY_SHOW_AFTER = "1"
$env:VICTORY_THIN = "1"
$env:VICTORY_THIN_THREADS = "6"
$env:VICTORY_LAND_THREADS = "6"
$env:VICTORY_VIEW_THREADS = "4"
$env:VICTORY_MAP_THREADS = "1"
$env:VICTORY_WIDE_MERGE = "1"
$env:VICTORY_UPLOAD_AHEAD = "1"
$env:VICTORY_LEVELS_ON_DEMAND = "1"
$env:VICTORY_ASYNC_SUBMIT = "1"
$env:VICTORY_SUBMIT_FRAMES = "3"
$env:VICTORY_JOIN_PARTS = "24"
$env:VICTORY_JOIN_LISTS = "128"
$env:VICTORY_CARD_BUFFERS_MIN_KB = "128"
$env:VICTORY_CARD_BUFFERS_MB = "1536"
$env:VICTORY_CARD_WINDOWS_MB = "1024"
$env:VICTORY_CARD_RESERVE_BUDGETS = "1"
$env:VICTORY_SETTLE_ROOM = "1"
$env:VICTORY_WAIT_RESIDENT = "0"

$env:VICTORY_MASKED_PATCH = "1"
$env:VICTORY_MESH_LAUNCH = "1"
$env:VICTORY_PORT_CONSTANTS_ORDER = "0"
$env:VICTORY_SCRATCH_ANY_SIZE = "1"
$env:VICTORY_SCRATCH_SPARE_MB = "256"
$env:VICTORY_SHADER_FAST_BITS = "24"
$env:VICTORY_RELAY_COPY_RUNS = "1"
$env:VICTORY_SCOPED_CARD_BARRIERS = "1"
$env:VICTORY_NAN_SCRUB = "1"
$env:VICTORY_CARD_BIG_ROOM = "256"
$env:VICTORY_UM_TEXLOOK = "1"
$env:VICTORY_UM_COALESCE_KB = "64"
$env:VICTORY_UM_WATCH = "0"
$env:VICTORY_TABLES_BY_CONTENT = "0"
$env:VICTORY_REMAP_SAME = "1"
$env:VICTORY_ALIAS_RUNS = "1"
$env:VICTORY_WINDOWS_BAR_MB = "0"

$env:VICTORY_DSTORAGE_IO = "handles"
$env:VICTORY_DSTORAGE_SORT = "1"
$env:VICTORY_DSTORAGE_WORKERS = "3"
$env:VICTORY_DSTORAGE_DECODE_WORKERS = "8"
$env:VICTORY_PIPELINE_LOWEST = "0"
$env:VICTORY_PIPELINE_WORKERS = "6"

$env:VICTORY_OUTPUT = $Output
$env:VICTORY_UPSCALER = "6"
$env:VICTORY_RENDER_SCALE = $RenderScale
$env:VICTORY_FRAME_INTERVAL_MS = "16.667"
Remove-Item Env:VICTORY_GPU_TIMING -ErrorAction SilentlyContinue
$env:VICTORY_AUTO_A = "90"
$env:VICTORY_BENCH_WARMUP_S = "$WarmupS"
$env:VICTORY_BENCH_WORLD_S = "$WorldS"
$env:VICTORY_BENCH_MIN_DRAWS = "500"
$env:VICTORY_BENCH_ABORT_S = "420"

if ($useDiag) {
    $env:VICTORY_LOG = "info"
    # IMPORTANT: VICTORY_CAPTURE forces late_stage=0 (disables RELAY=2 & ASYNC_SUBMIT=1)!
    # Only enable PNG capture when -Screenshot is explicitly requested.
    if ($Screenshot) {
        $frameDir = Join-Path $BenchDir "frames-$Tag"
        New-Item -ItemType Directory -Force -Path $frameDir | Out-Null
        $env:VICTORY_CAPTURE = $frameDir
        $env:VICTORY_CAPTURE_WORLD = "1"
        $env:VICTORY_CAPTURE_WORLD_ONLY = "1"
    }
} else {
    $env:VICTORY_LOG = "info"
}

if ($Mode -eq "ab") {
    if ([string]::IsNullOrWhiteSpace($AbSwitches)) { throw "Specify -AbSwitches for -Mode ab" }
    $env:VICTORY_AB = "$PhaseS,$AbSwitches"
    $env:VICTORY_AB_WORLD = "1"
}

if ($Mode -eq "record") {
    New-Item -ItemType Directory -Force -Path $CaptureDir | Out-Null
    Remove-Item -Force -ErrorAction SilentlyContinue (Join-Path $CaptureDir "go")
    $env:VICTORY_RECORD = $CaptureDir
    $env:VICTORY_BENCH_RECORD_DIR = $CaptureDir
    $env:VICTORY_BENCH_WARMUP_S = "10"
    $env:VICTORY_BENCH_WORLD_S = "20"
}

# Apply caller overrides (semicolon-separated KEY=VAL)
if (-not [string]::IsNullOrWhiteSpace($ExtraEnv)) {
    foreach ($pair in $ExtraEnv.Split(';')) {
        $kv = $pair.Split('=', 2)
        if ($kv.Length -eq 2 -and -not [string]::IsNullOrWhiteSpace($kv[0])) {
            [System.Environment]::SetEnvironmentVariable($kv[0].Trim(), $kv[1].Trim(), "Process")
        }
    }
}

$logPath = Join-Path $BenchDir "bench-$Tag.log"
Write-Host "=== GTA 6 AUTO-BENCH [$Mode] tag=$Tag runner=$(Split-Path $runner -Leaf) out=$Output scale=$RenderScale ambient=$Ambient ==="
Write-Host "Log: $logPath"

Push-Location $LauncherDir
try {
    & $runner run $GameExe --console series-s --execute -- -skipToIIS *> $logPath
    $rc = $LASTEXITCODE
} finally {
    Pop-Location
}

Write-Host "Exit code: $rc"
Select-String -Path $logPath -Pattern "BENCH_WORLD_ENTERED|BENCH_RESULT|BENCH_TIMEOUT|BENCH_RECORD_TRIGGERED|A/B phase|frame recorder: the frame is captured|draws/frame|the card:|preparing:|waited for flights:" |
    Select-Object -Last 30 |
    ForEach-Object { Write-Host $_.Line }

$fpsMatches = Select-String -Path $logPath -Pattern '(\d+(?:\.\d+)?)\s*fps' -AllMatches |
    ForEach-Object { $_.Matches } |
    ForEach-Object { [double]$_.Groups[1].Value } |
    Where-Object { $_ -gt 0.5 -and $_ -lt 500 }
if ($fpsMatches) {
    $avgFps = ($fpsMatches | Measure-Object -Average).Average
    $maxFps = ($fpsMatches | Measure-Object -Maximum).Maximum
    Write-Host ("FPS summary: avg={0:F1} fps, max={1:F1} fps (samples={2})" -f $avgFps, $maxFps, $fpsMatches.Count)
}

if ($useDiag) {
    $frameDir = Join-Path $BenchDir "frames-$Tag"
    $pngs = Get-ChildItem -Path $frameDir -Filter "frame-*.png" -ErrorAction SilentlyContinue | Sort-Object LastWriteTime | Select-Object -Last 3
    if ($pngs) {
        Write-Host "Captured screenshots:"
        $pngs | ForEach-Object { Write-Host "  $($_.FullName)" }
    }
}
exit $rc
