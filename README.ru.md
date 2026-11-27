<p align="center">
  <img src="docs/banner.jpg" alt="VIctory-Hydra — Объединённый D3D12-рантайм и рекомпилятор шейдеров" width="100%">
</p>

<h1 align="center">VIctory-Hydra</h1>

<p align="center">
  <b>Объединённый мультифорк Direct3D 12-рантайма, рекомпилятора шейдеров и оптимизаций на базе VIctory</b><br>
  Собираем лучшие наработки по производительности, стабильности, шейдерам и инструментам из разных форков в одном протестированном рантайме под Windows.
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Platform-Windows%2010%2F11%20x64-0078D6?style=flat-square" alt="Platform">
  <img src="https://img.shields.io/badge/Graphics-Direct3D%2012%20(SM%206.6%20%2B%20Mesh%20Shaders)-76B900?style=flat-square" alt="D3D12">
  <img src="https://img.shields.io/badge/Language-Rust%20(MSVC)-orange?style=flat-square" alt="Rust">
  <img src="https://img.shields.io/badge/License-GPL--3.0--or--later-blue?style=flat-square" alt="License">
</p>

<p align="center">
  <b>Язык:</b> <a href="README.md">English (README.md)</a> · <b>Русский</b>
</p>

---

## Что такое VIctory-Hydra?

**VIctory-Hydra** — это «франкенштейн»-ветка поверх `cool_fox` / `VIctory`. Вместо того чтобы держать удачные патчи разбросанными по десяти несовместимым форкам, сюда переносятся ключевые улучшения по производительности, шейдерам и стабильности — **с обязательной A/B-проверкой на реальном железе (NVIDIA RTX + 6-ядерный CPU)**: всё, что реально ускоряет кадр и чинит картинку, остаётся; всё, что даёт регрессию, отключается или переписывается.

> **Юридическая политика и ассеты:** В репозитории **нет и не будет** файлов игры, бинарей, текстур, шейдеров, звука или скриншотов. Пользователь подключает собственный легальный дамп локально, а чистота репо автоматически проверяется через `cargo xtask policy` и `.githooks/pre-commit`. См. [docs/asset-policy.md](docs/asset-policy.md).

---

## Новые возможности (Stages 8 – 10 Wave 1)

Ключевые улучшения после Stage 7B — строго по принципу **без деградации картинки** (никакого вырезания геометрии, света, воды или LOD ради искусственного FPS):

- **Форвардинг placed-копий и локальные слабы дескрипторов (`VICTORY_PLACED_FORWARD=1`, `VICTORY_DYNAMIC_SLAB=1`, `VICTORY_ASYNC_RECYCLE=1`):** убрали простои `BEHIND_PLACED` (`21.3–23.0 ms → 0.0–1.8 ms`), снизили блокировки `HostHeap::dynamic` с `26.9 ms → 0.5 ms` на 6 400+ таблиц/кадр и добавили защиту `HostHeap::id`, чтобы старые командные списки не освобождали слоты нового хипа по тому же адресу.
- **Приоритеты воркеров, потоки под 6 ядер и преаллокация буферов (`VICTORY_HELPER_PRIORITY`, `VICTORY_DSTORAGE_PRIORITY=-1`, `VICTORY_BUFFERS_AHEAD`, `VICTORY_PATCH_LEAN=1`, `VICTORY_INPLACE_CONSTANTS=1`):** подняли приоритет потоков `heap-sync`, `aside`, `land` и `wide` до `ABOVE_NORMAL` и понизили фоновую распаковку DirectStorage (ожидание `heaps' records` упало с `4.2–9.8 ms → 1.8–2.3 ms`), добавили преаллокацию буферов 64 KiB до входа в мир и вывели `PLACED_RAW` из Release-сборки `texture_at()`.
- **Единый порядок статических семплеров и фиксы шейдеров под 32-lane NVIDIA (`VICTORY_SAMPLER_ORDER`, `VICTORY_THREAD_TICKET`, `VICTORY_LDS_LOOP_ATOMICS`, `VICTORY_LDS_BOUNDS`, `VICTORY_LDS_FIRST_INIT`, `VICTORY_LOOPS_WAVE`, `VICTORY_EARLY_DEPTH`):** починили полосатость 3D LUT (`PS_CompositeNMB`), квадратные дыры в облаках (`CS_SortRayTiles`), тёмные блоки на воде, раздачу 2D-групп `32×2` на NVIDIA, поворот каналов BC7 (только режимы 4/5) и фиолетовый шум зондов освещения.
- **Чистота D3D12, контроль VRAM и ускорение итераций:** исправлены ошибки D3D12 `538` (раздельный `ExecuteCommandLists` для сравнения буферов) и `646` (проверка типа/размера depth SRV в `ambient`), добавлены `VICTORY_RAW_VIEW_CLAMP`, `VICTORY_VRAM_BUDGET_MB`, `VICTORY_SKIP_BINK=1`, `VICTORY_AUTO_A=90`, авто-подкладка `D3D12\D3D12Core.dll` в лаунчерах и починен оффлайн-реплей на Agility SDK `1.620.2`.

### Было vs. Стало (`Stage 7B → Stage 10 Wave 1`, RTX 3060 12 GB + Ryzen 5 5600X)

| Показатель | Было (Stage 7B) | Сейчас (Stages 8 – 10 Wave 1) |
| :--- | :--- | :--- |
| **Лёгкие сцены мира (`1 000–1 750 draws`)** | `13–14 FPS` (`~70–80 ms`) | **`25.9–30.8 FPS`** (`32.5–38.7 ms`) |
| **Тяжёлые сцены мира (`2 800–3 700 draws`)** | `11.2 FPS` (`89.0 ms`) | **`16.9–20.9 FPS`** (`47.8–59.2 ms`) |
| **CPU-подготовка кадра (`~6 300–7 700 draws`)** | `60–69 ms/кадр` | **`36.9 ms/кадр`** (~1.8–2× быстрее) |
| **Простои `BEHIND_PLACED`** | `21.3–23.0 ms/кадр` | **`0.0–1.8 ms/кадр`** |
| **Конкуренция `HostHeap::dynamic`** | `26.9 ms/кадр` | **`0.5 ms/кадр`** |
| **Ожидание `heaps' records`** | `4.2–9.8 ms/кадр` | **`1.8–2.3 ms/кадр`** |
| **D3D12 Debug Layer (`538` / `646`)** | Ошибки `538` (compare) и `646` (depth SRV) | **Исправлены в сабмите и `ambient`** |
| **Качество картинки (NVIDIA 32-lane)** | Полосы 3D LUT, дыры в облаках, блоки воды, фиолет | **Всё исправлено без урезания эффектов** |

---

## Текущий прогресс, замер FPS и характеристики тестового ПК

> Самооценка по открытой методике [PortWatch VI](https://portwatch-vi.xyz/) (`50` базовых баллов за вехи `База → Играбельно` + `17.5` Сюжет + `12.5` Рендеринг + `10` Производительность `[средний FPS ÷ 30]` + `10` Стабильность). `VIctory-Hydra` — самостоятельная объединительная ветка (не подана в официальный рейтинг PortWatch).

<p align="center">
  <img src="docs/progress-ru.svg" alt="Трекер прогресса VIctory-Hydra в стиле PortWatch VI — 82.2%" width="100%">
</p>

### Отдельный прогресс по подсистемам (локальные замеры)
| Категория | Баллы | Текущее состояние |
| :--- | :--- | :--- |
| **Вехи (`0–50%`)** | `50.0 / 50.0 б.` (`100%`) | `База` ● `Запуск .exe` ● `1-й кадр` ● `Меню` ● `В мире` ● `Играбельно` |
| **Сюжет (`17.5 б.`)** | `6.1 / 17.5 б.` (`35%`) | Пролог пройден, открытый мир и HUD работают (поздняя кампания локально ещё не пройдена) |
| **Рендеринг (`12.5 б.`)** | `10.6 / 12.5 б.` (`85%`) | Исправлены 3D LUT, облака, вода, BC7, 32×2 wave ticket и фиолетовые зонды; `305+` шейдерных тестов пройдены |
| **Производительность (`10.0 б.`)** | `7.0 / 10.0 б.` (`70%`) | **`25.9–30.8 FPS`** лёгкие сцены · **`16.9–20.9 FPS`** тяжёлые сцены (`60.0 FPS` меню) к цели `30 FPS` |
| **Стабильность (`10.0 б.`)** | `8.5 / 10.0 б.` (`85%`) | Фиксы D3D12 `538`/`646`, `HostHeap::id`, `VICTORY_RAW_VIEW_CLAMP`, `VICTORY_VRAM_BUDGET_MB`, тесты WARP + RTX зелёные |
| **Итого** | **`82.2%`** | **`+5.4%`** с Stage 7B (`76.8% → 82.2%`) |

### Конфигурация тестового ПК
| Компонент | Характеристики |
| :--- | :--- |
| **ОС** | Windows 11 x64 (сборка `22621`) |
| **Процессор** | AMD Ryzen 5 5600X (6 ядер / 12 потоков, Zen 3) |
| **Видеокарта** | NVIDIA GeForce RTX 3060 12 GB (`ID3D12Device13`, D3D12 Agility SDK `1.620.2`, локальный бюджет ~11 345 MiB VRAM) |
| **ОЗУ** | 32 GB |
| **Накопители** | HDD под дамп игры · системный SSD под кэш шейдеров и пайплайнов (`%LOCALAPPDATA%`) |

### Текущий результат FPS (`1280×720`, `render_scale = 0.667`, `--console series-s`)
- **Меню / экраны загрузки:** `60.0 FPS` (`16.7 ms/кадр`).
- **Лёгкие сцены в мире (`1 020–1 742 draws/frame`):** **`25.9–30.8 FPS`** (`32.5–38.7 ms/кадр`).
- **Тяжёлые сцены в мире (`2 838–3 675 draws/frame`):** **`16.9–20.9 FPS`** (`47.8–59.2 ms/кадр`).
- **Пиковая подгрузка на входе в мир (`7 633–9 391 draws/frame`, холодные пайплайны):** медиана `10.9 FPS`, CPU-подготовка `36.9 ms/кадр` после выхода из всплеска.
- **Следующая цель:** пробить стабильные **`30+ FPS`** и в тяжёлых сценах на 6 ядрах в **Stage 10 Wave 2** (`VICTORY_WATCH_RESERVATION`, `VICTORY_COPY_SMALL`, `VICTORY_ASYNC_PRESENT` + `VICTORY_FRAMES_AHEAD=2`).

### Карта этапов разработки
| Этап | Что сделано / планируется | Статус |
| :--- | :--- | :--- |
| **Stages 1–5** | База `cool_fox` (Pareto-резидентность под 32 GB RAM), 256-шардовые блокировки, `NaN`-очистка, приоритетный прогрев шейдеров персонажей/лиц | Готово |
| **Stages 6–6.1** | Фикс `MSInfo.PayloadSizeInBytes` (разблокировано 1 762 mesh-пайплайна), убрано ~3 964 глобальных UAV-барьера/кадр, бюджеты VRAM, сортировка DirectStorage под HDD | Готово |
| **Stages 7–7B** | Автобенчмарк, живые A/B-тесты и оффлайн-реплей; исправление регрессий (`VICTORY_UM_WATCH=0`, `constant:0.8`); порт 8 фиксов `victory-shader` из `Neptune_VI` | Готово |
| **Stage 8** | `VICTORY_PATCH_LEAN=1`, `VICTORY_INPLACE_CONSTANTS=1`, `VICTORY_UM_COALESCE_KB=0`, фиксы `ir::whole_sections` и `POPS` (`11–14 → 17–31 FPS`) | Готово |
| **Stage 9 (Waves 1–2)** | `VICTORY_PLACED_FORWARD=1`, `VICTORY_DYNAMIC_SLAB=1`, `VICTORY_ASYNC_RECYCLE=1`, lock-free `Shadow::pending`, баланс потоков под 6 ядер, авто-подкладка Agility SDK и фиксы оффлайн-реплея | Готово |
| **Stage 10 Wave 1** | `PLACED_RAW` только в `diagnostics`, `VICTORY_HELPER_PRIORITY`, `HostHeap::id`, `VICTORY_RAW_VIEW_CLAMP`, фиксы D3D12 `538`/`646`, `VICTORY_SKIP_BINK`, `VICTORY_VRAM_BUDGET_MB`, `VICTORY_BUFFERS_AHEAD`, `VICTORY_SAMPLER_ORDER`, `VICTORY_THREAD_TICKET`, фиксы облаков/воды (LDS), BC7 и `VICTORY_EARLY_DEPTH` | Готово |
| **Stage 10 Wave 2** | `VICTORY_WATCH_RESERVATION` + `VICTORY_COPY_SMALL`, `VICTORY_ASYNC_PRESENT` + `VICTORY_FRAMES_AHEAD=2`, `dstorage` `cancel`/`error_record` + `MESH_PRIM_9BIT`, A/B-проверка `VICTORY_WAVE64_PAIRED` | В планах |

---

## Ключевые возможности и перенесённые улучшения

### 1. Рендерер, многопоточная отправка и память
- **Многопоточная запись и асинхронный сабмит (`VICTORY_THIN=1`, `VICTORY_RELAY=2`, `VICTORY_ASYNC_SUBMIT=1`):** выносит трансляцию списков команд D3D12, дескрипторов и AVX2-сравнение грязных страниц с потоков игры на пулы воркеров.
- **Pareto-бюджеты VRAM / RAM (`VICTORY_CARD_BUFFERS_MB`, `VICTORY_CARD_WINDOWS_MB`, `VICTORY_SETTLE_ROOM=1`):** предотвращает переполнение WDDM-пейджинга (`0x8007000E`) и вылеты на 32 GB RAM и видеокартах 8–12 GB.
- **Порты рендерера и быстрые пути:**
  - `VICTORY_PORT_CONSTANTS`, `VICTORY_PORT_DESCRIPTORS`, `VICTORY_PORT_GUARD`, `VICTORY_PORT_JOURNAL`
  - `VICTORY_UP_QUEUE=1`, `VICTORY_JOIN_BARRIERS=1`, `VICTORY_READBACK_POOL=1`, `VICTORY_TEXBACK_ASYNC=1`
  - `VICTORY_UM_TEXLOOK=1` (быстрый индекс текстур), `VICTORY_REMAP_SAME=1` и `VICTORY_ALIAS_RUNS=1`, `VICTORY_UM_COALESCE_KB=0` (устраняет порчу поверхностей)
  - `SEEN_ALL` без блокировок в эмуляции счётчика инструкций и отложенное сохранение кэша пайплайнов (`VICTORY_LIBRARY_STORE_LATER=1`)
- **DirectStorage-ускорение под HDD (`VICTORY_DSTORAGE_IO=handles`, `VICTORY_DSTORAGE_SORT=1`):** многопоточное чтение по хэндлам и сортировка пакетов по смещению для сокращения времени загрузки с HDD.

### 2. Рекомпилятор шейдеров (`victory-shader`) и фиксы под NVIDIA / AMD
- **Обход бага NVIDIA DXIL с LDS atomic sub (`negation_of` + `lds_rmw_loop`):** устраняет отказы компиляции на частицах погоды.
- **Нормализация DXIL CFG и циклов:** лимит трассировки циклов `TRACE_LOOP_LIMIT=1024`, единый выход цикла, `one_mode_per_register` (чинит ~70 шейдеров), `whole_sections` / `permutes_outside_sections` (вода/трансмиттанс), корректный stride для `unused_descriptor_kind`.
- **Покрытие инструкций:** `Input::GroupInfo`, `s_pack_{ll,lh,hh}_b32_b16`, `v_mad_u32_u24` / `u16`, `s_bfm_b32` / `s_mov_b64`, восстановление масок wave64 для вокселей и фикс LOD-слотов текстур.
- **Оффлайн-прогреватель пайплайнов ([Warm-Shaders.cmd](Warm-Shaders.cmd)):** многопоточная предкомпиляция с процентами прогресса, скоростью (`пайплайнов/с`), ETA и безопасным возобновлением через `warm-done.txt`.

### 3. Картинка и апскейлеры
- **Ровное освещение и отражения (`VICTORY_AMBIENT=constant:0.8`, `VICTORY_AMBIENT_REFLECT=0.5`):** убирает пересветы в дверных проёмах и провалы в чёрноту в помещениях, плюс `VICTORY_NAN_SCRUB=1` против `NaN`-артефактов.
- **Стабильность текстур:** `VICTORY_STALE_KEEP_MS=10000` / `VICTORY_STALE_KEEP_MAX_MS=120000` + `VICTORY_TEXTURE_BACK_MODES=0`.
- **Разрешение, масштаб и лимит кадров:** `VICTORY_OUTPUT`, `VICTORY_RENDER_SCALE`, `VICTORY_FRAME_INTERVAL_MS` (`60/120 FPS` без ускорения времени в игре).
- **Перехват ПК-апскейлеров (`VICTORY_FSR_DLL`):** опциональная подмена `ffx_*` на внешнюю DLL (FidelityFX / OptiScaler).

### 4. Автобенчмарк, живой A/B-тест и оффлайн-реплей
- Скрипт [tools/bench-gta6.ps1](tools/bench-gta6.ps1) поддерживает:
  - `live` — автозапуск до мира, прогрев и замер среднего/медианного FPS;
  - `ab` — переключение флагов `VICTORY_*` прямо в реальном кадре мира;
  - `record` / `replay` — запись одного кадра и оффлайн-прогон за ~5 секунд без перезапуска игры.

---

## Требования и необходимые зависимости

- **ОС:** Windows 10 (19041+) / Windows 11 x64
- **Процессор:** x86-64 с поддержкой AVX2 (рекомендуется 6+ ядер)
- **Видеокарта:** Direct3D 12 GPU с Shader Model 6.6 и Mesh Shaders (NVIDIA RTX 20/30/40/50 или AMD RX 6000/7000/9000; от 8 GB VRAM, для 4 GB есть отдельный батник) + свежий драйвер GPU (NVIDIA `566+` / AMD Adrenalin)
- **ОЗУ:** рекомендуется 32 GB
- **Файлы игры:** ваш собственный легальный дамп Xbox Series (`game_scarlett_master.exe` и архивы **вне папки репозитория**)

### Какие зависимости нужно скачать и установить

1. **DirectX 12 Agility SDK (`D3D12\D3D12Core.dll` — ОБЯЗАТЕЛЬНО, строго версия `1.620.2`):**
   - Бинарник `victory.exe` экспортирует `D3D12SDKVersion = 620` и `D3D12SDKPath = ".\D3D12\"`. Если папки `D3D12\D3D12Core.dll` нет **рядом с запускаемым `.exe`**, либо версия ниже `1.620.x` (например `1.614`–`1.619`), либо взята не `x64`, либо скачан `1.7xx-preview` без режима разработчика Windows — система выдаст **`0x887E0003`** (*«Конфигурация версии D3D12 пакета SDK хоста exe недопустима»*) и краш `D3D12 XDXR Error: D3D12SerializeVersionedRootSignature`.
   - Скачайте стабильный пакет [`Microsoft.Direct3D.D3D12` `1.620.2`](https://www.nuget.org/packages/Microsoft.Direct3D.D3D12/1.620.2) (**не** `1.7xx-preview`), достаньте `build\native\bin\x64\D3D12Core.dll` и положите в подпапку `D3D12\D3D12Core.dll` **в ту же папку, откуда запускается `victory.exe`** (или выполните команды ниже в PowerShell):
     ```powershell
     # Скачивает D3D12Core.dll (Agility SDK 1.620.2 x64) в target\release\D3D12\
     New-Item -ItemType Directory -Force "target\release\D3D12" | Out-Null
     Invoke-WebRequest "https://api.nuget.org/v3-flatcontainer/microsoft.direct3d.d3d12/1.620.2/microsoft.direct3d.d3d12.1.620.2.nupkg" -OutFile "$env:TEMP\d3d12_agility.zip"
     Expand-Archive "$env:TEMP\d3d12_agility.zip" "$env:TEMP\d3d12_agility" -Force
     Copy-Item "$env:TEMP\d3d12_agility\build\native\bin\x64\D3D12Core.dll" "target\release\D3D12\" -Force
     # Проверка (должно показать путь к D3D12Core.dll и ID3D12Device13 true):
     .\target\release\victory.exe probe
     ```
2. **Microsoft Visual C++ Redistributable 2015–2022 (x64):** нужен для `VCRUNTIME140.dll` и нативных DLL игры ([прямая ссылка](https://aka.ms/vs/17/release/vc_redist.x64.exe)).
3. **Для сборки из исходников:**
   - **Rustup** ([rustup.rs](https://rustup.rs/)) — сам подтянет точную версию Rust из `rust-toolchain.toml`
   - **Visual Studio 2022 Build Tools** (рабочая нагрузка *Разработка классических приложений на C++* + *Windows 10/11 SDK*)
   - **Git**

Быстрая установка всех системных зависимостей через `winget` (в PowerShell):
```powershell
winget install Microsoft.VCRedist.2015+.x64
winget install Git.Git
winget install Rustlang.Rustup
winget install Microsoft.VisualStudio.2022.BuildTools --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

---

## Быстрый старт

### 1. Сборка из исходников

```powershell
git clone https://github.com/ToLIManl/VIctory-Hydra.git
cd VIctory-Hydra

# Release-сборка для игры
cargo build --release --locked -p victory

# Опционально: диагностическая сборка для A/B-тестов и реплея кадра
cargo build --locked --profile diag -p victory --features diagnostics
```

### 2. Указать путь к дампу игры

Задайте переменную `VICTORY_GAME_DIR` (папка с `game_scarlett_master.exe`), либо положите репозиторий рядом с папкой `..\Game\`:

```powershell
$env:VICTORY_GAME_DIR = "D:\Games\MyDump"
```

### 3. Запуск

```powershell
# Основной объединённый пресет (720p для максимального FPS)
.\Play-VIctory.cmd

# Пресет для видеокарт с 4 GB VRAM
.\Play-VIctory-4GB.cmd
```

Запуск в `1080p` вместо базового `720p`:

```powershell
$env:VICTORY_OUTPUT = "1920x1080"
$env:VICTORY_RENDER_SCALE = "0.75"
.\Play-VIctory.cmd
```

### 4. Прогрев шейдеров и пайплайнов (рекомендуется после обновлений)

```powershell
.\Warm-Shaders.cmd
```

Прогресс сохраняется в `warm-done.txt`, можно прервать (`Ctrl+C`) и продолжить позже с того же места.

---

## Бенчмарки и обновление репозитория

```powershell
# Автобенчмарк 20 секунд в игровом мире (Release)
.\tools\bench-gta6.ps1 -Mode live -WorldS 20 -Tag baseline

# Живой A/B-тест любого флага в одном запуске
.\tools\bench-gta6.ps1 -Mode ab -PhaseS 10 -AbSwitches "VICTORY_UM_TEXLOOK=0,VICTORY_UM_TEXLOOK=1" -Tag ab-test

# Запись и 5-секундный оффлайн-реплей одного кадра
.\tools\bench-gta6.ps1 -Mode record -Tag cap1
.\tools\bench-gta6.ps1 -Mode replay
```

Проверка политики ассетов и безопасный коммит изменений (с защитой от утечки локальных путей):

```powershell
cargo xtask install-hooks
cargo xtask policy
.\tools\push-update.ps1 -Message "Stage 8: краткое описание" -Push
```

---

## Благодарности и исходные проекты

Проект объединяет наработки сообщества VIctory / PortWatch:

- **[VIctory](https://github.com/Marvisak/VIctory)** (Marvisak и контрибьюторы) — оригинальный загрузчик, HLE-сервисы, D3D12-бэкенд и рекомпилятор шейдеров.
- **cool_fox / CGBJ** (codec-d и DoubleGhosty) — Pareto-резидентность памяти и базовая стабильность на NVIDIA.
- **Neptune_VI** — улучшения нижнего уровня шейдеров (`one_mode_per_register`, `whole_sections`, обход NVIDIA LDS atomics, отражённый свет, коалесцинг страниц и реплей).
- **[fxceless/victory-fork](https://github.com/fxceless/victory-fork)** — `VICTORY_THIN`, асинхронный сабмит, relay/landing-пайплайн, рекордер/реплей кадра и диагностика.
- **AvatarManz (`VIctory-7900XTX`)** — многопоточная запись команд, разблокировка фреймрейта и порты рендерера.
- **Vice-Tay**, **Kelvaroq**, **[DexNom/VIctory](https://github.com/DexNom/VIctory)**, **[Belobug/VIctory-Belobug-AMD](https://github.com/Belobug/VIctory-Belobug-AMD)**, **[myokex/victory-amd](https://github.com/myokex/victory-amd)**, **[FQY55kai/ZJOU-VICTORY](https://github.com/FQY55kai/ZJOU-VICTORY)**, **Geckomacabre**, **cxy0072019-sketch**, **romainhedouin**.

---

## Лицензия

**GPL-3.0-or-later**, как и оригинальный VIctory. Подробности в [LICENSE](LICENSE) и [docs/asset-policy.md](docs/asset-policy.md).
