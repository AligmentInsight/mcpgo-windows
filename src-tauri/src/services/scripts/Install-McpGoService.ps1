<#
.SYNOPSIS
    Registers one McpGo service definition as a Windows service (SCM).

.DESCRIPTION
    Part of the McpGo for Windows port of the macOS LaunchAgents domain.
    Creates the SCM service "McpGo-<id>" whose ImagePath starts the McpGo
    application executable in service-worker mode:

        "<McpGo.exe>" --service-host <id> --data-dir "<data dir>"

    The service worker (src-tauri/src/services/worker.rs), not this script,
    supervises the actual runtime process (node / python / powershell):
    plain runtimes are not SCM-aware, so they cannot be registered with the
    SCM directly.

    Tool choice on purpose:
    * New-Service creates the service - PowerShell-native parameter
      binding passes the ImagePath verbatim, avoiding the well-known
      PowerShell 5.1 native-argument quoting bug that corrupts sc.exe
      command lines with embedded quotes;
    * sc.exe configures failure recovery - the only tool exposing it -
      and stays the deletion/start/stop workhorse everywhere else
      (Windows PowerShell 5.1 has no Remove-Service; it is PowerShell 6+).

    Service creation requires an administrator token, so the script
    self-elevates through a UAC prompt (Start-Process -Verb RunAs) when
    started with normal user rights. The elevated instance performs the
    work and writes its JSON result to -OutFile; the original instance
    prints that file so the Rust caller (installer.rs) keeps a single
    stdout result channel with a stable contract.

.NOTES
    Single-file deliverable by design: the elevation and result-reporting
    blocks are intentionally duplicated across Install / Uninstall /
    Control-McpGoService.ps1 so every script stays self-contained.

.PARAMETER Id
    Service definition id: lowercase letters, digits and inner hyphens
    (the guard mirrors the authoritative validation in definition.rs).
    The SCM service name is "McpGo-<id>".

.PARAMETER WorkerPath
    Absolute path of the McpGo executable (the service ImagePath target).

.PARAMETER DataDir
    Absolute McpGo data directory passed to the worker via --data-dir.
    Services run as LocalSystem whose %APPDATA% is NOT the installing
    user's - the explicit flag keeps the definition files findable.

.PARAMETER DisplayName
    Friendly name shown in services.msc (defaults to "McpGo <id>").
    Double quotes are replaced with apostrophes: they cannot travel
    through the service configuration APIs safely.

.PARAMETER Description
    Service description shown in services.msc (optional).

.PARAMETER Autostart
    'true' -> start type Automatic (at system boot), otherwise Manual.
    Accepted as a string ('true'/'false') because PowerShell boolean
    parameter binding treats the non-empty string 'false' as $true.

.PARAMETER KeepAlive
    'true' -> configure SCM failure recovery (restart after 5 s, three
    attempts, reset daily) so a crashed worker is restarted by Windows
    itself. The worker separately restarts the runtime process it
    supervises; this switch covers the worker's own crashes.

.PARAMETER OutFile
    Internal: file the elevated instance writes its JSON result to.

.PARAMETER Elevated
    Internal: marks the already-elevated instance (prevents an
    elevation loop when something is misconfigured).

.OUTPUTS
    One JSON line on stdout: {"ok":true|false,"operation":"install",...}
    - parsed by src-tauri/src/services/installer.rs.

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File .\Install-McpGoService.ps1 `
        -Id weather-mcp -WorkerPath "C:\Program Files\McpGo\McpGo.exe" `
        -DataDir "C:\Users\dev\AppData\Roaming\com.mcpgo.windows" `
        -DisplayName "Weather MCP" -Autostart true -KeepAlive true
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[a-z0-9][a-z0-9-]{0,39}$')]
    [string]$Id,

    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$WorkerPath,

    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$DataDir,

    [string]$DisplayName = '',

    [string]$Description = '',

    [string]$Autostart = 'false',

    [string]$KeepAlive = 'false',

    [string]$OutFile = '',

    [switch]$Elevated
)

$ErrorActionPreference = 'Stop'

# --- shared helpers ----------------------------------------------------------

function Test-McpGoAdmin {
    # IsInRole honours group membership and the elevation state of the
    # current token - more reliable than comparing user names.
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($identity)
    $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function ConvertTo-McpGoFlag {
    # Explicit string->bool: PowerShell would coerce ANY non-empty string
    # (including 'false') to $true when binding a [bool] parameter.
    param([string]$Value)
    @('true', '1', 'yes') -contains $Value.ToLowerInvariant()
}

function Write-McpGoResult {
    # Single reporting channel: one JSON line on stdout, mirrored into
    # -OutFile when this instance runs elevated. ConvertTo-Json -Compress
    # guarantees the one-line contract for the Rust-side parser.
    param([bool]$Ok, [string]$Detail = '')
    $result = [ordered]@{
        ok          = $Ok
        operation   = 'install'
        id          = $Id
        serviceName = "McpGo-$Id"
        detail      = $Detail
    }
    $json = ($result | ConvertTo-Json -Compress)
    if ($OutFile -ne '') {
        Set-Content -LiteralPath $OutFile -Value $json -Encoding UTF8
    }
    [Console]::Out.WriteLine($json)
}

# --- self-elevation ----------------------------------------------------------

if (-not (Test-McpGoAdmin)) {
    if ($Elevated) {
        # Defensive: the elevated relaunch must run with the administrator
        # token. Failing here beats looping forever.
        Write-McpGoResult -Ok $false -Detail 'the elevated instance still does not hold an administrator token'
        exit 1
    }

    $resultFile = [System.IO.Path]::GetTempFileName()
    $q = [char]34
    $relay = @(
        '-NoProfile', '-ExecutionPolicy', 'Bypass',
        '-File', "$q$PSCommandPath$q",
        '-Id', "$q$Id$q",
        '-WorkerPath', "$q$WorkerPath$q",
        '-DataDir', "$q$DataDir$q",
        '-DisplayName', "$q$DisplayName$q",
        '-Description', "$q$Description$q",
        '-Autostart', $Autostart,
        '-KeepAlive', $KeepAlive,
        '-Elevated',
        '-OutFile', "$q$resultFile$q"
    ) -join ' '

    try {
        # -Verb RunAs shows the UAC consent prompt; -Wait blocks until the
        # elevated instance finishes. A declined prompt throws here.
        Start-Process -FilePath 'powershell.exe' -ArgumentList $relay `
            -Verb RunAs -Wait -WindowStyle Hidden | Out-Null
    }
    catch {
        Write-McpGoResult -Ok $false -Detail ("UAC elevation declined or failed: " + $_.Exception.Message)
        Remove-Item -LiteralPath $resultFile -ErrorAction SilentlyContinue
        exit 1
    }

    if (Test-Path -LiteralPath $resultFile) {
        $payloadText = Get-Content -LiteralPath $resultFile -Raw
        # Relay the elevated instance's result verbatim - the Rust caller
        # parses exactly one JSON document from stdout.
        [Console]::Out.Write($payloadText)
        Remove-Item -LiteralPath $resultFile -ErrorAction SilentlyContinue
        try {
            $payload = $payloadText | ConvertFrom-Json
            if ($payload.ok) { exit 0 } else { exit 1 }
        }
        catch {
            exit 1
        }
    }
    Write-McpGoResult -Ok $false -Detail 'the elevated instance produced no result'
    exit 1
}

# --- work (elevated path) ----------------------------------------------------

try {
    if (-not (Test-Path -LiteralPath $WorkerPath -PathType Leaf)) {
        throw "the McpGo executable was not found at `"$WorkerPath`""
    }
    if (-not (Test-Path -LiteralPath $DataDir -PathType Container)) {
        throw "the McpGo data directory was not found at `"$DataDir`""
    }

    $serviceName = "McpGo-$Id"
    if (Get-Service -Name $serviceName -ErrorAction SilentlyContinue) {
        throw "a service named `"$serviceName`" is already installed"
    }

    # Display text: drop double quotes - the service configuration APIs
    # cannot carry them safely. Apostrophes read fine in services.msc.
    $safeDisplay = if ($DisplayName -ne '') { $DisplayName.Replace('"', "'") } else { "McpGo $Id" }
    $safeDescription = $Description.Replace('"', "'")
    $auto = ConvertTo-McpGoFlag -Value $Autostart
    $keepAlive = ConvertTo-McpGoFlag -Value $KeepAlive
    $startType = if ($auto) { 'Automatic' } else { 'Manual' }

    # ImagePath stored verbatim by New-Service: the quoted executable,
    # the worker flags and the quoted data directory.
    $binPath = '"' + $WorkerPath + '"' + " --service-host $Id --data-dir " + '"' + $DataDir + '"'

    New-Service -Name $serviceName -BinaryPathName $binPath `
        -DisplayName $safeDisplay -Description $safeDescription `
        -StartupType $startType -ErrorAction Stop | Out-Null

    if ($keepAlive) {
        # Restart the worker itself after crashes (three quick retries,
        # counter reset after a clean day). Quote-free argument list, so
        # sc.exe is safe to call from PowerShell here.
        $failureOutput = sc.exe failure $serviceName reset= 86400 actions= restart/5000/restart/5000/restart/5000 2>&1
        if ($LASTEXITCODE -ne 0) {
            throw ("sc.exe failure failed: " + ($failureOutput | Out-String).Trim())
        }
    }

    Write-McpGoResult -Ok $true -Detail ("installed as $serviceName (start type: $startType, keep-alive: $keepAlive)")
    exit 0
}
catch {
    Write-McpGoResult -Ok $false -Detail $_.Exception.Message
    exit 1
}
