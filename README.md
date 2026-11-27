<p align="center">
  <img src="docs/banner.jpg" alt="VIctory-Hydra — Unified D3D12 Runtime &amp; Shader Recompiler" width="100%">
</p>

<h1 align="center">VIctory-Hydra</h1>

<p align="center">
  <b>Unified multi-fork Direct3D 12 runtime, shader recompiler &amp; performance branch of VIctory</b><br>
  Combining verified performance, stability, shader-correctness, and tooling upgrades from across the community into one benchmarked, regression-tested Windows runtime.
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Platform-Windows%2010%2F11%20x64-0078D6?style=flat-square" alt="Platform">
  <img src="https://img.shields.io/badge/Graphics-Direct3D%2012%20(SM%206.6%20%2B%20Mesh%20Shaders)-76B900?style=flat-square" alt="D3D12">
  <img src="https://img.shields.io/badge/Language-Rust%20(MSVC)-orange?style=flat-square" alt="Rust">
  <img src="https://img.shields.io/badge/License-GPL--3.0--or--later-blue?style=flat-square" alt="License">
</p>

<p align="center">
  <b>Language:</b> <b>English</b> · <a href="README.ru.md">Русский (README.ru.md)</a>
</p>

---

## What is VIctory-Hydra?

**VIctory-Hydra** is a community-driven "Frankenstein" unification branch built on top of `cool_fox` / `VIctory`. Instead of leaving breakthroughs scattered across a dozen incompatible forks, every major optimization and shader fix is cherry-picked, adapted cleanly, and **A/B benchmarked on real NVIDIA RTX & mid-range CPU hardware** — keeping what genuinely improves frame time, stability, or visual fidelity, and disabling or fixing what regresses.

> **Legal & Asset Policy Notice:** This repository contains **zero** copyrighted game code, binaries, textures, shaders, audio, or screenshots. Every runtime file is supplied locally by the user from their own legal dump, enforced automatically by `cargo xtask policy` and `.githooks/pre-commit`. See [docs/asset-policy.md](docs/asset-policy.md).

---

## Current Progress, FPS & Test PC Specs

### Reference Hardware (Primary Test Rig)
| Component | Specification |
| :--- | :--- |
| **OS** | Windows 11 x64 (Build `22621`) |
| **CPU** | AMD Ryzen 5 5600X (6 Cores / 12 Threads, Zen 3) |
| **GPU** | NVIDIA GeForce RTX 3060 12 GB (`ID3D12Device13`, D3D12 Agility SDK `1.620.2`, ~11,345 MiB VRAM budget) |
| **RAM** | 32 GB DDR4 |
| **Storage** | HDD for game dump · SSD for `%LOCALAPPDATA%` shader & pipeline cache |

### Current Performance & Status (`1280×720`, `render_scale = 0.667`, `--console series-s`, Release)
- **Menus / Loading:** `60.0 FPS` (`16.7 ms/frame`).
- **In-World Automated Benchmark (205 world frames / 20.0s, `avg_draws = 6,284`):**
  - **Mean:** `11.24 FPS` (`89.0 ms/frame`) · **Median:** `11.71 FPS` (`85.4 ms/frame`) · `p90 = 104.5 ms`.
- **In-World Live Gameplay:** **`13–14 FPS`** (`69.3–79.5 ms/frame` at `4,444–4,786 draws/frame`) with full character models, clothing, shadows, ambient reflections, and HUD rendered accurately.
- **Bottleneck Breakdown:** The RTX 3060 finishes GPU passes in **`8.9–17.5 ms/frame`** on typical world scenes (`~34.9 ms` during streaming bursts). The primary bottleneck today is **CPU-side frame preparation & draw submission (`31–69 ms/frame`)** on 6 cores — **Stage 8** focuses specifically on cutting CPU frame-prep overhead to reach a playable **`30+ FPS`** baseline before scaling up resolution and quality.

### Stage Roadmap
| Stage | Focus | Status |
| :--- | :--- | :--- |
| **Stages 1–5** | Core `cool_fox` pareto residency, 256-shard guard locks, NaN scrub, eager decoration/face bakes, mesh launch setup | Done |
| **Stages 6–6.1** | `MSInfo.PayloadSizeInBytes` fix (unlocking 1,762 mesh pipelines), removal of ~3,964 global UAV barriers/frame, card-window VRAM budgets, HDD elevator I/O sort | Done |
| **Stages 7–7B** | Automated bench/A-B/replay harness, live A/B regression fixes (`VICTORY_UM_WATCH=0`, `constant:0.8`), Neptune `victory-shader` fixes (NVIDIA LDS atomics, CFG sections, `one_mode_per_register`, BVH trace limit) | Done (warming shaders & testing) |
| **Stage 8** | Next CPU performance wave toward **30+ FPS**: `VICTORY_GPU_DETILE`, `VICTORY_SEGMENTS_ACROSS`, `VICTORY_SPREAD_LEAN`, `VICTORY_ASYNC_PRESENT`, `VICTORY_HEAP_WORKER` | Next |

---

## Highlights & Unified Features

### 1. Renderer, Submission & Memory Pipeline
- **Multithreaded Thin-List Recording & Async Submission (`VICTORY_THIN=1`, `VICTORY_RELAY=2`, `VICTORY_ASYNC_SUBMIT=1`):** Moves D3D12 command-list translation, descriptor recording, AVX2 dirty compares, and submit joins off the title's threads onto dedicated worker pools.
- **Pareto Memory Residency & 32 GB RAM / 8–12 GB VRAM Stability:** Adaptive GPU memory residency budgets (`VICTORY_CARD_BUFFERS_MB`, `VICTORY_CARD_WINDOWS_MB`, `VICTORY_SETTLE_ROOM=1`), preventing WDDM paging exhaustion (`0x8007000E`) and periodic device-loss crashes.
- **Renderer Ports & Fast Paths:**
  - `VICTORY_PORT_CONSTANTS`, `VICTORY_PORT_DESCRIPTORS`, `VICTORY_PORT_GUARD`, `VICTORY_PORT_JOURNAL`
  - `VICTORY_UP_QUEUE=1`, `VICTORY_JOIN_BARRIERS=1`, `VICTORY_READBACK_POOL=1`, `VICTORY_TEXBACK_ASYNC=1`
  - `VICTORY_UM_COALESCE_KB=64` (user-memory page coalescing), `VICTORY_UM_TEXLOOK=1` (fast texture lookup epoch), `VICTORY_REMAP_SAME=1` & `VICTORY_ALIAS_RUNS=1` (cheap virtual memory alias remaps)
  - Lock-free console instruction counter fast path (`SEEN_ALL`) and deferred pipeline library persistence (`VICTORY_LIBRARY_STORE_LATER=1`)
- **DirectStorage Fast I/O (`VICTORY_DSTORAGE_IO=handles`, `VICTORY_DSTORAGE_SORT=1`):** Parallel handle-based reads and offset-sorted queues to cut loading times even on HDDs.

### 2. Shader Recompiler (`victory-shader`) & NVIDIA / AMD Fixes
- **NVIDIA LDS Atomic Sub Workaround:** Detects negated values (`negation_of`) and lowers affected LDS atomic loops cleanly to avoid NVIDIA DXIL driver edge cases.
- **DXIL CFG & Loop Normalization:** Expanded loop trace budget (`TRACE_LOOP_LIMIT=1024`), single-latch CFG normalization, `one_mode_per_register`, `whole_sections` / `permutes_outside_sections`, and `unused_descriptor_kind` structured handles.
- **Instruction & Intrinsic Coverage:** `Input::GroupInfo`, `s_pack_{ll,lh,hh}_b32_b16`, `wave_local` `v_mad_u32_u24` / `u16`, `s_bfm_b32` / `s_mov_b64` literal resource recovery, voxel visibility wave64 mask provenance, and shader LOD slot fix.
- **Offline Resume-Safe Pipeline Pre-Compiler:** `Warm-Shaders.cmd` (`victory warm --used`) with live progress percentage, compile rate, and ETA to eliminate in-game shader stutter after updates.

### 3. Visual Fidelity & Upscaler Integration
- **Balanced Ambient & Reflection Pass (`VICTORY_AMBIENT=constant:0.8`, `VICTORY_AMBIENT_REFLECT=0.5`):** Even interior/exterior lighting without doorway blowouts plus `CS_UpdateHistory` reflection shading (`Role::Reflect`), and `VICTORY_NAN_SCRUB=1` against NaN bloom artifacts.
- **Texture Longevity & Mode Control:** `VICTORY_STALE_KEEP_MS=10000` / `VICTORY_STALE_KEEP_MAX_MS=120000` + `VICTORY_TEXTURE_BACK_MODES=0` to reduce texture pop-in and readback stalls.
- **Custom Resolution, Render Scale & Pacing:** `VICTORY_OUTPUT`, `VICTORY_RENDER_SCALE`, `VICTORY_FRAME_INTERVAL_MS` (60/120 FPS pacing while keeping game simulation clock at 1.00x).
- **External PC Upscaler Hook (`VICTORY_FSR_DLL`):** Optional redirect for PC FidelityFX / OptiScaler DLLs without modifying game files.

### 4. Automated Benchmarking & Live A/B Testing
- **Built-in Benchmark & A/B Harness (`tools/bench-gta6.ps1`):**
  - `live` mode: timed world-frame benchmark with automatic warmup and exit (`VICTORY_BENCH_WORLD_S`, `VICTORY_BENCH_WARMUP_S`).
  - `ab` mode: live in-world A/B toggling of runtime flags (`VICTORY_AB`, `VICTORY_AB_WORLD=1`) to prove every flag's real frame-time delta.
  - `record` / `replay` mode: capture a deterministic world frame and replay GPU/shader passes in ~5 seconds without rebooting the title.

---

## Requirements

- **OS:** Windows 10 / 11 x64
- **CPU:** x86-64 with AVX2 (6+ cores recommended)
- **GPU:** Direct3D 12 GPU with Shader Model 6.6 & Mesh Shaders (NVIDIA RTX 20/30/40/50 or AMD RX 6000/7000/9000; 8 GB+ VRAM recommended, 4 GB profile included)
- **RAM:** 32 GB recommended
- **Build Tools:** Rust toolchain (pinned in `rust-toolchain.toml`) + Visual Studio / MSVC Build Tools (x64)
- **Game Files:** Your own legal Xbox Series dump (`game_scarlett_master.exe` and data files stored **outside** this repository)

---

## Quick Start

### 1. Clone & Build

```powershell
git clone https://github.com/ToLIManl/VIctory-Hydra.git
cd VIctory-Hydra

# Release runtime (fastest, for playing)
cargo build --release --locked -p victory

# Optional: Diagnostics build (for frame replay, A/B profiling, captures)
cargo build --locked --profile diag -p victory --features diagnostics
```

### 2. Point to Your Local Game Dump

Set `VICTORY_GAME_DIR` to the folder containing your `game_scarlett_master.exe` (or place this repo next to a sibling `..\Game\` folder):

```powershell
$env:VICTORY_GAME_DIR = "D:\Games\MyDump"
```

Alternatively, copy `victory.example.toml` to `victory.toml` (git-ignored) and edit `game_dir`.

### 3. Launch

```powershell
# Recommended unified preset (override any VICTORY_* variable before launching)
.\Play-VIctory.cmd

# 4 GB / low-VRAM preset
.\Play-VIctory-4GB.cmd
```

To run in 1080p instead of the default 720p performance baseline:

```powershell
$env:VICTORY_OUTPUT = "1920x1080"
$env:VICTORY_RENDER_SCALE = "0.75"
.\Play-VIctory.cmd
```

### 4. Pre-Compile Shaders & Pipelines (Recommended After Updates)

```powershell
.\Warm-Shaders.cmd
```

Progress is saved incrementally (`warm-done.txt`), so you can stop (`Ctrl+C`) and resume anytime.

---

## Automated Benchmarking & Frame Replay

Use [tools/bench-gta6.ps1](tools/bench-gta6.ps1) to measure changes objectively:

```powershell
# Timed release benchmark (20s of world frames after 6s warmup)
.\tools\bench-gta6.ps1 -Mode live -WorldS 20 -Tag baseline

# Live in-world A/B test of runtime flags
.\tools\bench-gta6.ps1 -Mode ab -PhaseS 10 -AbSwitches "VICTORY_UM_TEXLOOK=0,VICTORY_UM_TEXLOOK=1" -Tag ab-texlook

# Capture a single world frame and replay it offline in seconds
.\tools\bench-gta6.ps1 -Mode record -Tag capture1
.\tools\bench-gta6.ps1 -Mode replay
```

---

## Repository Hygiene & Asset Policy

Install the pre-commit hook before contributing:

```powershell
cargo xtask install-hooks
cargo xtask policy
```

For maintainers publishing periodic updates, [tools/push-update.ps1](tools/push-update.ps1) runs `cargo xtask policy` and scans all tracked files for accidental local-path leaks before committing.

---

## Credits & Upstream Projects

This project stands on the shoulders of the entire VIctory / PortWatch community:

- **[VIctory](https://github.com/Marvisak/VIctory)** (Marvisak & contributors) — original loader, HLE services, D3D12 backend, and shader recompiler.
- **cool_fox / CGBJ** (codec-d & DoubleGhosty) — Pareto residency architecture, NVIDIA stability baseline, and core continuation.
- **Neptune_VI** — shader lowering upgrades (`one_mode_per_register`, `whole_sections`, LDS atomic workarounds, reflection ambient pass, coalescing & replay refinements).
- **[fxceless/victory-fork](https://github.com/fxceless/victory-fork)** — thin-list recording, async submission, relay/landing pipeline, frame recorder/replay, and diagnostic tooling.
- **AvatarManz (`VIctory-7900XTX`)** — multithreaded recording, framerate pacing unlock, and renderer ports.
- **Vice-Tay**, **Kelvaroq**, **[DexNom/VIctory](https://github.com/DexNom/VIctory)**, **[Belobug/VIctory-Belobug-AMD](https://github.com/Belobug/VIctory-Belobug-AMD)**, **[myokex/victory-amd](https://github.com/myokex/victory-amd)**, **[FQY55kai/ZJOU-VICTORY](https://github.com/FQY55kai/ZJOU-VICTORY)**, **Geckomacabre**, **cxy0072019-sketch**, and **romainhedouin** — shader compiler passes, ambient lighting, texture readback, write-guard optimizations, and D3D12 fixes.

---

## License

Licensed under **GPL-3.0-or-later**, matching upstream VIctory. See [LICENSE](LICENSE) and [docs/asset-policy.md](docs/asset-policy.md).
