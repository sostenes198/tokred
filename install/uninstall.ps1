<#
.SYNOPSIS
  tokred uninstaller for Windows.

.DESCRIPTION
  irm https://raw.githubusercontent.com/sostenes198/tokred/main/install/uninstall.ps1 | iex

  Removes everything tokred put on the machine, without asking. When tokred.exe is there it runs
  `tokred uninstall` (stops the proxy, removes its scheduled task, takes the hooks, agents and
  proxy URL out of Claude Code and deletes tokred's data folder). That command then starts this
  same script again with -CleanupOnly, once it has exited (a running exe cannot delete itself),
  for the last part: tokred.exe, the tokred folder, the entry in your user PATH and the `claude`
  wrapper in your PowerShell profiles. Without tokred.exe this script does that part by itself.

  TOKRED_HOME (default %LOCALAPPDATA%\tokred) is where the install script put the binary.
  Internal (used by `tokred uninstall`): -CleanupOnly, -TokredHome <folder>, -WaitPid <pid>.
  -Yes and -Purge are accepted and ignored: nothing is asked any more.
#>
param(
    [switch]$CleanupOnly,
    [string]$TokredHome = '',
    [int]$WaitPid = 0,
    [switch]$Yes,
    [switch]$Purge
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$BlockBegin = '# >>> tokred >>>'
$BlockEnd = '# <<< tokred <<<'

function Write-Step([string]$Message) {
    Write-Host "==> $Message"
}

# By hand there is no `tokred uninstall` running yet: the folder is TOKRED_HOME or the default.
if (-not $CleanupOnly) {
    if ($env:TOKRED_HOME) {
        $TokredHome = $env:TOKRED_HOME
    }
    else {
        if (-not $env:LOCALAPPDATA) { throw 'LOCALAPPDATA is not set' }
        $TokredHome = Join-Path $env:LOCALAPPDATA 'tokred'
    }
}
if ($TokredHome) {
    $binDir = Join-Path $TokredHome 'bin'
    $exe = Join-Path $binDir 'tokred.exe'
}

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

if (-not $CleanupOnly) {
    if (Test-Path -LiteralPath $exe) {
        Write-Step 'Running tokred uninstall'
        & $exe uninstall
        # `tokred uninstall` starts the cleanup of the binary by itself and leaves (`return`, not
        # `exit`: this script may be running inside `irm | iex`, and `exit` would close the shell).
        return
    }
    Write-Step "No tokred binary at $exe : removing what is left in your shell"
}

# `tokred uninstall` starts this script and exits: wait for it, then the binary can go.
if ($WaitPid -gt 0) {
    $process = Get-Process -Id $WaitPid -ErrorAction SilentlyContinue
    if ($process) { [void]$process.WaitForExit(30000) }
}

if ($TokredHome) {
    Write-Step 'Removing the binary'
    foreach ($name in @('tokred.exe', 'tokred.exe.new', 'tokred.exe.old')) {
        Remove-WithRetry (Join-Path $binDir $name)
    }
    foreach ($folder in @($binDir, $TokredHome)) {
        if ((Test-Path -LiteralPath $folder) -and -not (Get-ChildItem -LiteralPath $folder -Force)) {
            Remove-Item -LiteralPath $folder -Force -ErrorAction SilentlyContinue
        }
    }
    if (Test-Path -LiteralPath $TokredHome) {
        Write-Host "    kept $TokredHome (it is not empty)"
    }
    else {
        Write-Host "    removed $TokredHome"
    }
}

Write-Step 'Cleaning your user PATH and PowerShell profiles'
if ($TokredHome) { Remove-FromUserPath $binDir }
$documents = [Environment]::GetFolderPath('MyDocuments')
Clear-ProfileBlock (Join-Path $documents 'WindowsPowerShell\profile.ps1')
Clear-ProfileBlock (Join-Path $documents 'PowerShell\profile.ps1')
Remove-Item -Path Function:\claude -ErrorAction SilentlyContinue

Write-Host ''
Write-Host 'tokred is removed. Open a new terminal so the claude wrapper and the PATH entry are gone.'
