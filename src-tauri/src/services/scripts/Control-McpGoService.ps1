<#
.SYNOPSIS
    Starts, stops or restarts one McpGo Windows service.

.DESCRIPTION
    Part of the McpGo for Windows port of the macOS LaunchAgents domain:
    the equivalent of "launchctl load/unload" for one agent.

    Actions:
    * Start   - sc.exe start. 1056 (ERROR_SERVICE_ALREADY_RUNNING) is
      reported as success with a note instead of an error.
    * Stop    - sc.exe stop, then a bounded wait (30 s) for the Stopped
      state: the service worker terminates its runtime process on the SCM
      stop request. A timeout is reported as a FAILURE - force-killing
      is deliberately left to Uninstall-McpGoService.ps1.
    * Restart - Stop (with the same wait), then Start.

    Status polling uses Get-Service: its ServiceControllerStatus names are
    culture-invariant, unlike the localized sc.exe query output.

    Service control requires an administrator token, so the script
    self-elevates through a UAC prompt (Start-Process -Verb RunAs) when
    started with normal user rights. The elevated instance performs the
    work and writes its JSON result to -OutFile; the original instance
    prints that file so the Rust caller (installer.rs) keeps a single
    stdout result channel.

.NOTES
    Single-file deliverable by design: the elevation and result-reporting
    blocks are intentionally duplicated across Install / Uninstall /
    Control-McpGoService.ps1 so every script stays self-contained.

.PARAMETER Id
    Service definition id (lowercase letters, digits, inner hyphens).
    The SCM service name is "McpGo-<id>".

.PARAMETER Action
    One of: Start, Stop, Restart.

.PARAMETER OutFile
    Internal: file the elevated instance writes its JSON result to.

.PARAMETER Elevated
    Internal: marks the already-elevated instance (prevents an
    elevation loop when something is misconfigured).

.OUTPUTS
    One JSON line on stdout:
    {"ok":true|false,"operation":"start|stop|restart",...,"status":"..."}
    - parsed by src-tauri/src/services/installer.rs.

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File .\Control-McpGoService.ps1 -Id weather-mcp -Action Restart
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[a-z0-9][a-z0-9-]{0,39}$')]
    [string]$Id,

    [Parameter(Mandatory = $true)]
    [ValidateSet('Start', 'Stop', 'Restart')]
    [string]$Action,

    [string]$OutFile = '',

    [switch]$Elevated
)

$ErrorActionPreference = 'Stop'

# Stop budget shared by the Stop and Restart actions.
$STOP_TIMEOUT_SECONDS = 30

# --- shared helpers ----------------------------------------------------------

function Test-McpGoAdmin {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($identity)
    $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Write-McpGoResult {
    param([bool]$Ok, [string]$Detail = '')
    $service = Get-Service -Name "McpGo-$Id" -ErrorAction SilentlyContinue
    $result = [ordered]@{
        ok          = $Ok
        operation   = $Action.ToLowerInvariant()
        id          = $Id
        serviceName = "McpGo-$Id"
        status      = if ($null -ne $service) { [string]$service.Status } else { $null }
        detail      = $Detail
    }
    $json = ($result | ConvertTo-Json -Compress)
    if ($OutFile -ne '') {
        Set-Content -LiteralPath $OutFile -Value $json -Encoding UTF8
    }
    [Console]::Out.WriteLine($json)
}

function Wait-McpGoServiceStopped {
    # Polls the culture-invariant status name until the service reports
    # Stopped or the budget is exhausted. Returns $true when stopped.
    param([string]$Name, [int]$TimeoutSeconds)
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ((Get-Date) -lt $deadline) {
        $service = Get-Service -Name $Name -ErrorAction SilentlyContinue
        if ($null -eq $service -or $service.Status -eq 'Stopped') {
            return $true
        }
        Start-Sleep -Milliseconds 500
    }
    return $false
}

function Invoke-McpGoStart {
    $output = sc.exe start "McpGo-$Id" 2>&1
    if ($LASTEXITCODE -eq 1056) {
        # ERROR_SERVICE_ALREADY_RUNNING - the desired state is reached.
        Write-McpGoResult -Ok $true -Detail 'already running'
        exit 0
    }
    if ($LASTEXITCODE -ne 0) {
        Write-McpGoResult -Ok $false -Detail ("sc.exe start failed: " + ($output | Out-String).Trim())
        exit 1
    }
    Write-McpGoResult -Ok $true -Detail 'start requested'
    exit 0
}

function Invoke-McpGoStop {
    $output = sc.exe stop "McpGo-$Id" 2>&1
    if ($LASTEXITCODE -ne 0 -and $LASTEXITCODE -ne 1062) {
        Write-McpGoResult -Ok $false -Detail ("sc.exe stop failed: " + ($output | Out-String).Trim())
        exit 1
    }
    if ($LASTEXITCODE -eq 1062) {
        # ERROR_SERVICE_NOT_ACTIVE - the service was already stopped.
        Write-McpGoResult -Ok $true -Detail 'already stopped'
        exit 0
    }
    if (Wait-McpGoServiceStopped -Name "McpGo-$Id" -TimeoutSeconds $STOP_TIMEOUT_SECONDS) {
        Write-McpGoResult -Ok $true -Detail 'stopped'
        exit 0
    }
    Write-McpGoResult -Ok $false -Detail ("the service did not reach the Stopped state within $STOP_TIMEOUT_SECONDS s")
    exit 1
}

# --- self-elevation ----------------------------------------------------------

if (-not (Test-McpGoAdmin)) {
    if ($Elevated) {
        Write-McpGoResult -Ok $false -Detail 'the elevated instance still does not hold an administrator token'
        exit 1
    }

    $resultFile = [System.IO.Path]::GetTempFileName()
    $q = [char]34
    $relay = @(
        '-NoProfile', '-ExecutionPolicy', 'Bypass',
        '-File', "$q$PSCommandPath$q",
        '-Id', "$q$Id$q",
        '-Action', $Action,
        '-Elevated',
        '-OutFile', "$q$resultFile$q"
    ) -join ' '

    try {
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
    if ($null -eq (Get-Service -Name "McpGo-$Id" -ErrorAction SilentlyContinue)) {
        Write-McpGoResult -Ok $false -Detail "service McpGo-$Id is not installed"
        exit 1
    }

    switch ($Action) {
        'Start' { Invoke-McpGoStart }
        'Stop' { Invoke-McpGoStop }
        'Restart' {
            # Stop (tolerating "already stopped"), then start.
            $output = sc.exe stop "McpGo-$Id" 2>&1
            if ($LASTEXITCODE -ne 0 -and $LASTEXITCODE -ne 1062) {
                Write-McpGoResult -Ok $false -Detail ("sc.exe stop failed: " + ($output | Out-String).Trim())
                exit 1
            }
            if ($LASTEXITCODE -ne 1062 -and
                -not (Wait-McpGoServiceStopped -Name "McpGo-$Id" -TimeoutSeconds $STOP_TIMEOUT_SECONDS)) {
                Write-McpGoResult -Ok $false -Detail ("the service did not reach the Stopped state within $STOP_TIMEOUT_SECONDS s")
                exit 1
            }
            Invoke-McpGoStart
        }
    }
    # Unreachable: every switch branch exits by itself.
    exit 1
}
catch {
    Write-McpGoResult -Ok $false -Detail $_.Exception.Message
    exit 1
}
