# McpGo для Windows

**Tauri 2 · Rust · React 19 · TypeScript · Tailwind CSS 4**

Центр управления MCP-инфраструктурой разработчика: интерактивный терминал с
отслеживанием задач, управление браузером через CDP, туннели OpenAI Secure
MCP Tunnels и фоновые Windows-службы. Порт macOS-проекта **McpGo** на
Windows.

**Статус: v0.1.0 — все шесть этапов дорожной карты выполнены.**

## Установка

**Самый простой путь — ничего устанавливать не нужно.** GitHub Actions
собирает готовые установщики автоматически: откройте страницу
[**Releases**](https://github.com/AligmentInsight/mcpgo-windows/releases),
скачайте `McpGo_0.1.0_x64-setup.exe` и запустите двойным кликом.
Требуется только Windows 10/11 x64 — WebView2 Runtime ставится
инсталлятором автоматически.

## Возможности

| Раздел | Что делает |
|---|---|
| **Terminal** | Задачи PowerShell с полным жизненным циклом: каждая команда — фоновая задача с портами `job_status` / `job_logs` из macOS-оригинала (`jobs\status\<id>.json`, `jobs\logs\<id>.log`, формат `[RFC3339-UTC] OUT\|ERR\|SYS <текст>`), живой вывод, отмена, история |
| **Browser** | Сессии Chrome с профилями пользователя: запуск `chrome.exe` с debug-портом из общего пула 9222..=9321, подключение к уже запущенным debug-инстансам, список целей CDP (`/json/list`), события `browser://status` |
| **Tunnels** | Туннели OpenAI Secure MCP Tunnels: определения с 11 регионами residency (global/us/eu/au/ca/gb/in/jp/kr/sg/ae), генерация YAML-профилей (`config_version 1`), управляемые ключи в `secrets\<id>.key` (значение не пересекает IPC), health-проба `/healthz` в диапазоне 18080..=18179, `keep_alive` с бюджетом перезапусков, автостарт |
| **Services** | Windows-службы вместо LaunchAgents: определения `services\<id>.json`, установка через самоэлевирующиеся PowerShell-скрипты (UAC), служба `McpGo-<id>` запускает сервис-воркера (`--service-host`), который супервизит runtime-процесс node / python / powershell |
| **Environment** | Диагностика машины: PowerShell 7/5.1, Node.js, Chrome, Python — пути найденных инструментов, предупреждения и блокирующие проблемы |

Ядро — 32 типизированные IPC-команды Tauri и 4 канала событий
(`job://status`, `job://output`, `browser://status`, `tunnel://status`).
Тяжёлая работа — только в Rust: PowerShell, SCM, CDP, профили туннелей.
Фронтенд вызывает команды и слушает события, прямые полномочия webview
минимальны (`capabilities/default.json`).

## Сборка из исходников

Требования: Windows 10/11 x64, Rust 1.90+ (rustup, стабильный канал),
Node.js 20.19+ (LTS). WebView2 Runtime ставится инсталлятором автоматически.

```powershell
git clone https://github.com/AligmentInsight/mcpgo-windows.git
cd mcpgo-windows
npm install
npm run tauri dev
```

Сборка дистрибутива (MSI + NSIS-инсталлятор):

```powershell
npm run tauri build
# → src-tauri\target\release\bundle\msi\*.msi
# → src-tauri\target\release\bundle\nsis\*-setup.exe
```

## Структура репозитория

```
mcpgo-windows/
├─ src/                    ФРОНТЕНД (React + TypeScript + Tailwind)
│  ├─ api/                 типизированный слой invoke()/listen()
│  ├─ components/          5 разделов UI: terminal, browser, tunnels, services, environment
│  ├─ hooks/               useJobs, useBrowserSessions, useTunnels, useServices, …
│  ├─ types/               зеркала wire-типов Rust-структур
│  └─ utils/               форматирование
├─ src-tauri/              ЯДРО (Rust, Tauri 2)
│  ├─ src/
│  │  ├─ main.rs           bootstrap, сервис-режим --service-host, диагностика окружения
│  │  ├─ paths.rs          файловый макет (единственный источник истины)
│  │  ├─ logging.rs        журналирование с ротацией
│  │  ├─ terminal/         запуск PowerShell (-NoLogo -NoProfile, CREATE_NO_WINDOW)
│  │  ├─ jobs/             реестр задач: job_status / job_logs
│  │  ├─ browser/          chrome.rs + cdp.rs: сессии и DevTools-протокол
│  │  ├─ tunnels/          definition, profile, client, health, супервизор
│  │  ├─ services/         definition, scm, installer, worker
│  │  └─ commands/         32 IPC-команды по доменам
│  ├─ capabilities/        модель разрешений Tauri v2
│  └─ tauri.conf.json      окно, CSP, bundle (msi + nsis)
└─ docs/
   ├─ ARCHITECTURE.md      архитектура и таблица переноса macOS → Windows
   └─ AGENT-SETUP.md       как подключить ИИ-ассистента к этим инструментам
```

## Данные приложения

| Каталог | Содержимое |
|---|---|
| `%APPDATA%\com.mcpgo.windows\` | `jobs\`, `services\`, `browser\`, `tunnels\` (определения, профили, `secrets\`, `bin\`) |
| `%LOCALAPPDATA%\com.mcpgo.windows\logs\` | журнал приложения с ротацией |

Пути вычисляет `src-tauri/src/paths.rs`; идентификатор синхронен с
`tauri.conf.json` и scope в capabilities.

## Дорожная карта

| Этап | Содержание | Статус |
|---|---|---|
| 1. Инициализация и структура | Tauri-конфигурация, capabilities, журналирование, проверка окружения | ✅ |
| 2. Каркас фронтенда | React + Tailwind, боковая навигация, панель диагностики, мост invoke/events | ✅ |
| 3. Терминал и задачи | PowerShell-мост, портированный job_status/job_logs | ✅ |
| 4. Управление браузером | Chrome/CDP-сессии, UI профилей | ✅ |
| 5. Туннели OpenAI | Supervision Secure MCP Tunnels, UI туннелей | ✅ |
| 6. Службы и дистрибуция | Windows-службы (SCM), инсталлятор MSI/NSIS | ✅ |

## Документация

- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — архитектура, модель
  безопасности, файловый макет, инвентарь IPC.
- [`docs/AGENT-SETUP.md`](docs/AGENT-SETUP.md) — практическое руководство:
  как превратить ИИ-ассистента в агента с инструментами McpGo.

## Конвенции

- Комментарии и документация — на русском; runtime-строки журналов — на
  английском (краткие, машиночитаемые).
- Заглушки в коде запрещены: без `TODO`, `FIXME`, `unimplemented!`.
- Модули изолированы: домены не зависят от внутренностей друг друга.
- Wire-типы TS зеркалят Rust-структуры camelCase-полями; контракт
  проверяется статически на каждом домене.

## Лицензия

MIT — см. [LICENSE](LICENSE).
