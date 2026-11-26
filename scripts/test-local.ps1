[CmdletBinding()]
param(
    [string]$Executable,
    [string]$Websocat = "websocat",
    [ValidateRange(1, 300)]
    [int]$StartupTimeoutSeconds = 30
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot

function Test-Administrator {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = [Security.Principal.WindowsPrincipal]::new($identity)
    return $principal.IsInRole(
        [Security.Principal.WindowsBuiltInRole]::Administrator
    )
}

function Resolve-MultimeterExecutable {
    param([string]$Path)

    if ($Path) {
        $resolved = Resolve-Path -LiteralPath $Path -ErrorAction Stop
        return $resolved.Path
    }

    $candidates = @(
        (Join-Path $repoRoot "target\release\multimeter.exe"),
        (Join-Path $repoRoot "target\debug\multimeter.exe"),
        (Join-Path $repoRoot "target\x86_64-pc-windows-msvc\release\multimeter.exe"),
        (Join-Path $repoRoot "target\aarch64-pc-windows-msvc\release\multimeter.exe"),
        (Join-Path $repoRoot "target\x86_64-pc-windows-gnu\release\multimeter.exe")
    )

    foreach ($candidate in $candidates) {
        if (Test-Path -LiteralPath $candidate -PathType Leaf) {
            return $candidate
        }
    }

    throw "No built multimeter executable found. Pass -Executable PATH."
}

function Resolve-WebsocatExecutable {
    param([string]$Path)

    if ($Path.Contains("\") -or $Path.Contains("/")) {
        $resolved = Resolve-Path -LiteralPath $Path -ErrorAction Stop
        return $resolved.Path
    }

    $command = Get-Command -Name $Path -CommandType Application -ErrorAction Stop |
        Select-Object -First 1
    return $command.Source
}

function ConvertTo-PowerShellLiteral {
    param([string]$Value)

    return "'$($Value.Replace("'", "''"))'"
}

function Get-RedactedLog {
    param([string]$Path)

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        return ""
    }

    return [IO.File]::ReadAllText($Path) -replace (
        "(?m)^launch URL: .+$",
        "launch URL: <redacted>"
    )
}

$resolvedExecutable = Resolve-MultimeterExecutable -Path $Executable
$resolvedWebsocat = Resolve-WebsocatExecutable -Path $Websocat

if (-not (Test-Administrator)) {
    Write-Host "Requesting administrator privileges..."

    $invocation = (
        "& $(ConvertTo-PowerShellLiteral $PSCommandPath) " +
        "-Executable $(ConvertTo-PowerShellLiteral $resolvedExecutable) " +
        "-Websocat $(ConvertTo-PowerShellLiteral $resolvedWebsocat) " +
        "-StartupTimeoutSeconds $StartupTimeoutSeconds"
    )
    $encodedCommand = [Convert]::ToBase64String(
        [Text.Encoding]::Unicode.GetBytes($invocation)
    )
    $hostExecutable = Join-Path $PSHOME "pwsh.exe"
    if (-not (Test-Path -LiteralPath $hostExecutable -PathType Leaf)) {
        $hostExecutable = Join-Path $PSHOME "powershell.exe"
    }

    Start-Process `
        -FilePath $hostExecutable `
        -Verb RunAs `
        -ArgumentList @(
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-EncodedCommand",
            $encodedCommand
        ) |
        Out-Null
    exit 0
}

$tempRoot = Join-Path (
    [IO.Path]::GetTempPath()
) ("multimeter-local-test-{0}" -f [Guid]::NewGuid().ToString("N"))
$stdoutPath = Join-Path $tempRoot "multimeter.stdout.log"
$stderrPath = Join-Path $tempRoot "multimeter.stderr.log"
$process = $null
$websocatExitCode = 0

try {
    New-Item -ItemType Directory -Path $tempRoot | Out-Null

    Write-Host "Launching elevated multimeter: $resolvedExecutable"
    $process = Start-Process `
        -FilePath $resolvedExecutable `
        -ArgumentList @("--no-browser") `
        -NoNewWindow `
        -PassThru `
        -RedirectStandardOutput $stdoutPath `
        -RedirectStandardError $stderrPath

    $deadline = [DateTime]::UtcNow.AddSeconds($StartupTimeoutSeconds)
    $launchUrl = $null

    while ([DateTime]::UtcNow -lt $deadline) {
        if ($process.HasExited) {
            $stdout = Get-RedactedLog -Path $stdoutPath
            $stderr = Get-RedactedLog -Path $stderrPath
            throw "multimeter exited before publishing launch information.`n$stdout`n$stderr"
        }

        if (Test-Path -LiteralPath $stdoutPath -PathType Leaf) {
            $stdout = [IO.File]::ReadAllText($stdoutPath)
            $match = [regex]::Match(
                $stdout,
                "(?m)^launch URL: (?<url>[^\r\n]+)\r?$"
            )
            if ($match.Success) {
                $launchUrl = $match.Groups["url"].Value.Trim()
                break
            }
        }

        Start-Sleep -Milliseconds 100
    }

    if (-not $launchUrl) {
        $stdout = Get-RedactedLog -Path $stdoutPath
        $stderr = Get-RedactedLog -Path $stderrPath
        throw (
            "multimeter did not publish launch information within " +
            "$StartupTimeoutSeconds seconds.`n$stdout`n$stderr"
        )
    }

    $payloadMatch = [regex]::Match(
        $launchUrl,
        "#multimeter=(?<payload>[A-Za-z0-9_-]+)$"
    )
    if (-not $payloadMatch.Success) {
        throw "Launch URL does not contain a multimeter bootstrap fragment."
    }

    $encoded = $payloadMatch.Groups["payload"].Value
    $padding = switch ($encoded.Length % 4) {
        0 { "" }
        2 { "==" }
        3 { "=" }
        default {
            throw "Launch URL contains an invalid bootstrap fragment length."
        }
    }
    $base64 = $encoded.Replace("-", "+").Replace("_", "/") + $padding
    $bootstrapJson = [Text.Encoding]::UTF8.GetString(
        [Convert]::FromBase64String($base64)
    )
    $bootstrap = $bootstrapJson | ConvertFrom-Json

    if ($bootstrap.v -ne 1) {
        throw "Unsupported multimeter bootstrap protocol version."
    }
    if (-not ("$($bootstrap.ws)" -match "^wss?://")) {
        throw "Bootstrap data does not contain a valid WebSocket URL."
    }
    if (-not ("$($bootstrap.token)" -match "^[A-Za-z0-9_-]+$")) {
        throw "Bootstrap data does not contain a valid authentication token."
    }

    $websocketUrl = [string]$bootstrap.ws
    $token = [string]$bootstrap.token
    $authPayload = [ordered]@{
        type = "authenticate"
        token = $token
    } | ConvertTo-Json -Compress

    Write-Host ""
    Write-Host "WebSocket URL: $websocketUrl"
    Write-Host "Authentication token extracted ($($token.Length) characters)."
    Write-Host "Starting websocat; the authenticate message is sent automatically."
    Write-Host "Type one JSON command per line. Exit websocat to stop multimeter."
    Write-Host ""

    if (Get-Variable -Name PSNativeCommandArgumentPassing -ErrorAction SilentlyContinue) {
        $PSNativeCommandArgumentPassing = "Standard"
    } else {
        # Windows PowerShell needs embedded quotes escaped for native programs.
        $authPayload = $authPayload.Replace('"', '\"')
    }
    & $resolvedWebsocat -t "--preamble=$authPayload" $websocketUrl
    $websocatExitCode = $LASTEXITCODE
} finally {
    if ($null -ne $process) {
        if (-not $process.HasExited) {
            Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
            [void]$process.WaitForExit(5000)
        }
    }

    if (Test-Path -LiteralPath $tempRoot -PathType Container) {
        Remove-Item -LiteralPath $tempRoot -Recurse -Force
    }
}

exit $websocatExitCode
