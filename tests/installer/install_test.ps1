<#
.SYNOPSIS
  Tests for install/install.ps1 and install/uninstall.ps1 (Windows).

.DESCRIPTION
  Serves a fake release (a real zip built from the just-built tokred.exe, in the layout `dist`
  uses, plus its .sha256) through TOKRED_RELEASE_BASE_URL=file:///..., with a throwaway
  TOKRED_HOME, data folder and Claude folder. Unlike the shell tests this cannot sandbox two
  things: your user PATH (registry) and your PowerShell profile files. They are saved first and
  restored at the end, and the test refuses to run unless CI is set or -AllowRealProfile is given.

    cargo build -p tokred
    powershell -NoProfile -File tests\installer\install_test.ps1 -AllowRealProfile
#>
param(
    [switch]$AllowRealProfile
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

if (-not $env:CI -and -not $AllowRealProfile) {
    throw 'this test edits your user PATH and PowerShell profile (and restores them); set CI=1 or pass -AllowRealProfile'
}

$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$root = (Resolve-Path (Join-Path $here '..\..')).Path
$bin = if ($env:TOKRED_TEST_BIN) { $env:TOKRED_TEST_BIN } else { Join-Path $root 'target\debug\tokred.exe' }
if (-not (Test-Path -LiteralPath $bin)) { throw "build the binary first: cargo build -p tokred (looked for $bin)" }
$installPs1 = Join-Path $root 'install\install.ps1'
$uninstallPs1 = Join-Path $root 'install\uninstall.ps1'
$psHost = (Get-Process -Id $PID).Path
$begin = '# >>> tokred >>>'

function Ok([string]$Message) { Write-Host "ok - $Message" }
# On Windows the last part of an uninstall (the binary and its folder) runs after `tokred`
# has exited, in a process of its own: wait for it.
function Wait-ForCleanup([string]$Path) {
    for ($i = 0; $i -lt 150; $i++) {
        if (-not (Test-Path -LiteralPath $Path)) { return }
        Start-Sleep -Milliseconds 300
    }
}
function Die([string]$Message) { throw "not ok - $Message" }
function Assert-True([bool]$Condition, [string]$Message) { if (-not $Condition) { Die $Message } }
function Count-Blocks([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) { return 0 }
    return ([regex]::Matches([System.IO.File]::ReadAllText($Path), [regex]::Escape($begin))).Count
}

$work = Join-Path ([System.IO.Path]::GetTempPath()) ('tokred-installer-test-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $work | Out-Null

$documents = [Environment]::GetFolderPath('MyDocuments')
$profilePath = Join-Path $documents 'WindowsPowerShell\profile.ps1'
$profileBackup = $null
if (Test-Path -LiteralPath $profilePath) { $profileBackup = [System.IO.File]::ReadAllBytes($profilePath) }
$userPathBackup = [Environment]::GetEnvironmentVariable('Path', 'User')
$saved = @{}
foreach ($name in @('TOKRED_HOME', 'TOKRED_DATA_DIR', 'CLAUDE_CONFIG_DIR', 'TOKRED_OFFLINE', 'TOKRED_RELEASE_BASE_URL', 'TOKRED_NO_ALIAS')) {
    $saved[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}

try {
    # ---------------------------------------------------------- fake release
    $target = if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { 'aarch64-pc-windows-msvc' } else { 'x86_64-pc-windows-msvc' }
    $asset = "tokred-$target.zip"
    $release = Join-Path $work 'release'
    $pack = Join-Path $release "pack\tokred-$target"
    New-Item -ItemType Directory -Force -Path $pack | Out-Null
    Copy-Item -LiteralPath $bin -Destination (Join-Path $pack 'tokred.exe')
    Set-Content -LiteralPath (Join-Path $pack 'README.md') -Value 'readme'
    Compress-Archive -Path $pack -DestinationPath (Join-Path $release $asset) -Force
    $digest = (Get-FileHash -LiteralPath (Join-Path $release $asset) -Algorithm SHA256).Hash.ToLowerInvariant()
    Set-Content -LiteralPath (Join-Path $release "$asset.sha256") -Value "$digest  $asset"

    # ---------------------------------------------------------- sandbox
    $tokredHome = Join-Path $work 'tokred home'
    $claudeDir = Join-Path $work 'claude'
    New-Item -ItemType Directory -Force -Path (Join-Path $claudeDir 'projects') | Out-Null
    $env:TOKRED_HOME = $tokredHome
    $env:TOKRED_DATA_DIR = Join-Path $work 'data'
    $env:CLAUDE_CONFIG_DIR = $claudeDir
    $env:TOKRED_OFFLINE = '1'
    $env:TOKRED_RELEASE_BASE_URL = ([System.Uri]$release).AbsoluteUri
    $env:TOKRED_NO_ALIAS = $null
    $port = Get-Random -Minimum 20000 -Maximum 50000

    # ---------------------------------------------------------- 1. install
    & $psHost -NoProfile -ExecutionPolicy Bypass -File $installPs1 -Yes -Port $port
    Assert-True ($LASTEXITCODE -eq 0) 'the install failed'
    $exe = Join-Path $tokredHome 'bin\tokred.exe'
    Assert-True (Test-Path -LiteralPath $exe) 'tokred.exe was not installed'
    Assert-True (((& $exe --version) -join ' ') -eq ((& $bin --version) -join ' ')) 'the installed binary reports another version'
    Ok 'the binary is installed and runs'

    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $binDir = Join-Path $tokredHome 'bin'
    $entries = @($userPath.Split(';') | Where-Object { $_.TrimEnd('\') -ieq $binDir.TrimEnd('\') })
    Assert-True ($entries.Count -eq 1) 'the user PATH should hold the bin folder once'
    Assert-True ((Count-Blocks $profilePath) -eq 1) 'the profile should hold one tokred block'
    Assert-True ([System.IO.File]::ReadAllText($profilePath).Contains('function claude {')) 'the profile lacks the claude function'
    Ok 'PATH and the profile were updated'

    $toml = [System.IO.File]::ReadAllText((Join-Path $env:TOKRED_DATA_DIR 'tokred.toml'))
    Assert-True ($toml.Contains("127.0.0.1:$port")) 'configure did not write the port'
    Assert-True (Test-Path -LiteralPath (Join-Path $claudeDir 'agents\tokred-dev.md')) 'configure did not install the agents'
    Ok 'tokred configure ran with the port'

    # ---------------------------------------------------------- 2. idempotent
    $before = [System.IO.File]::ReadAllText($profilePath)
    & $psHost -NoProfile -ExecutionPolicy Bypass -File $installPs1 -Yes -Port $port
    Assert-True ($LASTEXITCODE -eq 0) 'the second install failed'
    Assert-True ([System.IO.File]::ReadAllText($profilePath) -ceq $before) 'the profile changed on the second run'
    $entries = @([Environment]::GetEnvironmentVariable('Path', 'User').Split(';') | Where-Object { $_.TrimEnd('\') -ieq $binDir.TrimEnd('\') })
    Assert-True ($entries.Count -eq 1) 'the user PATH grew on the second run'
    Ok 'installing twice changes nothing'

    # ---------------------------------------------------------- 3. the wrapper end to end
    $fake = Join-Path $work 'fake'
    New-Item -ItemType Directory -Force -Path $fake | Out-Null
    $log = Join-Path $work 'wrapper.log'
    Set-Content -LiteralPath (Join-Path $fake 'claude.cmd') -Value "@echo off`r`necho claude %* >> `"$log`""
    Set-Content -LiteralPath (Join-Path $fake 'tokred.cmd') -Value "@echo off`r`necho tokred %* >> `"$log`""
    $driver = Join-Path $work 'drive.ps1'
    Set-Content -LiteralPath $driver -Value ". '$profilePath'`r`n`$env:Path = '$fake;' + `$env:SystemRoot + '\system32'`r`nclaude 'one' 'two words'"
    & $psHost -NoProfile -ExecutionPolicy Bypass -File $driver
    $lines = @(Get-Content -LiteralPath $log)
    Assert-True ($lines.Count -eq 2) "the wrapper logged $($lines.Count) lines, expected 2"
    Assert-True ($lines[0].Trim() -eq 'tokred initialize') "tokred initialize did not run first: $($lines[0])"
    Assert-True ($lines[1].Contains('claude') -and $lines[1].Contains('one') -and $lines[1].Contains('two words')) "claude did not get its arguments: $($lines[1])"
    Ok 'claude runs tokred initialize first and passes its arguments'

    Remove-Item -LiteralPath (Join-Path $fake 'tokred.cmd') -Force
    Remove-Item -LiteralPath $log -Force
    & $psHost -NoProfile -ExecutionPolicy Bypass -File $driver
    $lines = @(Get-Content -LiteralPath $log)
    Assert-True ($lines.Count -eq 1 -and $lines[0].Contains('claude')) 'claude did not run without tokred'
    Ok 'claude still runs when tokred is missing'

    # ---------------------------------------------------------- 4. uninstall
    & $psHost -NoProfile -ExecutionPolicy Bypass -File $uninstallPs1
    Assert-True ($LASTEXITCODE -eq 0) 'the uninstall failed'
    Wait-ForCleanup $tokredHome
    Assert-True (-not (Test-Path -LiteralPath $tokredHome)) 'the tokred folder is still there'
    Assert-True ((Count-Blocks $profilePath) -eq 0) 'the profile block is still there'
    $left = @([Environment]::GetEnvironmentVariable('Path', 'User').Split(';') | Where-Object { $_.TrimEnd('\') -ieq $binDir.TrimEnd('\') })
    Assert-True ($left.Count -eq 0) 'the user PATH still holds the bin folder'
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $claudeDir 'agents\tokred-dev.md'))) 'the agents are still there'
    Assert-True (-not (Test-Path -LiteralPath $env:TOKRED_DATA_DIR)) 'the data folder should be deleted too'
    Ok 'uninstall removes everything without asking: the binary, PATH entry, profile block, hooks, agents and data'

    # ---------------------------------------------------------- 5. the switches and failures
    $env:TOKRED_NO_ALIAS = '1'
    & $psHost -NoProfile -ExecutionPolicy Bypass -File $installPs1 -Yes -NoConfigure
    Assert-True ($LASTEXITCODE -eq 0) 'install with TOKRED_NO_ALIAS failed'
    Assert-True ((Count-Blocks $profilePath) -eq 0) 'TOKRED_NO_ALIAS=1 still wrote the wrapper'
    & $psHost -NoProfile -ExecutionPolicy Bypass -File $uninstallPs1
    Wait-ForCleanup $tokredHome
    $env:TOKRED_NO_ALIAS = $null
    Ok 'TOKRED_NO_ALIAS=1 skips the wrapper'

    $tampered = Join-Path $work 'tampered'
    New-Item -ItemType Directory -Force -Path $tampered | Out-Null
    Copy-Item -LiteralPath (Join-Path $release $asset) -Destination $tampered
    Set-Content -LiteralPath (Join-Path $tampered "$asset.sha256") -Value ('0' * 64 + "  $asset")
    $env:TOKRED_RELEASE_BASE_URL = ([System.Uri]$tampered).AbsoluteUri
    # The installer fails by writing to stderr: under `Stop` Windows PowerShell 5.1 turns that
    # into a terminating error of this script, so the call runs with `Continue`.
    $previousPreference = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    & $psHost -NoProfile -ExecutionPolicy Bypass -File $installPs1 -Yes -NoConfigure 2>&1 | Out-Null
    $rejected = $LASTEXITCODE
    $ErrorActionPreference = $previousPreference
    Assert-True ($rejected -ne 0) 'a wrong checksum was accepted'
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $tokredHome 'bin\tokred.exe'))) 'a rejected download was installed'
    Ok 'a wrong checksum stops the install'

    Write-Host 'all installer tests passed'
}
finally {
    foreach ($name in $saved.Keys) { [Environment]::SetEnvironmentVariable($name, $saved[$name], 'Process') }
    [Environment]::SetEnvironmentVariable('Path', $userPathBackup, 'User')
    if ($null -ne $profileBackup) {
        [System.IO.File]::WriteAllBytes($profilePath, $profileBackup)
    }
    elseif (Test-Path -LiteralPath $profilePath) {
        Remove-Item -LiteralPath $profilePath -Force
    }
    Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
}
