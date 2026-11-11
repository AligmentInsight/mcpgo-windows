<#
.SYNOPSIS
    Removes one McpGo Windows service from the Service Control Manager.

.DESCRIPTION
    Part of the McpGo for Windows port of the macOS LaunchAgents domain.
    Deletes the SCM service "McpGo-<id>" (the equivalent of unloading and
    deleting a LaunchAgent plist on macOS). The definition file itself is
    NOT touched - it stays available for a later reinstall.

    Steps (elevated path):
    1. sc.exe stop          - requests a stop; 1062 ("not active") is fine.
    2. wait for Stopped     - polls Get-Service for up to 30 s. The service
       worker terminates its runtime process on the SCM stop request;
       Get-Service yields the culture-invariant ServiceControllerStatus
       names, unlike the localized sc.exe query output.
    3. belt-and-braces      - a worker stuck in StopPending gets its whole
       process tree force-killed (taskkill /T /F) so the deletion cannot
       hang on a wedged runtime.
    4. sc.exe delete        - Windows PowerShell 5.1 has no Remove-Service
       (PowerShell 6+ only), so sc.exe is the universal deletion tool.

    Idempotent: uninstalling a service that is not installed reports
    success ("not installed") instead of an error, so the UI and retry
    logic stay simple.

    Service deletion requires an administrator token, so the script
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

.PARAMETER OutFile
    Internal: file the elevated instance writes its JSON result to.

.PARAMETER Elevated
    Internal: marks the already-elevated instance (prevents an
    elevation loop when something is misconfigured).

.OUTPUTS
    One JSON line on stdout: {"ok":true|false,"operation":"uninstall",...}
    - parsed by src-tauri/src/services/installer.rs.

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File .\Uninstall-McpGoService.ps1 -Id weather-mcp
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[a-z0-9][a-z0-9-]{0,39}$')]
    [string]$Id,

    [string]$OutFile = '',

    [switch]$Elevated
)

$ErrorActionPreference = 'Stop'

# Generous stop budget: the worker asks its runtime process to terminate
# and Windows needs a moment to retire the process handles.
$STOP_TIMEOUT_SECONDS = 30

# --- shared helpers ----------------------------------------------------------

function Test-McpGoAdmin {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($identity)
    $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Write-McpGoResult {
    param([bool]$Ok, [string]$Detail = '')
    $result = [ordered]@{
        ok          = $Ok
        operation   = 'uninstall'
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
    $serviceName = "McpGo-$Id"
    $existing = Get-Service -Name $serviceName -ErrorAction SilentlyContinue
    if ($null -eq $existing) {
        # Idempotent success: nothing to remove.
        Write-McpGoResult -Ok $true -Detail "service $serviceName is not installed"
        exit 0
    }

    # 1. Request the stop. 1062 (ERROR_SERVICE_NOT_ACTIVE) simply means
    #    the service was already stopped.
    $null = sc.exe stop $serviceName
    if ($LASTEXITCODE -ne 0 -and $LASTEXITCODE -ne 1062) {
        throw "sc.exe stop failed with exit code $LASTEXITCODE"
    }

    # 2. Give the worker its stop budget.
    $stopped = Wait-McpGoServiceStopped -Name $serviceName -TimeoutSeconds $STOP_TIMEOUT_SECONDS

    # 3. Force-kill a stuck worker tree so the delete cannot hang.
    if (-not $stopped) {
        $instance = Get-CimInstance -ClassName Win32_Service -Filter "Name='$serviceName'"
        if ($null -ne $instance -and $instance.ProcessId -gt 0) {
            $null = taskkill /PID $instance.ProcessId /T /F
        }
        $stopped = Wait-McpGoServiceStopped -Name $serviceName -TimeoutSeconds 5
    }

    # 4. Delete the service. sc.exe is used because Windows PowerShell
    #    5.1 has no Remove-Service cmdlet.
    $deleteOutput = sc.exe delete $serviceName 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw ("sc.exe delete failed: " + ($deleteOutput | Out-String).Trim())
    }

    if ($stopped) {
        Write-McpGoResult -Ok $true -Detail ("removed service $serviceName")
    }
    else {
        # Deleted while still winding down - the SCM retires the name as
        # soon as the last handle closes. Reported as success with a note.
        Write-McpGoResult -Ok $true -Detail ("removed service $serviceName (it was still winding down)")
    }
    exit 0
}
catch {
    Write-McpGoResult -Ok $false -Detail $_.Exception.Message
    exit 1
}
