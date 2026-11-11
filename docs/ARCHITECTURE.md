# Архитектура McpGo для Windows

> Финальная архитектурная справка порта macOS-проекта McpGo на Windows.
> Все шесть этапов дорожной карты выполнены; документ описывает
> реализованное состояние.

## 1. Назначение приложения

McpGo — центр управления MCP-инфраструктурой разработчика. Приложение
даёт четыре инструментальных домена и диагностику окружения:

- **Терминал и задачи** — каждая команда PowerShell выполняется фоновым
  процессом (`-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass`,
  `CREATE_NO_WINDOW`), отслеживается реестром задач и оставляет
  машиночитаемые артефакты `job_status` / `job_logs`;
- **Управление браузером** — сессии Chrome с изолированными
  user-data-профилями и CDP debug-портом из пула 9222..=9321, а также
  подключение к уже запущенным debug-инстантам;
- **Туннели** — supervision туннелей OpenAI Secure MCP Tunnels: от
  определения до YAML-профиля, health-пробы `/healthz` (пул 18080..=18179)
  и автоматического перезапуска;
- **Фоновые службы** — те же возможности оформлены Windows-службами SCM:
  установка через самоэлевирующиеся PowerShell-скрипты, супервизия
  runtime-процессов (node / python / powershell) сервис-воркером.

## 2. Перенос компонентов: macOS → Windows

| Компонент macOS-оригинала | Реализация в Windows-порте |
|---|---|
| `native/` — GUI на SwiftUI/AppKit | `src/` (React + Tailwind CSS) + `src-tauri/` (Rust, ядро Tauri 2) |
| `launchagents/` — LaunchAgents (plist + launchctl) | домен `services/`: определения `services\<id>.json` + SCM + PowerShell-инсталляторы |
| Bash / Zsh | PowerShell 7 (`pwsh`) с откатом на PowerShell 5.1 |
| `job_status` / `job_logs` | портированы 1:1: `jobs\status\<id>.json` (атомарная запись), `jobs\logs\<id>.log` (`[RFC3339-UTC] OUT\|ERR\|SYS <текст>`) |
| Playwright-даемон Chrome | `browser/chrome.rs` + `browser/cdp.rs`: один `chrome.exe` на сессию, подтверждение CDP-эндпоинта, `/json/list` |
| OpenAI Secure MCP Tunnels | домен `tunnels/`: определения, генератор профилей, супервизор `mcp-tunnel run --profile-file` |
| launchctl | Service Control Manager + командлеты PowerShell |
| `~/Library/Application Support/<app>` | `%APPDATA%\com.mcpgo.windows` — данные |
| `~/Library/Logs/<app>` | `%LOCALAPPDATA%\com.mcpgo.windows\logs` — журналы |

## 3. Слои приложения

```text
┌────────────────────────────────────────────────────────────┐
│  Фронтенд (src/, React 19 + TypeScript + Tailwind 4)       │
│  5 разделов: terminal · browser · tunnels · services ·     │
│  environment. Типизированный слой src/api/ — invoke() и    │
│  listen(); состояние — хуки src/hooks/                     │
├────────────── IPC: 32 команды, 4 канала событий ──────────┤
│  Ядро (src-tauri/src/, Rust)                               │
│  terminal/  — запуск процессов PowerShell                  │
│  jobs/      — реестр задач: статусы, журналы, отмена       │
│  browser/   — реестр сессий, chrome.rs, cdp.rs             │
│  tunnels/   — definition, profile, client, health, mod     │
│  services/  — definition, scm, installer, worker           │
│  commands/  — IPC-обёртки по доменам                       │
│  paths.rs / logging.rs / windows_env.rs / main.rs          │
├────────────────────────────────────────────────────────────┤
│  ОС Windows: SCM, UAC, WebView2, процессы без консоли      │
└────────────────────────────────────────────────────────────┘
```

Два режима одного исполняемого файла:

1. **GUI** — обычный запуск: окно Tauri, пять разделов.
2. **Сервис-воркер** — `McpGo.exe --service-host <id> --data-dir <dir>`:
   безголовый режим для SCM, супервизит runtime-процесса одного
   определения службы (см. `services/worker.rs`). Обычные runtime-процессы
   не умеют протокол служб, поэтому SCM регистрирует наш воркер, а он уже
   запускает и сопровождает node / python / powershell.

## 4. Инвентарь IPC

| Домен | Команды | События |
|---|---|---|
| terminal (jobs) | `start_job`, `cancel_job`, `list_jobs`, `get_job`, `read_job_log` | `job://status`, `job://output` |
| browser | `browser_profile`, `launch_browser`, `list_browser_sessions`, `get_browser_session`, `stop_browser_session`, `read_browser_targets` | `browser://status` |
| tunnels | `create_tunnel_definition`, `update_tunnel_definition`, `delete_tunnel_definition`, `get_tunnel_definition`, `list_tunnels`, `tunnels_profile`, `set_tunnel_api_key`, `start_tunnel`, `stop_tunnel`, `read_tunnel_log` | `tunnel://status` |
| services | `create_service_definition`, `update_service_definition`, `delete_service_definition`, `get_service_definition`, `list_services`, `install_service`, `uninstall_service`, `start_service`, `stop_service`, `restart_service`, `read_service_log` | — |
| environment | `get_environment_report` | — |

Wire-контракт: TS-интерфейсы в `src/types/` зеркалят Rust-структуры
(camelCase ↔ snake_case), перечисления — строковые литералы. Событие
`job://output` — сигнал инвалидации: источник истины по логам — файл,
UI перечитывает его командой `read_job_log`.

## 5. Модель безопасности Tauri v2

Разрешения фронтенда декларируются в `src-tauri/capabilities/default.json`
и проверяются сборкой (`tauri-build` останавливает компиляцию на
несуществующее разрешение). Действует правило **«тяжёлая работа — только в
Rust»**: PowerShell, SCM, CDP, работа с ключами выполняются в ядре; webview
получает только типизированные результаты.

Секреты туннелей не пересекают IPC: определение хранит ссылку
`ApiKeySource` (managed / env / file), значение управляемого ключа пишется
в `tunnels\secrets\<id>.key` командой ядра, возвращающей unit, и в
профиль попадает только форма `file:<путь>` / `env:<имя>`. Значения ключей
не журналируются и не отображаются повторно.

Доверенная сторона терминала — локальный пользователь: команды вводятся
в McpGo UI и выполняются с правами этого же пользователя на его машине
(модель идентична macOS-оригиналу).

## 6. Файловый макет данных

```
%APPDATA%\com.mcpgo.windows\
├─ jobs\
│  ├─ status\<id>.json     job_status: атомарно перезаписывается при каждом переходе
│  └─ logs\<id>.log        job_logs: append-only, одна запись на строку
├─ services\<id>.json      определения служб (аналог plist)
├─ browser\                данные домена браузера (profiles/)
└─ tunnels\
   ├─ <id>.json            определения туннелей
   ├─ profiles\<id>.yaml   сгенерированные профили запуска
   ├─ secrets\<id>.key     управляемые ключи (значение не покидает ядро)
   └─ bin\                 размещение mcp-tunnel.exe (если не на PATH)
%LOCALAPPDATA%\com.mcpgo.windows\
└─ logs\                   журнал приложения с ротацией
```

Пути вычисляет `src-tauri/src/paths.rs` — единственный источник истины,
синхронный с `identifier` в `tauri.conf.json` и scope в capabilities.

## 7. Жизненный цикл запуска

1. `main` проверяет `--service-host` — при наличии запускает воркер и
   выходит (GUI не строится).
2. Инициализация журналирования (первым — чтобы setup уже логировался).
3. `windows_env::validate()` — детекция PowerShell 7/5.1, Chrome, Node.js,
   Python; создание файлового макета; отчёт `EnvironmentReport` публикуется
   для раздела Environment.
4. Регистрация хранилищ доменов: `jobs`, `services`, `browser`,
   `tunnels` (создание каталогов, `TunnelRegistry` управляется как
   состояние Tauri).
5. Автостарт: туннели, помеченные `autostart`, запускаются последовательно.
6. Приложение не блокируется при неготовом окружении: диагностика
   показывается в UI, все события уже в журнале.
7. При выходе — аккуратный `shutdown()` реестра туннелей.

## 8. Контракты доменов (кратко)

- **Терминал/задачи**: жизненный цикл `Running → Completed | Failed |
  Canceled`; журнал — источник истины, UI читает его по инвалидации.
- **Браузер**: пул портов 9222..=9321; `Starting → Running` (CDP-эндпоинт
  ответил за 15 с) `→ Stopped | Failed`; список целей — `/json/list` с
  маппингом `type → kind`.
- **Туннели**: 11 регионов residency; профиль `config_version 1`:
  `control_plane` (base_url по региону, tunnel_id, api_key-ссылка),
  `log` (info, ndjson), `health` (`127.0.0.1:<порт>`, пул 18080..=18179),
  один mcp-binding канала `main` (`server_urls` для HTTP-целей,
  `commands` для stdio); все скаляры одинарно-квочены. Запуск:
  `mcp-tunnel run --profile-file <путь>`, без консольного окна; резолюция
  бинаря override → PATH → `tunnels\bin`. Один рантайм на туннель;
  `keep_alive` с бюджетом перезапусков; изменение определения при живом
  рантайме запрещено.
- **Службы**: префикс `McpGo-<id>`; установка/удаление/управление —
  PowerShell-скрипты с самоэлевацией UAC; удаление определения при живой
  службе запрещено (иначе осиротевшая служба).

## 9. Конвенции

- Комментарии и документация — на русском; runtime-строки журналов — на
  английском (краткие, машиночитаемые).
- Заглушки запрещены (без `TODO`/`FIXME`/`unimplemented!`).
- Модули изолированы: домены не зависят от внутренностей друг друга.
- Wire-контракты доменов проверяются статически: скрипты проверки
  сверяют TS `invoke()` ↔ Rust `#[tauri::command]`, поля wire-типов,
  перечисления, имена событий и пути импортов фронтенда.
