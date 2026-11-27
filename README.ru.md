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

## Текущий прогресс, замер FPS и характеристики тестового ПК

> Самооценка по открытой методике [PortWatch VI](https://portwatch-vi.xyz/) (`50` базовых баллов за вехи `База → Играбельно` + `17.5` Сюжет + `12.5` Рендеринг + `10` Производительность `[средний FPS ÷ 30]` + `10` Стабильность). `VIctory-Hydra` — самостоятельная объединительная ветка (не подана в официальный рейтинг PortWatch).

<p align="center">
  <img src="docs/progress-ru.svg" alt="Трекер прогресса VIctory-Hydra в стиле PortWatch VI — 76.8%" width="100%">
</p>

### Отдельный прогресс по подсистемам (локальные замеры)
| Категория | Баллы | Текущее состояние |
| :--- | :--- | :--- |
| **Вехи (`0–50%`)** | `50.0 / 50.0 б.` (`100%`) | `База` ● `Запуск .exe` ● `1-й кадр` ● `Меню` ● `В мире` ● `Играбельно` |
| **Сюжет (`17.5 б.`)** | `6.1 / 17.5 б.` (`35%`) | Пролог пройден, открытый мир и HUD работают (поздняя кампания локально ещё не пройдена) |
| **Рендеринг (`12.5 б.`)** | `9.4 / 12.5 б.` (`75%`) | Персонажи, текстуры, тени и отражения работают корректно; `297/298` шейдерных тестов пройдены |
| **Производительность (`10.0 б.`)** | `3.8 / 10.0 б.` (`37.5%`) | `11.24 FPS` средний за 20с (`11.71` медианный) · `13–14 FPS` в живой игре (`60.0 FPS` меню) от цели `30 FPS` |
| **Стабильность (`10.0 б.`)** | `7.5 / 10.0 б.` (`75%`) | Pareto 32 GB и 256-шардовые блокировки держат память; длительный прогон Stage 7B в процессе |
| **Итого** | **`76.8%`** | Главный упор подтверждён: **CPU-подготовка `31–69 ms` при GPU RTX 3060 всего `8.9–17.5 ms`** |

### Конфигурация тестового ПК
| Компонент | Характеристики |
| :--- | :--- |
| **ОС** | Windows 11 x64 (сборка `22621`) |
| **Процессор** | AMD Ryzen 5 5600X (6 ядер / 12 потоков, Zen 3) |
| **Видеокарта** | NVIDIA GeForce RTX 3060 12 GB (`ID3D12Device13`, D3D12 Agility SDK `1.620.2`, локальный бюджет ~11 345 MiB VRAM) |
| **ОЗУ** | 32 GB |
| **Накопители** | HDD под дамп игры · системный SSD под кэш шейдеров и пайплайнов (`%LOCALAPPDATA%`) |

### Текущий результат FPS (`1280×720`, `render_scale = 0.667`, `--console series-s`, Release)
- **Меню / экраны загрузки:** `60.0 FPS` (`16.7 ms/кадр`).
- **Автобенчмарк в открытом мире (`205` кадров за `20.0с`, в среднем `6 284` draw-вызова на кадр):**
  - **Средний (mean):** `11.24 FPS` (`89.0 ms/кадр`) · **Медианный:** `11.71 FPS` (`85.4 ms/кадр`) · `p90 = 104.5 ms`.
- **Живой геймплей в мире:** **`13–14 FPS`** (`69.3–79.5 ms/кадр` при `4 444–4 786 draws/frame`) с корректной геометрией персонажей, тенями, отражённым освещением и HUD.
- **Куда уходит время кадра:** Сама RTX 3060 отрабатывает GPU-часть за **`8.9–17.5 ms/кадр`** (до `~34.9 ms` в моменты подгрузки стриминга). Главный упор сейчас — **подготовка кадра и отправка draw-вызовов на CPU (`31–69 ms/кадр`)** на 6 ядрах. Поэтому приоритет **Stage 8** — разгрузить CPU до стабильных **`30+ FPS`** на 720p, а уже затем поднимать разрешение (1080p) и качество с апскейлерами.

### Карта этапов разработки
| Этап | Что сделано / планируется | Статус |
| :--- | :--- | :--- |
| **Stages 1–5** | База `cool_fox` (Pareto-резидентность памяти под 32 GB RAM), 256-шардовые блокировки страниц, очистка `NaN` в пикселях, приоритетный прогрев шейдеров персонажей/лиц | Готово |
| **Stages 6–6.1** | Фикс `MSInfo.PayloadSizeInBytes` (разблокировано 1 762 mesh-пайплайна), убрано ~3 964 лишних глобальных UAV-барьера на кадр, 1 GiB буферов перенесён в VRAM RTX 3060, сортировка DirectStorage под HDD | Готово |
| **Stages 7–7B** | Автобенчмарк, живые A/B-тесты и оффлайн-реплей кадра; исправление регрессий (`VICTORY_UM_WATCH=0`, `constant:0.8`); порт 8 фиксов `victory-shader` из `Neptune_VI` (NVIDIA LDS atomics, CFG-секции, `one_mode_per_register`, лимит BVH-циклов) | Готово (прогрев шейдеров и тест) |
| **Stage 8** | Следующая волна разгрузки CPU к цели **30+ FPS**: `VICTORY_GPU_DETILE`, `VICTORY_SEGMENTS_ACROSS`, `VICTORY_SPREAD_LEAN`, `VICTORY_ASYNC_PRESENT`, `VICTORY_HEAP_WORKER` | В планах |

---

## Ключевые возможности и перенесённые улучшения

### 1. Рендерер, многопоточная отправка и память
- **Многопоточная запись и асинхронный сабмит (`VICTORY_THIN=1`, `VICTORY_RELAY=2`, `VICTORY_ASYNC_SUBMIT=1`):** выносит трансляцию списков команд D3D12, дескрипторов и AVX2-сравнение грязных страниц с потоков игры на пулы воркеров.
- **Pareto-бюджеты VRAM / RAM (`VICTORY_CARD_BUFFERS_MB`, `VICTORY_CARD_WINDOWS_MB`, `VICTORY_SETTLE_ROOM=1`):** предотвращает переполнение WDDM-пейджинга (`0x8007000E`) и вылеты на 32 GB RAM и видеокартах 8–12 GB.
- **Порты рендерера и быстрые пути:**
  - `VICTORY_PORT_CONSTANTS`, `VICTORY_PORT_DESCRIPTORS`, `VICTORY_PORT_GUARD`, `VICTORY_PORT_JOURNAL`
  - `VICTORY_UP_QUEUE=1`, `VICTORY_JOIN_BARRIERS=1`, `VICTORY_READBACK_POOL=1`, `VICTORY_TEXBACK_ASYNC=1`
  - `VICTORY_UM_COALESCE_KB=64` (слияние страниц памяти), `VICTORY_UM_TEXLOOK=1` (быстрый индекс текстур), `VICTORY_REMAP_SAME=1` и `VICTORY_ALIAS_RUNS=1`
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
