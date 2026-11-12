#!/usr/bin/env node
/**
 * McpGo Swiss-Knife — MCP-сервер «швейцарский нож» для вашей Windows-машины.
 *
 * Даёт подключённому ИИ-клиенту пять инструментов:
 *   1. run_command — выполнять команды PowerShell и получать вывод;
 *   2. read_file   — читать файлы;
 *   3. write_file  — писать файлы (с созданием папок);
 *   4. list_dir    — смотреть содержимое папок;
 *   5. get_disks   — видеть диски и свободное место.
 *
 * Особенности:
 *   - НОЛЬ зависимостей: не нужен npm install, работает любым Node.js 16+;
 *   - протокол MCP (JSON-RPC 2.0) поверх stdio реализован вручную;
 *   - stdout занимается ТОЛЬКО протоколом — все журналы идут в stderr;
 *   - режим самопроверки: `node server.js --selftest`.
 *
 * Безопасность: сервер выполняет действия с правами того пользователя,
 * который его запустил. Подключайте его только к своим ИИ-клиентам и
 * только через доверенные каналы (локальный запуск или Secure MCP Tunnel).
 */
'use strict';

const fs = require('fs');
const path = require('path');
const os = require('os');
const { spawn } = require('child_process');
const readline = require('readline');

const SERVER_NAME = 'mcpgo-swiss-knife';
const SERVER_VERSION = '1.0.0';
const PROTOCOL_VERSION = '2025-06-18';

const MAX_OUTPUT_BYTES = 1024 * 1024; // 1 МБ на поток вывода команды
const DEFAULT_TIMEOUT_SECONDS = 60;
const MAX_TIMEOUT_SECONDS = 600;
const DEFAULT_READ_BYTES = 256 * 1024; // 256 КБ чтения файла по умолчанию

// ---------------------------------------------------------------------------
// Утилиты
// ---------------------------------------------------------------------------

function log(...args) {
  // Только в stderr — stdout зарезервирован под протокол MCP.
  process.stderr.write('[swiss-knife] ' + args.join(' ') + '\n');
}

function requireString(value, name) {
  if (typeof value !== 'string' || value.trim() === '') {
    throw new Error('аргумент "' + name + '" обязателен и должен быть непустой строкой');
  }
  return value.trim();
}

function clampNumber(value, fallback, max) {
  const n = Number(value);
  if (!Number.isFinite(n) || n <= 0) return fallback;
  return Math.min(Math.floor(n), max);
}

function killTree(child) {
  if (!child || child.pid == null || child.exitCode !== null) return;
  try {
    if (process.platform === 'win32') {
      spawn('taskkill', ['/PID', String(child.pid), '/T', '/F'], { windowsHide: true });
    } else {
      child.kill('SIGKILL');
    }
  } catch (_) {
    try { child.kill(); } catch (_) { /* уже мёртв */ }
  }
}

// ---------------------------------------------------------------------------
// Инструмент 1: run_command
// ---------------------------------------------------------------------------

async function toolRunCommand(args) {
  const command = requireString(args.command, 'command');
  const timeoutSeconds = clampNumber(
    args.timeoutSeconds, DEFAULT_TIMEOUT_SECONDS, MAX_TIMEOUT_SECONDS);
  let cwd;
  if (args.workingDir != null) {
    cwd = requireString(args.workingDir, 'workingDir');
    if (!fs.existsSync(cwd)) {
      throw new Error('рабочая папка не найдена: ' + cwd);
    }
  }

  // Префикс: заставляем PowerShell 5.1 отдавать UTF-8 (иначе русские буквы
  // приходят в кодировке OEM и превращаются в кракозябры).
  const fullCommand =
    'try { [Console]::OutputEncoding = [System.Text.Encoding]::UTF8 } catch { }; ' +
    command;

  return new Promise((resolve, reject) => {
    const child = spawn('powershell.exe', [
      '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass',
      '-Command', fullCommand,
    ], { cwd: cwd || undefined, windowsHide: true });

    let out = Buffer.alloc(0);
    let err = Buffer.alloc(0);
    let truncated = false;
    let timedOut = false;

    const append = (buf, chunk) => {
      if (buf.length + chunk.length > MAX_OUTPUT_BYTES) {
        truncated = true;
        return Buffer.concat([buf, chunk.subarray(0, MAX_OUTPUT_BYTES - buf.length)]);
      }
      return Buffer.concat([buf, chunk]);
    };

    child.stdout.on('data', (c) => { out = append(out, c); });
    child.stderr.on('data', (c) => { err = append(err, c); });

    const timer = setTimeout(() => {
      timedOut = true;
      killTree(child);
    }, timeoutSeconds * 1000);

    child.on('error', (e) => {
      clearTimeout(timer);
      reject(new Error('не удалось запустить PowerShell: ' + e.message));
    });

    child.on('close', (code) => {
      clearTimeout(timer);
      const parts = [];
      parts.push('=== код выхода: ' + (code === null ? 'нет (процесс остановлен)' : code) + ' ===');
      if (out.length) parts.push('--- stdout ---\n' + out.toString('utf8').trimEnd());
      if (err.length) parts.push('--- stderr ---\n' + err.toString('utf8').trimEnd());
      if (timedOut) {
        parts.push('--- ВНИМАНИЕ: команда не уложилась в ' + timeoutSeconds +
          ' с и была остановлена ---');
      }
      if (truncated) parts.push('--- вывод обрезан до 1 МБ ---');
      resolve(parts.join('\n'));
    });
  });
}

// ---------------------------------------------------------------------------
// Инструмент 2: read_file
// ---------------------------------------------------------------------------

async function toolReadFile(args) {
  const target = requireString(args.path, 'path');
  const maxBytes = clampNumber(args.maxBytes, DEFAULT_READ_BYTES, 4 * 1024 * 1024);
  const stat = fs.statSync(target); // бросает понятную ошибку, если пути нет
  if (stat.isDirectory()) {
    throw new Error('это папка, а не файл — для папок есть list_dir: ' + target);
  }
  const handle = fs.openSync(target, 'r');
  try {
    const buffer = Buffer.alloc(Math.min(stat.size, maxBytes));
    const read = fs.readSync(handle, buffer, 0, buffer.length, 0);
    const text = buffer.subarray(0, read).toString('utf8');
    const tail = stat.size > read
      ? '\n--- обрезано: показано ' + read + ' из ' + stat.size + ' байт ---'
      : '';
    return '=== ' + target + ' (' + stat.size + ' байт, изменён ' +
      stat.mtime.toISOString() + ') ===\n' + text + tail;
  } finally {
    fs.closeSync(handle);
  }
}

// ---------------------------------------------------------------------------
// Инструмент 3: write_file
// ---------------------------------------------------------------------------

async function toolWriteFile(args) {
  const target = requireString(args.path, 'path');
  const content = typeof args.content === 'string' ? args.content : '';
  fs.mkdirSync(path.dirname(target), { recursive: true });
  fs.writeFileSync(target, content, 'utf8');
  const bytes = Buffer.byteLength(content, 'utf8');
  return 'записано: ' + target + ' (' + bytes + ' байт)';
}

// ---------------------------------------------------------------------------
// Инструмент 4: list_dir
// ---------------------------------------------------------------------------

async function toolListDir(args) {
  const target = requireString(args.path, 'path');
  const entries = fs.readdirSync(target, { withFileTypes: true });
  const rows = [];
  for (const entry of entries) {
    let size = '';
    let mtime = '';
    try {
      const stat = fs.statSync(path.join(target, entry.name));
      size = entry.isDirectory() ? '' : String(stat.size);
      mtime = stat.mtime.toISOString().replace('T', ' ').slice(0, 16);
    } catch (_) { /* нет доступа к метаданным — показываем без них */ }
    rows.push({
      name: entry.name,
      type: entry.isDirectory() ? 'папка' : (entry.isSymbolicLink() ? 'ссылка' : 'файл'),
      size,
      mtime,
    });
  }
  rows.sort((a, b) => (a.type === b.type ? a.name.localeCompare(b.name)
    : (a.type === 'папка' ? -1 : 1)));
  const lines = ['=== ' + target + ' — ' + rows.length + ' записей ==='];
  const width = Math.max(...rows.map((r) => r.name.length), 4);
  for (const r of rows) {
    lines.push(r.name.padEnd(width) + ' | ' + r.type.padEnd(6) + ' | ' +
      (r.size || '').padStart(10) + ' | ' + r.mtime);
  }
  return lines.join('\n');
}

// ---------------------------------------------------------------------------
// Инструмент 5: get_disks
// ---------------------------------------------------------------------------

async function toolGetDisks() {
  const ps =
    "Get-PSDrive -PSProvider FileSystem | " +
    "Select-Object Name, @{n='Root';e={$_.Root}}, @{n='UsedGB';e={ if ($_.Used) { [math]::Round($_.Used/1GB,2) } }}, @{n='FreeGB';e={ if ($_.Free) { [math]::Round($_.Free/1GB,2) } }} | " +
    "ConvertTo-Json -Compress";
  const output = await toolRunCommand({ command: ps, timeoutSeconds: 30 });
  // Вытаскиваем только JSON между маркерами stdout.
  const start = output.indexOf('--- stdout ---');
  const json = start >= 0 ? output.slice(start + 13).split('--- ')[0].trim() : output;
  return '=== диски (Name, Root, UsedGB, FreeGB) ===\n' + json;
}

// ---------------------------------------------------------------------------
// Реестр инструментов (описания видит ИИ-клиент)
// ---------------------------------------------------------------------------

const TOOLS = [
  {
    name: 'run_command',
    description:
      'Выполнить команду PowerShell на компьютере пользователя и вернуть stdout, ' +
      'stderr и код выхода. Основной инструмент: установка программ, диагностика, ' +
      'работа с файлами — всё через него.',
    inputSchema: {
      type: 'object',
      properties: {
        command: { type: 'string', description: 'Команда PowerShell, например Get-Date' },
        timeoutSeconds: {
          type: 'number',
          description: 'Лимит выполнения в секундах (по умолчанию 60, максимум 600)',
        },
        workingDir: { type: 'string', description: 'Рабочая папка (необязательно)' },
      },
      required: ['command'],
    },
    handler: toolRunCommand,
  },
  {
    name: 'read_file',
    description: 'Прочитать текстовый файл (UTF-8) и вернуть его содержимое.',
    inputSchema: {
      type: 'object',
      properties: {
        path: { type: 'string', description: 'Абсолютный путь к файлу' },
        maxBytes: { type: 'number', description: 'Сколько байт читать (по умолчанию 262144)' },
      },
      required: ['path'],
    },
    handler: toolReadFile,
  },
  {
    name: 'write_file',
    description:
      'Записать текстовый файл (UTF-8). Недостающие папки создаются автоматически.',
    inputSchema: {
      type: 'object',
      properties: {
        path: { type: 'string', description: 'Абсолютный путь к файлу' },
        content: { type: 'string', description: 'Содержимое файла' },
      },
      required: ['path', 'content'],
    },
    handler: toolWriteFile,
  },
  {
    name: 'list_dir',
    description: 'Показать содержимое папки: имя, тип, размер, дата изменения.',
    inputSchema: {
      type: 'object',
      properties: {
        path: { type: 'string', description: 'Абсолютный путь к папке' },
      },
      required: ['path'],
    },
    handler: toolListDir,
  },
  {
    name: 'get_disks',
    description: 'Показать диски компьютера: имя, корень, занято и свободно (ГБ).',
    inputSchema: { type: 'object', properties: {} },
    handler: toolGetDisks,
  },
];

const TOOLS_BY_NAME = new Map(TOOLS.map((t) => [t.name, t]));

// ---------------------------------------------------------------------------
// Протокол MCP поверх stdio (JSON-RPC 2.0, по одному сообщению на строку)
// ---------------------------------------------------------------------------

function send(message) {
  try {
    process.stdout.write(JSON.stringify(message) + '\n');
  } catch (e) {
    log('не удалось записать ответ:', e.message);
  }
}

function rpcResult(id, result) {
  return { jsonrpc: '2.0', id, result };
}

function rpcError(id, code, message) {
  return { jsonrpc: '2.0', id, error: { code, message } };
}

async function handleMessage(msg) {
  // Уведомления (без id) не требуют ответа.
  if (msg && msg.method !== undefined && msg.id === undefined) {
    if (msg.method === 'notifications/initialized') log('клиент инициализирован');
    return null;
  }
  if (!msg || typeof msg.method !== 'string' || msg.id === undefined) return null;

  const id = msg.id;
  const method = msg.method;
  const params = msg.params || {};

  if (method === 'initialize') {
    // Отвечаем версией клиента, если она известна, иначе своей.
    const clientVersion = typeof params.protocolVersion === 'string'
      ? params.protocolVersion : PROTOCOL_VERSION;
    return rpcResult(id, {
      protocolVersion: clientVersion,
      capabilities: {
        tools: { listChanged: false },
        resources: {},
        prompts: {},
      },
      serverInfo: { name: SERVER_NAME, version: SERVER_VERSION },
      instructions:
        'Швейцарский нож McpGo: инструменты для работы на компьютере пользователя — ' +
        'run_command (PowerShell), read_file, write_file, list_dir, get_disks. ' +
        'Действуйте аккуратно: команды выполняются с правами пользователя.',
    });
  }

  if (method === 'ping') return rpcResult(id, {});

  if (method === 'tools/list') {
    return rpcResult(id, {
      tools: TOOLS.map((t) => ({
        name: t.name,
        description: t.description,
        inputSchema: t.inputSchema,
      })),
    });
  }

  if (method === 'tools/call') {
    const tool = TOOLS_BY_NAME.get(params.name);
    if (!tool) {
      return rpcResult(id, {
        content: [{ type: 'text', text: 'неизвестный инструмент: ' + params.name }],
        isError: true,
      });
    }
    try {
      const text = await tool.handler(params.arguments || {});
      return rpcResult(id, { content: [{ type: 'text', text }], isError: false });
    } catch (e) {
      return rpcResult(id, {
        content: [{ type: 'text', text: 'Ошибка: ' + e.message }],
        isError: true,
      });
    }
  }

  if (method === 'resources/list') return rpcResult(id, { resources: [] });
  if (method === 'prompts/list') return rpcResult(id, { prompts: [] });

  return rpcError(id, -32601, 'метод не найден: ' + method);
}

function serve() {
  log(SERVER_NAME + ' v' + SERVER_VERSION + ' запущен (транспорт: stdio)');
  const rl = readline.createInterface({ input: process.stdin });
  rl.on('line', (line) => {
    const trimmed = line.trim();
    if (!trimmed) return;
    let msg;
    try {
      msg = JSON.parse(trimmed);
    } catch (_) {
      send(rpcError(null, -32700, 'ошибка разбора JSON'));
      return;
    }
    // Поддержка батчей (массив сообщений) из спецификации 2025-06-18.
    const batch = Array.isArray(msg) ? msg : [msg];
    Promise.all(batch.map(handleMessage)).then((replies) => {
      const filtered = replies.filter((r) => r !== null);
      if (filtered.length === 0) return;
      send(Array.isArray(msg) ? filtered : filtered[0]);
    }).catch((e) => {
      log('внутренняя ошибка обработки:', e.message);
    });
  });
  rl.on('close', () => {
    log('stdin закрыт — завершаюсь');
    process.exit(0);
  });
}

// ---------------------------------------------------------------------------
// Режим самопроверки: node server.js --selftest
// ---------------------------------------------------------------------------

async function selftest() {
  let failures = 0;
  const check = (label, ok, detail) => {
    console.log((ok ? 'PASS' : 'FAIL') + '  ' + label + (ok ? '' : '  -- ' + detail));
    if (!ok) failures += 1;
  };

  // 1. get_disks (на Windows пойдёт через PowerShell; на другой ОС —
  // честный FAIL, потому что этот сервер сделан для Windows).
  try {
    const disks = await toolGetDisks();
    check('get_disks', disks.includes('UsedGB'), disks.slice(0, 120));
  } catch (e) { check('get_disks', false, e.message); }

  // 2. run_command
  try {
    const out = await toolRunCommand({ command: 'Get-Date' });
    check('run_command (Get-Date)', out.includes('=== код выхода: 0 ==='), out.slice(0, 120));
  } catch (e) { check('run_command', false, e.message); }

  // 3. write_file + read_file + list_dir
  const testFile = path.join(os.tmpdir(), 'mcpgo-selftest-' + Date.now() + '.txt');
  try {
    const marker = 'проверка швейцарского ножа ' + new Date().toISOString();
    const w = await toolWriteFile({ path: testFile, content: marker });
    const r = await toolReadFile({ path: testFile });
    const dir = await toolListDir({ path: os.tmpdir() });
    check('write_file', w.includes('записано'), w);
    check('read_file', r.includes(marker), r.slice(0, 120));
    check('list_dir', dir.includes('записей'), dir.slice(0, 120));
    fs.unlinkSync(testFile);
  } catch (e) {
    check('write/read/list', false, e.message);
    try { fs.unlinkSync(testFile); } catch (_) { /* уже нет */ }
  }

  console.log('');
  if (failures === 0) {
    console.log('SELFTEST OK — все 5 инструментов работают.');
    process.exit(0);
  } else {
    console.log('SELFTEST: ошибок — ' + failures + '. Пришли этот текст в чат.');
    process.exit(1);
  }
}

// ---------------------------------------------------------------------------

if (process.argv.includes('--selftest')) {
  selftest().catch((e) => { console.error('самопроверка упала:', e); process.exit(1); });
} else {
  serve();
}
