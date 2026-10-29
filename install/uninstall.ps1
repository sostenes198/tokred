<#
.SYNOPSIS
  tokred uninstaller for Windows.

.DESCRIPTION
  irm https://raw.githubusercontent.com/sostenes198/tokred/main/install/uninstall.ps1 | iex

  Runs `tokred uninstall` (which stops the proxy, removes its scheduled task and takes the hooks
  and agents out of Claude Code), then - once that process has exited, because a running exe
  cannot delete itself - deletes tokred.exe, the tokred folder when it is empty, the entry in
  your user PATH and the `claude` wrapper in your PowerShell profiles.

  Options (environment variable for `irm | iex`):
    -Yes     TOKRED_YES=1    do not ask anything (keeps tokred's data folder unless -Purge)
    -Purge   TOKRED_PURGE=1  also delete tokred's data folder (database, config, logs, reports)
  TOKRED_HOME (default %LOCALAPPDATA%\tokred) is where the install script put the binary.
#>
param(
    [switch]$Yes,
    [switch]$Purge
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$BlockBegin = '# >>> tokred >>>'
$BlockEnd = '# <<< tokred <<<'

function Test-On([string]$Value) {
    return [bool]($Value -and $Value -ne '0')
}

function Write-Step([string]$Message) {
    Write-Host "==> $Message"
}

if (Test-On $env:TOKRED_YES) { $Yes = $true }
if (Test-On $env:TOKRED_PURGE) { $Purge = $true }

if ($env:TOKRED_HOME) {
    $tokredHome = $env:TOKRED_HOME
}
else {
    if (-not $env:LOCALAPPDATA) { throw 'LOCALAPPDATA is not set' }
    $tokredHome = Join-Path $env:LOCALAPPDATA 'tokred'
}
$binDir = Join-Path $tokredHome 'bin'
$exe = Join-Path $binDir 'tokred.exe'

function Remove-ProfileBlock([string]$Text) {
    $pattern = '(?s)' + [regex]::Escape($BlockBegin) + '.*?' + [regex]::Escape($BlockEnd) + '\r?\n?'
    return [regex]::Replace($Text, $pattern, '')
}

function Clear-ProfileBlock([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) { return }
    $text = [System.IO.File]::ReadAllText($Path)
    if (-not $text.Contains($BlockBegin)) { return }
    $text = Remove-ProfileBlock $text
    if ($text.Trim().Length -eq 0) {
        Remove-Item -LiteralPath $Path -Force
        Write-Host "    removed $Path"
    }
    else {
        [System.IO.File]::WriteAllText($Path, $text, (New-Object System.Text.UTF8Encoding($true)))
        Write-Host "    cleaned $Path"
    }
}

function Remove-FromUserPath([string]$Directory) {
    $current = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (-not $current) { return }
    $entries = @($current.Split(';') | Where-Object { $_ })
    $kept = @($entries | Where-Object { $_.TrimEnd('\') -ine $Directory.TrimEnd('\') })
    if ($kept.Count -ne $entries.Count) {
        [Environment]::SetEnvironmentVariable('Path', ($kept -join ';'), 'User')
        Write-Host "    removed $Directory from your user PATH"
    }
}

function Remove-WithRetry([string]$Path) {
    for ($attempt = 0; $attempt -lt 10; $attempt++) {
        if (-not (Test-Path -LiteralPath $Path)) { return }
        try {
            Remove-Item -LiteralPath $Path -Force -ErrorAction Stop
            return
        }
        catch {
            # The process that just exited may still hold the file for a moment.
            Start-Sleep -Milliseconds 300
        }
    }
    Write-Warning "could not delete $Path; delete it by hand"
}

if (Test-Path -LiteralPath $exe) {
    Write-Step 'Running tokred uninstall'
    $uninstallArgs = @('uninstall')
    if ($Purge) { $uninstallArgs += '--purge' }
    if ($Yes) {
        $uninstallArgs += '--yes'
    }
    elseif ([Console]::IsInputRedirected) {
        Write-Host '    no terminal to ask on: continuing without confirmation (--yes)'
        $uninstallArgs += '--yes'
    }
    & $exe @uninstallArgs
    $code = $LASTEXITCODE
    if ($code -eq 2) {
        # `tokred uninstall` exits with 2 when you answer "no".
        Write-Host 'Nothing was changed.'
        return
    }
    if ($code -ne 0) {
        throw "'tokred uninstall' failed; the binary was kept so Claude Code does not point at nothing. Fix the problem above and run this again."
    }
}
else {
    Write-Step "No tokred binary at $exe : skipping 'tokred uninstall'"
}

Write-Step 'Removing the binary'
foreach ($name in @('tokred.exe', 'tokred.exe.new', 'tokred.exe.old')) {
    Remove-WithRetry (Join-Path $binDir $name)
}
foreach ($folder in @($binDir, $tokredHome)) {
    if ((Test-Path -LiteralPath $folder) -and -not (Get-ChildItem -LiteralPath $folder -Force)) {
        Remove-Item -LiteralPath $folder -Force -ErrorAction SilentlyContinue
    }
}
if (Test-Path -LiteralPath $tokredHome) {
    Write-Host "    kept $tokredHome (it is not empty)"
}
else {
    Write-Host "    removed $tokredHome"
}

Write-Step 'Cleaning your user PATH and PowerShell profiles'
Remove-FromUserPath $binDir
$documents = [Environment]::GetFolderPath('MyDocuments')
Clear-ProfileBlock (Join-Path $documents 'WindowsPowerShell\profile.ps1')
Clear-ProfileBlock (Join-Path $documents 'PowerShell\profile.ps1')
Remove-Item -Path Function:\claude -ErrorAction SilentlyContinue

Write-Host ''
Write-Host 'tokred is removed. Open a new terminal so the claude wrapper and the PATH entry are gone.'
