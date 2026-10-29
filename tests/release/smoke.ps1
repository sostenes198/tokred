<#
.SYNOPSIS
  Release smoke test for Windows (Windows PowerShell 5.1 and PowerShell 7): installs tokred from a
  PUBLISHED GitHub release and checks every feature against real state. The same scenario as
  smoke.sh; the JSON and database assertions are in check.py, the fake Anthropic API in
  fake_upstream.py (both need `python`, 3.7+).

.DESCRIPTION
    powershell -NoProfile -ExecutionPolicy Bypass -File tests\release\smoke.ps1 -Tag v0.1.0-rc.1

  It downloads the release's own install.ps1 and uninstall.ps1 (the binary comes through
  install.ps1) and exits non-zero on the first failed assertion with `not ok - <what>`.

  What it cannot sandbox on Windows: your user PATH (registry) and your PowerShell profile files
  (`Documents\WindowsPowerShell\profile.ps1` and `Documents\PowerShell\profile.ps1`, the paths the
  installer uses). They are backed up first and restored at the end, exactly like
  tests\installer\install_test.ps1, and the test refuses to run unless CI is set or
  -AllowRealProfile is given. TOKRED_HOME, TOKRED_DATA_DIR and CLAUDE_CONFIG_DIR point into a
  throwaway folder.

  Parameters / environment knobs:
    -Tag / SMOKE_TAG         release tag to test, e.g. v0.1.0-rc.1 (required)
    -Repo / SMOKE_REPO       GitHub repository (default sostenes198/tokred); install.ps1 fetches the
                             binary from the repository it was built for
    -AllowRealProfile        run outside CI (edits and restores your user PATH and profiles)
    SMOKE_PORT               proxy port (default: a free one)
    SMOKE_DIR                working folder (default: a new temp folder, removed at the end unless
                             SMOKE_KEEP=1)
    SMOKE_KEEP=1             keep the working folder
    SMOKE_LOG_DIR            on failure the logs are copied here (CI uploads them)
    SMOKE_CONTINUE=1         do not stop at the first failed assertion: count it, go on, exit 1 at the end
    SMOKE_SKIP_UPDATE=1      skip the `tokred update` step
    SMOKE_PYTHON             the python to use (default: python on PATH)
    GITHUB_TOKEN             optional: used by this script's own probe of the GitHub API only

  HTTP is done with curl.exe (in Windows 10 and later) so the requests are the ones smoke.sh makes.
  Arguments with quotes or empty strings are not asserted through the wrapper: Windows PowerShell
  5.1 itself rewrites them when it starts a native program.

  UNVERIFIED: this script could not be run on Windows when it was written; it mirrors the idioms of
  tests\installer\install_test.ps1 (which pass in CI). The first run in CI is its test.
#>
param(
    [string]$Tag = $env:SMOKE_TAG,
    [string]$Repo = $(if ($env:SMOKE_REPO) { $env:SMOKE_REPO } else { 'sostenes198/tokred' }),
    [switch]$AllowRealProfile
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

if (-not $Tag -or $Tag -notmatch '^v\d') {
    [Console]::Error.WriteLine('usage: smoke.ps1 -Tag <tag>   (for example v0.1.0-rc.1)')
    exit 2
}
if (-not $env:CI -and -not $AllowRealProfile) {
    throw 'this test edits your user PATH and PowerShell profiles (and restores them); set CI=1 or pass -AllowRealProfile'
}
$version = $Tag.Substring(1)
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$check = Join-Path $here 'check.py'
$fakeUpstream = Join-Path $here 'fake_upstream.py'
$psHost = (Get-Process -Id $PID).Path
$begin = '# >>> tokred >>>'
$continue = ($env:SMOKE_CONTINUE -eq '1')

# ------------------------------------------------------------------ assertions

$script:asserts = 0
$script:fails = 0
$script:skips = 0
$script:stepName = 'start'
$script:rc = 0

function Step([string]$Name) { $script:stepName = $Name; Write-Host ''; Write-Host "==> $Name" }
function Ok([string]$Message) { $script:asserts++; Write-Host "ok - $Message" }
function Note([string]$Message) { Write-Host "note: $Message" }
function Skip([string]$Message) { $script:skips++; Write-Host "skip - $Message" }
function Die([string]$Message) {
    $text = "not ok - [$($script:stepName)] $Message"
    if ($continue) {
        $script:fails++
        [Console]::Error.WriteLine($text)
        return
    }
    throw $text
}
function Assert-True([bool]$Condition, [string]$Good, [string]$Bad) { if ($Condition) { Ok $Good } else { Die $Bad } }
function Assert-Eq($Actual, $Expected, [string]$Message) {
    if ("$Actual" -ceq "$Expected") { Ok $Message } else { Die "${Message}: got '$Actual', expected '$Expected'" }
}
function Assert-Missing([string]$Path, [string]$Message) { Assert-True (-not (Test-Path -LiteralPath $Path)) $Message "${Message}: should not exist: $Path" }
function Assert-File([string]$Path, [string]$Message) { Assert-True (Test-Path -LiteralPath $Path -PathType Leaf) $Message "${Message}: missing file $Path" }
function Assert-Has([string]$Path, [string]$Text, [string]$Message) {
    $content = if (Test-Path -LiteralPath $Path) { [System.IO.File]::ReadAllText($Path) } else { '' }
    Assert-True ($content.Contains($Text)) $Message "${Message}: $Path does not contain: $Text"
}
function Same-Bytes([string]$A, [string]$B) {
    $x = [System.IO.File]::ReadAllBytes($A)
    $y = [System.IO.File]::ReadAllBytes($B)
    if ($x.Length -ne $y.Length) { return $false }
    for ($i = 0; $i -lt $x.Length; $i++) { if ($x[$i] -ne $y[$i]) { return $false } }
    return $true
}
function Assert-SameFile([string]$A, [string]$B, [string]$Message) { Assert-True (Same-Bytes $A $B) $Message "${Message}: $A differs from $B" }

# Runs a native program without letting stderr text stop the script (Windows PowerShell 5.1 turns
# it into a terminating error under `Stop`). Returns the exit code; the text goes to $OutFile.
function Invoke-Native([string]$OutFile, [string]$File, [string[]]$Arguments) {
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $lines = @($null | & $File @Arguments 2>&1 | ForEach-Object { "$_" })
        $code = $LASTEXITCODE
    }
    finally { $ErrorActionPreference = $previous }
    if ($OutFile) { [System.IO.File]::WriteAllText($OutFile, (($lines -join "`n") + "`n"), (New-Object System.Text.UTF8Encoding($false))) }
    return $code
}

# Chk ARGS...: runs check.py, which prints one `ok - ...` line per assertion it makes.
function Chk([string[]]$Arguments) {
    $file = Join-Path $out 'check.out'
    $code = Invoke-Native $file $py (@('-I', $check) + $Arguments)
    $text = [System.IO.File]::ReadAllText($file)
    if ($code -ne 0) {
        [Console]::Error.WriteLine($text)
        Die "check.py $($Arguments[0]) failed (see above)"
        return
    }
    Write-Host $text.TrimEnd()
    $script:asserts += @($text -split "`n" | Where-Object { $_.StartsWith('ok - ') }).Count
}

# ------------------------------------------------------------------ the sandbox

$py = $null
foreach ($candidate in @($env:SMOKE_PYTHON, 'python', 'python3')) {
    if (-not $candidate) { continue }
    $found = Get-Command $candidate -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($found -and $found.Source -notlike '*WindowsApps*') { $py = $found.Source; break }
}
if (-not $py) { throw 'python (3.7+) is required' }
$curl = (Get-Command curl.exe -ErrorAction Stop).Source

if ($env:SMOKE_DIR) { $work = $env:SMOKE_DIR; $createdWork = $false }
else { $work = Join-Path ([System.IO.Path]::GetTempPath()) ('tokred-smoke-' + [Guid]::NewGuid().ToString('N')); $createdWork = $true }
New-Item -ItemType Directory -Force -Path $work | Out-Null
$work = (Resolve-Path -LiteralPath $work).Path
$tokredHome = Join-Path $work 'th'
$dataDir = Join-Path $work 'data'
$claudeDir = Join-Path $work 'claude'
$dl = Join-Path $work 'dl'
$out = Join-Path $work 'out'
$orig = Join-Path $work 'orig'
$fake = Join-Path $work 'fakebin'
$jar = Join-Path $work 'cookies'
$binDir = Join-Path $tokredHome 'bin'
$exe = Join-Path $binDir 'tokred.exe'
foreach ($d in @($claudeDir, $dl, $out, $orig, $fake)) { New-Item -ItemType Directory -Force -Path $d | Out-Null }

function Get-FreePort {
    $listener = New-Object System.Net.Sockets.TcpListener([System.Net.IPAddress]::Loopback, 0)
    $listener.Start()
    $value = $listener.LocalEndpoint.Port
    $listener.Stop()
    return $value
}
$port = if ($env:SMOKE_PORT) { [int]$env:SMOKE_PORT } else { Get-FreePort }
$releasesApi = "https://api.github.com/repos/$Repo/releases/latest"
$base = "https://github.com/$Repo/releases/download/$Tag"

function Tk([string]$Name, [string[]]$Arguments) {
    $script:rc = Invoke-Native (Join-Path $out "$Name.out") $exe $Arguments
}
function Rc-Is([int]$Expected, [string]$Message, [string]$Name) {
    $text = if (Test-Path (Join-Path $out "$Name.out")) { [System.IO.File]::ReadAllText((Join-Path $out "$Name.out")) } else { '' }
    if ($text.Length -gt 400) { $text = $text.Substring(0, 400) }
    Assert-True ($script:rc -eq $Expected) $Message "${Message}: exit status $($script:rc), expected $Expected ($text)"
}
function Http-Code([string[]]$Arguments) {
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    # The body goes to NUL unless the caller names an output file with -o.
    $sink = if ($Arguments -contains '-o') { @() } else { @('-o', 'NUL') }
    try { $code = (& $curl -s @sink --max-time 10 -w '%{http_code}' @Arguments 2>$null) -join '' }
    finally { $ErrorActionPreference = $previous }
    if (-not $code) { return '000' }
    return $code
}
function Health-Code { return (Http-Code @("http://127.0.0.1:$port/__tokred/health")) }
function Wait-Health {
    for ($i = 0; $i -lt 60; $i++) { if ((Health-Code) -eq '200') { return $true }; Start-Sleep -Milliseconds 300 }
    return $false
}
function Wait-Down {
    for ($i = 0; $i -lt 60; $i++) { if ((Health-Code) -eq '000') { return $true }; Start-Sleep -Milliseconds 300 }
    return $false
}
# On Windows the last part of an uninstall (the binary and its folder) runs after `tokred` has
# exited, in a process of its own: wait for it.
function Wait-ForCleanup([string]$Path) {
    for ($i = 0; $i -lt 150; $i++) {
        if (-not (Test-Path -LiteralPath $Path)) { return }
        Start-Sleep -Milliseconds 300
    }
}
function Count-Blocks([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) { return 0 }
    return ([regex]::Matches([System.IO.File]::ReadAllText($Path), [regex]::Escape($begin))).Count
}
function Remove-Block([string]$Text) {
    return [regex]::Replace($Text, ('(?s)' + [regex]::Escape($begin) + '.*?' + [regex]::Escape('# <<< tokred <<<') + '\r?\n?'), '')
}
function User-PathEntries { return @([Environment]::GetEnvironmentVariable('Path', 'User').Split(';') | Where-Object { $_ }) }

# ------------------------------------------------------------------ the real files that cannot be sandboxed

$documents = [Environment]::GetFolderPath('MyDocuments')
$profileDirs = @((Join-Path $documents 'WindowsPowerShell'), (Join-Path $documents 'PowerShell'))
$profiles = @(foreach ($d in $profileDirs) { Join-Path $d 'profile.ps1' })
$profileBackup = @{}
$dirExisted = @{}
foreach ($i in 0..1) {
    $dirExisted[$i] = Test-Path -LiteralPath $profileDirs[$i]
    $profileBackup[$i] = $null
    if (Test-Path -LiteralPath $profiles[$i]) { $profileBackup[$i] = [System.IO.File]::ReadAllBytes($profiles[$i]) }
}
$userPathBackup = [Environment]::GetEnvironmentVariable('Path', 'User')
$sessionPathBackup = $env:Path
$saved = @{}
foreach ($name in @('TOKRED_HOME', 'TOKRED_DATA_DIR', 'CLAUDE_CONFIG_DIR', 'TOKRED_OFFLINE', 'TOKRED_RELEASE_BASE_URL', 'TOKRED_RELEASES_URL', 'TOKRED_NO_ALIAS', 'TOKRED_NO_PROXY', 'TOKRED_NO_CONFIGURE', 'TOKRED_YES', 'TOKRED_PORT', 'TOKRED_VERSION', 'TOKRED_RECORD_CONTENT')) {
    $saved[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}
$fakeProcess = $null

try {
    $env:TOKRED_HOME = $tokredHome
    $env:TOKRED_DATA_DIR = $dataDir
    $env:CLAUDE_CONFIG_DIR = $claudeDir
    $env:TOKRED_OFFLINE = '1'
    foreach ($name in @('TOKRED_RELEASE_BASE_URL', 'TOKRED_RELEASES_URL', 'TOKRED_NO_ALIAS', 'TOKRED_NO_PROXY', 'TOKRED_NO_CONFIGURE', 'TOKRED_YES', 'TOKRED_PORT', 'TOKRED_VERSION', 'TOKRED_RECORD_CONTENT')) {
        [Environment]::SetEnvironmentVariable($name, $null, 'Process')
    }

    # The user's starting state: a settings.json tokred must not damage, and profiles with their own lines.
    $utf8NoBom = New-Object System.Text.UTF8Encoding($false)
    $utf8Bom = New-Object System.Text.UTF8Encoding($true)
    $settingsJson = '{"model":"opus","hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"echo user-hook"}]}]},"env":{"FOO":"bar"}}'
    [System.IO.File]::WriteAllText((Join-Path $orig 'settings.json'), $settingsJson + "`n", $utf8NoBom)
    [System.IO.File]::WriteAllText((Join-Path $claudeDir 'settings.json'), $settingsJson + "`n", $utf8NoBom)
    foreach ($i in 0..1) {
        New-Item -ItemType Directory -Force -Path $profileDirs[$i] | Out-Null
        [System.IO.File]::WriteAllText($profiles[$i], "# my profile $i`r`n`$env:USER_PROFILE_LINE = '1'`r`n", $utf8Bom)
        Copy-Item -LiteralPath $profiles[$i] -Destination (Join-Path $orig "profile$i.ps1")
    }
    $claudeListBefore = @(Get-ChildItem -LiteralPath $claudeDir -Recurse -Force | ForEach-Object { $_.FullName.Substring($claudeDir.Length) } | Sort-Object)

    # assert_clean: nothing of tokred is left.
    function Assert-Clean([string]$Label, [switch]$EditedAgent) {
        Wait-ForCleanup $tokredHome
        Assert-Missing $tokredHome "${Label}: the tokred folder is gone"
        Assert-Missing $dataDir "${Label}: the data folder is gone"
        Chk @('settings', (Join-Path $claudeDir 'settings.json'), '--hooks', 'absent', '--no-url', '--user', '--equals', (Join-Path $orig 'settings.json'))
        if ($EditedAgent) {
            Chk @('agents', (Join-Path $claudeDir 'agents'), '--only', 'dev')
            Assert-SameFile (Join-Path $claudeDir 'agents\tokred-dev.md') (Join-Path $work 'dev-edited.md') "${Label}: the agent the user edited was kept untouched"
        }
        else { Chk @('agents', (Join-Path $claudeDir 'agents'), '--none') }
        foreach ($i in 0..1) {
            Assert-Eq (Count-Blocks $profiles[$i]) 0 "${Label}: profile $i has no tokred block"
            Assert-SameFile $profiles[$i] (Join-Path $orig "profile$i.ps1") "${Label}: profile $i is byte-for-byte what the user had"
        }
        $left = @(User-PathEntries | Where-Object { $_.TrimEnd('\') -ieq $binDir.TrimEnd('\') })
        Assert-Eq $left.Count 0 "${Label}: the user PATH has no tokred entry"
        Assert-True ((Wait-Down)) "${Label}: nothing answers on port $port" "${Label}: something still answers on port $port"
        $strays = @(Get-Process -Name tokred -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path.StartsWith($work) })
        Assert-Eq $strays.Count 0 "${Label}: no tokred process is left"
        $listNow = @(Get-ChildItem -LiteralPath $claudeDir -Recurse -Force | ForEach-Object { $_.FullName.Substring($claudeDir.Length) } | Sort-Object)
        $allowed = @('\settings.json', '\settings.json.tokred.bak', '\agents')
        if ($EditedAgent) { $allowed += '\agents\tokred-dev.md' }
        $extra = @($listNow | Where-Object { $allowed -notcontains $_ })
        Assert-Eq $extra.Count 0 "${Label}: the Claude folder has only settings.json and tokred's settings backup (extra: $($extra -join ', '))"
        if ($listNow -contains '\settings.json.tokred.bak') { Note "${Label}: settings.json.tokred.bak stays in the Claude folder" }
    }

    # ---------------------------------------------------------- 0. the release assets
    Step "0. download the release assets of $Tag from $Repo"
    foreach ($asset in @('install.ps1', 'uninstall.ps1')) {
        $target = Join-Path $dl $asset
        $code = Invoke-Native $null $curl @('--proto', '=https', '--tlsv1.2', '-fsSL', '--retry', '3', '-o', $target, "$base/$asset")
        if ($code -ne 0 -or -not (Test-Path -LiteralPath $target) -or (Get-Item -LiteralPath $target).Length -eq 0) {
            Die "could not download $base/$asset (is $Tag published in $Repo?)"
        }
        else { Ok "the release has $asset" }
    }

    # ---------------------------------------------------------- 1. install
    Step "1. install from the release: install.ps1 -Yes -Version $version -Port $port"
    $script:rc = Invoke-Native (Join-Path $out 'install.out') $psHost @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $dl 'install.ps1'), '-Yes', '-Version', $version, '-Port', "$port")
    Rc-Is 0 'install.ps1 succeeded' 'install'
    Assert-Has (Join-Path $out 'install.out') 'Verifying the checksum' 'the installer downloaded the archive and verified its checksum'
    Assert-File $exe 'tokred.exe is in $TOKRED_HOME\bin'
    Assert-Eq (((& $exe --version) -join ' ').Trim()) "tokred $version" "tokred --version equals the tag ($Tag)"
    $entries = @(User-PathEntries | Where-Object { $_.TrimEnd('\') -ieq $binDir.TrimEnd('\') })
    Assert-Eq $entries.Count 1 'the user PATH holds the bin folder once'
    foreach ($i in 0..1) {
        Assert-Eq (Count-Blocks $profiles[$i]) 1 "profile $i has exactly one tokred block"
        $stripped = Join-Path $work 'stripped.ps1'
        [System.IO.File]::WriteAllText($stripped, (Remove-Block ([System.IO.File]::ReadAllText($profiles[$i]))), $utf8Bom)
        Assert-SameFile $stripped (Join-Path $orig "profile$i.ps1") "profile ${i}: the user's own lines are intact"
        Assert-Has $profiles[$i] 'function claude {' "profile $i defines the claude function"
    }
    Chk @('settings', (Join-Path $claudeDir 'settings.json'), '--hooks', 'present', '--bin', $exe, '--user', '--url', "http://127.0.0.1:$port")
    Chk @('agents', (Join-Path $claudeDir 'agents'), '--count', '6')
    Chk @('toml', (Join-Path $dataDir 'tokred.toml'), 'proxy', 'enabled', 'true')
    Chk @('toml', (Join-Path $dataDir 'tokred.toml'), 'proxy', 'bind', "`"127.0.0.1:$port`"")
    Chk @('toml', (Join-Path $dataDir 'tokred.toml'), 'proxy', 'log_bodies', 'false')
    Assert-Eq (Health-Code) '200' "the proxy answers GET /__tokred/health on port $port"
    $healthFile = Join-Path $out 'health.json'
    [void](Invoke-Native $healthFile $curl @('-s', '--max-time', '10', "http://127.0.0.1:$port/__tokred/health"))
    Assert-Has $healthFile "`"version`":`"$version`"" "the proxy that answers is version $version"
    Chk @('marker', (Join-Path $dataDir 'tokred.db'), '--is', $version)

    # ---------------------------------------------------------- 2. the claude wrapper
    Step '2. the claude wrapper'
    $log = Join-Path $work 'wrapper.log'
    [System.IO.File]::WriteAllText((Join-Path $fake 'claude.cmd'), "@echo off`r`necho claude %* >> `"$log`"`r`n", $utf8NoBom)
    $driver = Join-Path $work 'drive.ps1'
    $driverText = @"
. '$($profiles[0])'
`$env:Path = '$fake;$binDir;' + `$env:Path
(Get-Command claude).CommandType
claude 'one' 'two words' 'a`$HOME'
"@
    [System.IO.File]::WriteAllText($driver, $driverText, $utf8NoBom)
    $driverNoTokred = Join-Path $work 'drive-notokred.ps1'
    $driverText2 = @"
. '$($profiles[0])'
`$env:Path = '$fake;' + '$($sessionPathBackup -replace "'", "''")'
if (Get-Command tokred -ErrorAction SilentlyContinue) { 'TOKRED-IS-STILL-ON-PATH' }
claude 'one' 'two words' 'a`$HOME'
"@
    [System.IO.File]::WriteAllText($driverNoTokred, $driverText2, $utf8NoBom)

    # What `initialize` does must be visible: stop the proxy and take the URL out first.
    Tk 'proxy-stop' @('proxy', 'stop')
    Rc-Is 0 'tokred proxy stop succeeded' 'proxy-stop'
    Assert-True (Wait-Down) 'the proxy is down before the wrapper runs' 'the proxy still answers after proxy stop'
    Chk @('edit', (Join-Path $claudeDir 'settings.json'), '--drop-url')
    Chk @('settings', (Join-Path $claudeDir 'settings.json'), '--no-url')

    $wrapperOut = Join-Path $out 'wrapper-ps.out'
    $script:rc = Invoke-Native $wrapperOut $psHost @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $driver)
    Rc-Is 0 'the claude wrapper ran in a fresh PowerShell' 'wrapper-ps'
    $text = [System.IO.File]::ReadAllText($wrapperOut)
    Assert-True ($text -match '(?m)^Function\s*$') 'claude is a function in a fresh PowerShell after loading the profile' "claude is not a function: $text"
    $logText = if (Test-Path -LiteralPath $log) { [System.IO.File]::ReadAllText($log) } else { '' }
    Assert-True ($logText.Contains('claude') -and $logText.Contains('one') -and $logText.Contains('"two words"') -and $logText.Contains('a$HOME')) 'the arguments (with a space and a $) reach claude intact' "claude did not get its arguments: $logText"
    Assert-Eq (Health-Code) '200' 'tokred initialize ran: the stopped proxy is running again'
    Chk @('settings', (Join-Path $claudeDir 'settings.json'), '--url', "http://127.0.0.1:$port")
    Remove-Item -LiteralPath $log -Force
    $script:rc = Invoke-Native (Join-Path $out 'wrapper-notokred.out') $psHost @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $driverNoTokred)
    Rc-Is 0 'the wrapper ran with tokred off PATH' 'wrapper-notokred'
    $logText = if (Test-Path -LiteralPath $log) { [System.IO.File]::ReadAllText($log) } else { '' }
    Assert-True ($logText.Contains('claude') -and $logText.Contains('"two words"') -and -not ([System.IO.File]::ReadAllText((Join-Path $out 'wrapper-notokred.out'))).Contains('TOKRED-IS-STILL')) 'with tokred removed from PATH, claude still runs with its arguments' "claude did not run without tokred: $logText"

    # ---------------------------------------------------------- 3. traffic through the proxy
    Step '3. traffic through the proxy, the dashboard and the metrics'
    $fakePortFile = Join-Path $work 'fake.port'
    $fakeLog = Join-Path $work 'fake.log'
    $fakeProcess = Start-Process -FilePath $py -ArgumentList @('-I', "`"$fakeUpstream`"", '--port-file', "`"$fakePortFile`"") -PassThru -WindowStyle Hidden -RedirectStandardError $fakeLog
    for ($i = 0; $i -lt 50 -and -not (Test-Path -LiteralPath $fakePortFile); $i++) { Start-Sleep -Milliseconds 200 }
    if (-not (Test-Path -LiteralPath $fakePortFile)) { Die 'the fake upstream did not start' }
    $fakePort = ([System.IO.File]::ReadAllText($fakePortFile)).Trim()
    Assert-Eq (Http-Code @("http://127.0.0.1:$fakePort/__fake/health")) '200' "the fake upstream answers on port $fakePort"

    $tomlPath = Join-Path $dataDir 'tokred.toml'
    $tomlText = [System.IO.File]::ReadAllText($tomlPath) -replace '(?m)^upstream = .*$', "upstream = `"http://127.0.0.1:$fakePort`""
    [System.IO.File]::WriteAllText($tomlPath, $tomlText, $utf8NoBom)
    Chk @('toml', $tomlPath, 'proxy', 'upstream', "`"http://127.0.0.1:$fakePort`"")
    Tk 'proxy-stop' @('proxy', 'stop')
    Rc-Is 0 'tokred proxy stop succeeded' 'proxy-stop'
    Assert-True (Wait-Down) 'the proxy stopped' 'the proxy still answers after proxy stop'
    Tk 'enable-1' @('proxy', 'enable', '--port', "$port", '--record-content', 'true')
    Rc-Is 0 "tokred proxy enable --port $port --record-content true succeeded" 'enable-1'
    Assert-True (Wait-Health) "the proxy answers again on port $port" "the proxy did not come back on port $port"
    Chk @('toml', $tomlPath, 'proxy', 'log_bodies', 'true')

    $sid = 'smoke-s1'
    $mainModel = 'claude-sonnet-4-5'
    $subModel = 'claude-haiku-4-5'
    $agent = 'smoke-agent-1'
    $systemPrompt = 'You are a smoke test'
    $mainPrompt = 'hello main loop'
    $subPrompt = 'find where main is'
    $mainText = 'fake-reply: hello main loop ok'
    $subText = 'fake-reply: find where main is'
    $tools = '"tools":[{"name":"Read","description":"read","input_schema":{"type":"object"}}]'
    function Write-Body([string]$Name, [string]$Json) { $path = Join-Path $work $Name; [System.IO.File]::WriteAllText($path, $Json, $utf8NoBom); return $path }
    $bodyMain = Write-Body 'req-main.json' "{`"model`":`"$mainModel`",`"max_tokens`":64,`"stream`":true,`"system`":`"$systemPrompt`",$tools,`"messages`":[{`"role`":`"user`",`"content`":`"$mainPrompt`"}]}"
    $bodySub = Write-Body 'req-sub.json' "{`"model`":`"$subModel`",`"max_tokens`":64,`"tools`":[{`"name`":`"Grep`",`"description`":`"grep`",`"input_schema`":{`"type`":`"object`"}}],`"messages`":[{`"role`":`"user`",`"content`":`"$subPrompt`"}]}"

    # Send-Msg BODY OUTFILE SESSION [extra curl args]: a Claude Code style request through the proxy.
    function Send-Msg([string]$Body, [string]$OutFile, [string]$Session, [string[]]$Extra = @()) {
        $curlArgs = @('-s', '-N', '--max-time', '30', '-D', "$OutFile.hdr", '-o', $OutFile, '-w', '%{http_code} %{time_starttransfer} %{time_total}\n',
            "http://127.0.0.1:$port/v1/messages", '-X', 'POST', '-H', 'content-type: application/json', '-H', 'anthropic-version: 2023-06-01',
            '-H', 'x-api-key: smoke-test-not-a-key', '-H', "x-claude-code-session-id: $Session") + $Extra + @('--data-binary', "@$Body")
        $code = Invoke-Native "$OutFile.timing" $curl $curlArgs
        if ($code -ne 0) { Die "the request to the proxy failed ($Body)" }
        return ([System.IO.File]::ReadAllText("$OutFile.timing")).Trim().Split(' ')
    }

    $replyMain = Join-Path $out 'reply-main.sse'
    $t = Send-Msg $bodyMain $replyMain $sid
    Assert-Eq $t[0] '200' 'the main-loop request answered 200 through the proxy'
    Chk @('sse', $replyMain, '--text', $mainText, '--model', $mainModel)
    Assert-True ([System.IO.File]::ReadAllText("$replyMain.hdr") -match '(?mi)^x-fake-upstream: 1') "the reply carries the fake upstream's header (the proxy forwarded it)" 'the reply lacks x-fake-upstream: the proxy answered by itself?'
    $inv = [System.Globalization.CultureInfo]::InvariantCulture
    $first = [double]::Parse($t[1], $inv)
    $last = [double]::Parse($t[2], $inv)
    Assert-True (($last - $first) -ge 0.25) "the reply was streamed: first byte at $first s, last at $last s" "the reply was buffered, not streamed: first byte at $first s, last at $last s"

    $replySub = Join-Path $out 'reply-sub.json'
    $t = Send-Msg $bodySub $replySub $sid @('-H', "x-claude-code-agent-id: $agent")
    Assert-Eq $t[0] '200' 'the subagent request answered 200 through the proxy'
    Chk @('reply', $replySub, '--text', $subText, '--model', $subModel)

    function Dash-Login {
        $previous = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        try { $printed = (@($null | & $exe dashboard --no-open 2>$null) -join "`n") } finally { $ErrorActionPreference = $previous }
        $m = [regex]::Match($printed, 'http://[^\s]*token=[0-9a-f]+')
        if (-not $m.Success) { Die "tokred dashboard --no-open printed no address: $printed"; return }
        Remove-Item -LiteralPath $jar -Force -ErrorAction SilentlyContinue
        $script:dashUrl = $m.Value
        $script:dashCode = Http-Code @('-D', (Join-Path $out 'redeem.hdr'), '-c', $jar, $m.Value)
    }
    function Dash-Get([string]$Path, [string]$OutFile) { return (Http-Code @('-b', $jar, '-o', $OutFile, "http://127.0.0.1:$port$Path")) }

    Dash-Login
    Assert-Eq $script:dashCode '303' 'redeeming the one-time dashboard URL answers 303'
    Assert-Has (Join-Path $out 'redeem.hdr') 'location: /dashboard' 'the redirect goes to /dashboard'
    Assert-Has $jar 'tokred_dash' 'the redeem set the session cookie'
    Assert-Eq (Http-Code @($script:dashUrl)) '401' 'the one-time token cannot be used twice'
    Assert-Eq (Http-Code @("http://127.0.0.1:$port/dashboard/api/sessions")) '401' 'without a cookie the dashboard API is 401'
    Assert-Eq (Dash-Get '/dashboard/api/sessions' (Join-Path $out 'sessions.json')) '200' 'with the cookie the sessions API answers 200'
    Chk @('sessions', (Join-Path $out 'sessions.json'), '--session', $sid, '--requests', '2', '--models', "$mainModel,$subModel")
    Assert-Eq (Dash-Get "/dashboard/api/sessions/$sid" (Join-Path $out 'session-s1.json')) '200' 'the session detail answers 200'
    Chk @('turns', (Join-Path $out 'session-s1.json'), '--session', $sid, '--content', 'true', '--agent-id', $agent,
        '--main-model', $mainModel, '--sub-model', $subModel, '--system', $systemPrompt,
        '--main-sent', $mainPrompt, '--main-text', $mainText, '--sub-sent', $subPrompt, '--sub-text', $subText)
    $metricsFile = Join-Path $out 'metrics.html'
    Assert-Eq (Dash-Get '/metrics' $metricsFile) '200' '/metrics answers 200 with the cookie'
    Assert-True (([System.IO.File]::ReadAllText($metricsFile)) -match '(?i)<html') '/metrics is an HTML page' '/metrics is not HTML'
    Assert-Eq (Http-Code @('-b', $jar, '-H', 'Host: evil.example', "http://127.0.0.1:$port/dashboard/api/sessions")) '421' 'a request with a wrong Host is 421'

    Tk 'stats' @('stats', '--json')
    Rc-Is 0 'tokred stats --json succeeded' 'stats'
    Chk @('stats', (Join-Path $out 'stats.out'), '--requests', '2', '--p95-under', '10')
    Tk 'stats-text' @('stats')
    Rc-Is 0 'tokred stats succeeded' 'stats-text'
    Assert-Has (Join-Path $out 'stats-text.out') 'Proxy' 'tokred stats prints the Proxy section'
    Assert-Has (Join-Path $out 'stats-text.out') '2 handled' 'the Proxy section says 2 requests were handled'
    Tk 'routing' @('routing', 'status')
    Rc-Is 0 'tokred routing status runs' 'routing'
    Assert-Has (Join-Path $out 'routing.out') 'Mode:' 'tokred routing status prints the mode'
    Tk 'pstatus' @('proxy', 'status', '--json')
    Rc-Is 0 'tokred proxy status --json succeeded' 'pstatus'
    Chk @('proxy-status', (Join-Path $out 'pstatus.out'), '--running', 'true', '--port', "$port")

    # A conversation that grows: only the messages that are new are `sent`.
    $s2a = Write-Body 'req-s2a.json' "{`"model`":`"$mainModel`",`"max_tokens`":64,`"system`":`"$systemPrompt`",$tools,`"messages`":[{`"role`":`"user`",`"content`":`"first question`"}]}"
    $s2b = Write-Body 'req-s2b.json' "{`"model`":`"$mainModel`",`"max_tokens`":64,`"system`":`"$systemPrompt`",$tools,`"messages`":[{`"role`":`"user`",`"content`":`"first question`"},{`"role`":`"assistant`",`"content`":`"first answer`"},{`"role`":`"user`",`"content`":`"second question`"}]}"
    [void](Send-Msg $s2a (Join-Path $out 'reply-s2a.json') 'smoke-s2')
    [void](Send-Msg $s2b (Join-Path $out 'reply-s2b.json') 'smoke-s2')
    Assert-Eq (Dash-Get '/dashboard/api/sessions/smoke-s2' (Join-Path $out 'session-s2.json')) '200' "the second session's detail answers 200"
    Chk @('sent', (Join-Path $out 'session-s2.json'), '--session', 'smoke-s2', '--expect', 'assistant=first answer', '--expect', 'user=second question', '--count', '2')

    # Content recording off: nothing is recorded (checked on a NEW session, right after the switch).
    Tk 'enable-2' @('proxy', 'enable', '--record-content', 'false')
    Rc-Is 0 'tokred proxy enable --record-content false succeeded' 'enable-2'
    Chk @('toml', $tomlPath, 'proxy', 'log_bodies', 'false')
    $s3 = Write-Body 'req-s3.json' "{`"model`":`"$mainModel`",`"max_tokens`":64,$tools,`"messages`":[{`"role`":`"user`",`"content`":`"private words`"}]}"
    [void](Send-Msg $s3 (Join-Path $out 'reply-s3.json') 'smoke-s3')
    # Turning recording off restarted the proxy, and the dashboard's sign-ins live in its memory.
    Dash-Login
    Assert-Eq $script:dashCode '303' 'a new dashboard login after the setting changed answers 303'
    Assert-Eq (Dash-Get '/dashboard/api/sessions/smoke-s3' (Join-Path $out 'session-s3.json')) '200' "the new session's detail answers 200"
    Chk @('null-content', (Join-Path $out 'session-s3.json'), '--session', 'smoke-s3')

    # And after a restart (so the later steps start from a known state whatever the above found).
    Tk 'proxy-stop' @('proxy', 'stop')
    Rc-Is 0 'tokred proxy stop succeeded' 'proxy-stop'
    Assert-True (Wait-Down) 'the proxy stopped' 'the proxy still answers after proxy stop'
    Tk 'enable-3' @('proxy', 'enable', '--record-content', 'false')
    Rc-Is 0 'tokred proxy enable --record-content false (after a restart) succeeded' 'enable-3'
    Assert-True (Wait-Health) 'the proxy is back' 'the proxy did not come back'
    $s4 = Write-Body 'req-s4.json' "{`"model`":`"$mainModel`",`"max_tokens`":64,$tools,`"messages`":[{`"role`":`"user`",`"content`":`"private words again`"}]}"
    [void](Send-Msg $s4 (Join-Path $out 'reply-s4.json') 'smoke-s4')
    Dash-Login
    Assert-Eq $script:dashCode '303' 'a new dashboard login after the restart answers 303'
    Assert-Eq (Dash-Get '/dashboard/api/sessions/smoke-s4' (Join-Path $out 'session-s4.json')) '200' 'the session detail answers 200 after the restart'
    Chk @('null-content', (Join-Path $out 'session-s4.json'), '--session', 'smoke-s4')

    # ---------------------------------------------------------- 4. unconfigure / configure
    Step '4. unconfigure and configure'
    $devAgent = Join-Path $claudeDir 'agents\tokred-dev.md'
    [System.IO.File]::AppendAllText($devAgent, "`n# edited by the user`n")
    Copy-Item -LiteralPath $devAgent -Destination (Join-Path $work 'dev-edited.md') -Force
    Tk 'unconfigure' @('unconfigure')
    Rc-Is 0 'tokred unconfigure succeeded' 'unconfigure'
    Chk @('settings', (Join-Path $claudeDir 'settings.json'), '--hooks', 'absent', '--user', '--url', "http://127.0.0.1:$port")
    foreach ($a in @('architect', 'planner', 'reviewer', 'explorer', 'summarizer')) {
        Assert-Missing (Join-Path $claudeDir "agents\tokred-$a.md") "unconfigure removed the agent tokred-$a.md"
    }
    Assert-SameFile $devAgent (Join-Path $work 'dev-edited.md') 'the agent the user edited (tokred-dev.md) is kept untouched'
    Assert-Eq (Health-Code) '200' 'unconfigure left the proxy running'
    Chk @('marker', (Join-Path $dataDir 'tokred.db'), '--empty')
    Tk 'doctor' @('doctor')
    Rc-Is 0 'tokred doctor runs' 'doctor'
    Tk 'init' @('initialize')
    Rc-Is 0 'tokred initialize runs' 'init'
    Tk 'stats2' @('stats')
    Rc-Is 0 'tokred stats runs' 'stats2'
    Chk @('settings', (Join-Path $claudeDir 'settings.json'), '--hooks', 'absent', '--user', '--url', "http://127.0.0.1:$port")
    Assert-Missing (Join-Path $claudeDir 'agents\tokred-architect.md') 'doctor/initialize/stats did not put an agent back'
    Chk @('marker', (Join-Path $dataDir 'tokred.db'), '--empty')
    Tk 'configure' @('configure', '--yes')
    Rc-Is 0 'tokred configure --yes succeeded' 'configure'
    Chk @('settings', (Join-Path $claudeDir 'settings.json'), '--hooks', 'present', '--bin', $exe, '--user', '--url', "http://127.0.0.1:$port")
    Chk @('agents', (Join-Path $claudeDir 'agents'), '--count', '6')
    Assert-SameFile $devAgent (Join-Path $work 'dev-edited.md') 'configure kept the edited agent'
    Assert-File (Join-Path $claudeDir 'agents\tokred-dev.md.new') 'configure wrote the new version of the edited agent next to it (.new)'
    Chk @('marker', (Join-Path $dataDir 'tokred.db'), '--is', $version)

    # ---------------------------------------------------------- 5. the proxy switch
    Step '5. proxy disable / enable'
    Tk 'disable' @('proxy', 'disable')
    Rc-Is 0 'tokred proxy disable succeeded' 'disable'
    Chk @('settings', (Join-Path $claudeDir 'settings.json'), '--hooks', 'present', '--user', '--no-url')
    Chk @('toml', $tomlPath, 'proxy', 'enabled', 'false')
    Assert-Eq (Health-Code) '200' 'disable leaves a running proxy running'
    Remove-Item -LiteralPath $log -Force -ErrorAction SilentlyContinue
    $script:rc = Invoke-Native (Join-Path $out 'wrapper-disabled.out') $psHost @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $driver)
    Rc-Is 0 'claude through the wrapper still runs while the proxy is disabled' 'wrapper-disabled'
    Chk @('settings', (Join-Path $claudeDir 'settings.json'), '--no-url')
    Chk @('toml', $tomlPath, 'proxy', 'enabled', 'false')
    Ok 'opening claude through the wrapper did not re-enable the proxy'
    Tk 'enable-4' @('proxy', 'enable')
    Rc-Is 0 'tokred proxy enable (no flags) succeeded' 'enable-4'
    Chk @('toml', $tomlPath, 'proxy', 'bind', "`"127.0.0.1:$port`"")
    Chk @('toml', $tomlPath, 'proxy', 'enabled', 'true')
    Chk @('settings', (Join-Path $claudeDir 'settings.json'), '--user', '--url', "http://127.0.0.1:$port")
    Assert-Eq (Health-Code) '200' "the proxy is still answering on the saved port $port"

    $foreign = 'https://gateway.example.test/anthropic'
    Chk @('edit', (Join-Path $claudeDir 'settings.json'), '--set-url', $foreign)
    Tk 'init-foreign' @('initialize')
    Rc-Is 0 'tokred initialize with a foreign ANTHROPIC_BASE_URL exits 0' 'init-foreign'
    Assert-Has (Join-Path $out 'init-foreign.out') 'leaves it alone' 'initialize warns that the foreign ANTHROPIC_BASE_URL is left alone'
    Chk @('settings', (Join-Path $claudeDir 'settings.json'), '--foreign', $foreign)
    Tk 'disable-foreign' @('proxy', 'disable')
    Rc-Is 0 'tokred proxy disable with a foreign URL succeeded' 'disable-foreign'
    Chk @('settings', (Join-Path $claudeDir 'settings.json'), '--foreign', $foreign, '--user', '--hooks', 'present')
    Chk @('edit', (Join-Path $claudeDir 'settings.json'), '--drop-url')
    Tk 'enable-5' @('proxy', 'enable')
    Rc-Is 0 'tokred proxy enable brings the URL back' 'enable-5'
    Chk @('settings', (Join-Path $claudeDir 'settings.json'), '--url', "http://127.0.0.1:$port", '--user')

    # ---------------------------------------------------------- 6. update
    Step '6. tokred update against the GitHub release API'
    if ($env:SMOKE_SKIP_UPDATE -eq '1') { Skip 'step 6 skipped (SMOKE_SKIP_UPDATE=1)' }
    else {
        $probeArgs = @('-H', 'User-Agent: tokred-release-smoke')
        if ($env:GITHUB_TOKEN) { $probeArgs += @('-H', "Authorization: Bearer $($env:GITHUB_TOKEN)") }
        $probe = Http-Code ($probeArgs + @($releasesApi))
        $env:TOKRED_OFFLINE = $null
        $env:TOKRED_RELEASES_URL = $releasesApi
        Tk 'update' @('update')
        $env:TOKRED_OFFLINE = '1'
        $env:TOKRED_RELEASES_URL = $null
        $updateText = [System.IO.File]::ReadAllText((Join-Path $out 'update.out'))
        if ($probe -eq '404') {
            Note "$Repo has only pre-releases (GitHub's latest answers 404)"
            Assert-Eq $script:rc 1 'tokred update exits 1 when there is no stable release'
            Assert-True ($updateText.Contains('no stable release')) 'tokred update says that no stable release exists' "tokred update did not say that no stable release exists: $($updateText.Trim())"
        }
        elseif ($probe -eq '200') {
            Note "$Repo has a stable release: the update branch is unverified by design (it may replace the binary)"
            if ($updateText -match 'up to date') { Assert-Eq $script:rc 0 'tokred update says it is up to date and exits 0' }
            elseif ($updateText -match 'Updating|Updated to|is available') { Assert-Eq $script:rc 0 'tokred update offered or applied a newer version and exits 0' }
            elseif ($updateText -match '(?i)rate limit|403|429') { Skip 'tokred update was rate limited by the GitHub API' }
            else { Die "tokred update said something unexpected: $($updateText.Trim())" }
        }
        else { Skip "the GitHub API answered '$probe' to the probe (rate limit or no network): step 6 not checked" }
        Assert-True ((((& $exe --version) -join ' ').Trim()).StartsWith('tokred ')) 'the binary still runs after tokred update' 'the binary does not run after tokred update'
    }

    # ---------------------------------------------------------- 6b. update, every scenario
    # A second installation, with folders of its own, goes through every way `tokred update` can be asked
    # to move (on Windows an .exe in use cannot be overwritten), and must run and report the right version
    # after each one.
    Step '6b. tokred update in every scenario (same version, unknown version, another version and back, latest)'
    if ($env:SMOKE_SKIP_UPDATE -eq '1') { Skip 'step 6b skipped (SMOKE_SKIP_UPDATE=1)' }
    else {
        $apiArgs = @('-H', 'User-Agent: tokred-release-smoke')
        if ($env:GITHUB_TOKEN) { $apiArgs += @('-H', "Authorization: Bearer $($env:GITHUB_TOKEN)") }
        $listFile = Join-Path $out 'releases.json'
        $latestFile = Join-Path $out 'latest.json'
        [void](Http-Code ($apiArgs + @('-o', $listFile, "https://api.github.com/repos/$Repo/releases?per_page=30")))
        [void](Http-Code ($apiArgs + @('-o', $latestFile, $releasesApi)))
        $other = ''
        try {
            # The newest other published version that has binaries (v0.1.0-rc.1 has no `update --version`).
            $releases = @([System.IO.File]::ReadAllText($listFile) | ConvertFrom-Json)
            foreach ($release in $releases) {
                if ($release.draft -or $release.tag_name -eq $tag -or $release.tag_name -eq 'v0.1.0-rc.1' -or -not $release.assets) { continue }
                $other = "$($release.tag_name)".TrimStart('v')
                break
            }
        }
        catch { $other = '' }
        # The newest other stable release that has binaries (the one a plain `tokred update` must move on from).
        $older = ''
        try {
            foreach ($release in @([System.IO.File]::ReadAllText($listFile) | ConvertFrom-Json)) {
                if ($release.draft -or $release.prerelease -or $release.tag_name -eq $tag -or -not $release.assets) { continue }
                $older = "$($release.tag_name)".TrimStart('v')
                break
            }
        }
        catch { $older = '' }
        # What GitHub calls the latest release: the newest stable one (empty when there is none).
        $latest = ''
        try { $latest = "$(([System.IO.File]::ReadAllText($latestFile) | ConvertFrom-Json).tag_name)" } catch { $latest = '' }
        if (-not $other) { Skip 'no other published version to switch to (or the GitHub API did not answer): step 6b not checked' }
        else {
            $swap = Join-Path $work 'swap'
            foreach ($name in @('home', 'data', 'claude')) { New-Item -ItemType Directory -Force -Path (Join-Path $swap $name) | Out-Null }
            # `update` re-applies the configuration afterwards, which needs a Claude Code folder to configure.
            [System.IO.File]::WriteAllText((Join-Path $swap 'claude\settings.json'), "{}`n", (New-Object System.Text.UTF8Encoding($false)))
            $swapExe = Join-Path $swap 'th\bin\tokred.exe'
            # Swap-Run NAME ARGS...: the second installation's tokred; output in out\NAME.out, status in $script:rc.
            function Swap-Run([string]$Name, [string[]]$Arguments) { $script:rc = Invoke-Native (Join-Path $out "$Name.out") $swapExe $Arguments }
            # Swap-Is VERSION LABEL: the binary runs and reports exactly that version.
            function Swap-Is([string]$Expected, [string]$Label) {
                $reported = ''
                try { $reported = (((& $swapExe --version) -join ' ').Trim()) } catch { $reported = "$_" }
                Assert-True ($reported -eq "tokred $Expected") $Label "${Label}: it reports '$reported', expected 'tokred $Expected'"
            }
            function Out-Text([string]$Name) { return [System.IO.File]::ReadAllText((Join-Path $out "$Name.out")) }
            # GitHub's API refused the question: the scenario is skipped, not failed.
            function Limited([string]$Text) { return ($Text -match '(?i)rate limit|403|429') }
            $saved = @{}
            foreach ($name in @('TOKRED_HOME', 'TOKRED_DATA_DIR', 'CLAUDE_CONFIG_DIR', 'TOKRED_OFFLINE', 'TOKRED_RELEASES_URL')) { $saved[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
            try {
                $env:TOKRED_HOME = Join-Path $swap 'th'
                $env:TOKRED_DATA_DIR = Join-Path $swap 'data'
                $env:CLAUDE_CONFIG_DIR = Join-Path $swap 'claude'
                $env:TOKRED_OFFLINE = $null
                $env:TOKRED_RELEASES_URL = $releasesApi
                $script:rc = Invoke-Native (Join-Path $out 'swap-install.out') $psHost @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $dl 'install.ps1'), '-Yes', '-Version', $version, '-NoModifyPath', '-NoAlias', '-NoProxy', '-NoConfigure')
                Rc-Is 0 'a second install.ps1, with folders of its own, succeeded' 'swap-install'
                Swap-Is $version "the second installation starts at $version"

                # The version that is already installed.
                Swap-Run 'swap-same' @('update', '--version', $version)
                Rc-Is 0 "tokred update --version $version (already installed) exits 0" 'swap-same'
                Assert-True ((Out-Text 'swap-same').Contains('is already installed')) "tokred update says that $version is already installed" "tokred update did not say that $version is already installed: $((Out-Text 'swap-same').Trim())"
                Swap-Is $version 'the binary is untouched after asking for the version it already is'

                # A version nobody published: an error that explains itself, and a binary that still works.
                Swap-Run 'swap-unknown' @('update', '--version', '9.9.9')
                Rc-Is 1 'tokred update --version 9.9.9 (not published) exits 1' 'swap-unknown'
                Assert-True ((Out-Text 'swap-unknown').Contains('Is it a published version')) 'tokred update explains that 9.9.9 is not a published version' "tokred update did not explain the error: $((Out-Text 'swap-unknown').Trim())"
                Swap-Is $version 'the binary is untouched after an update that failed'

                # Another published version, and back: the binary on disk really is replaced each time.
                Swap-Run 'swap-other' @('update', '--version', $other)
                Rc-Is 0 "tokred update --version $other succeeds" 'swap-other'
                Swap-Is $other "the replaced binary reports $other"
                Swap-Run 'swap-back' @('update', '--version', $version)
                Rc-Is 0 "tokred update --version $version succeeds" 'swap-back'
                Swap-Is $version "the replaced binary reports $version again"

                if ($latest -ne $tag) { Note "$tag is not GitHub's latest release ('$latest'): the scenarios of tokred update without a version are not checked" }
                else {
                    # On the newest stable release: nothing newer to move to.
                    Swap-Run 'swap-current' @('update')
                    if (Limited (Out-Text 'swap-current')) { Skip 'tokred update was rate limited by the GitHub API' }
                    else {
                        Rc-Is 0 'tokred update on the latest release exits 0' 'swap-current'
                        Assert-True ((Out-Text 'swap-current').Contains('is up to date')) 'tokred update says that the latest release is up to date' "tokred update did not say it is up to date: $((Out-Text 'swap-current').Trim())"
                        Swap-Is $version 'the latest release stays where it is'
                    }
                    # From an older stable release, a plain `tokred update` moves forward to the latest one.
                    if (-not $older) { Note 'no older stable release is published: the move from an older stable release is not checked' }
                    else {
                        Swap-Run 'swap-down' @('update', '--version', $older)
                        Rc-Is 0 "tokred update --version $older (the older stable release) succeeds" 'swap-down'
                        Swap-Run 'swap-latest' @('update')
                        if (Limited (Out-Text 'swap-latest')) { Skip 'tokred update was rate limited by the GitHub API' }
                        else {
                            Rc-Is 0 "tokred update from $older exits 0" 'swap-latest'
                            Assert-True ((Out-Text 'swap-latest').Contains("Updated to tokred $version")) "tokred update from $older reports the move to $version" "tokred update from $older did not report the move: $((Out-Text 'swap-latest').Trim())"
                            Swap-Is $version "tokred update from $older lands on the latest release"
                        }
                    }
                }
            }
            finally {
                foreach ($name in $saved.Keys) { [Environment]::SetEnvironmentVariable($name, $saved[$name], 'Process') }
            }
        }
    }

    # ---------------------------------------------------------- 7. uninstall, both ways
    Step '7a. uninstall with the installed binary: tokred uninstall'
    Tk 'uninstall1' @('uninstall')
    Rc-Is 0 'tokred uninstall succeeded' 'uninstall1'
    Assert-Clean 'tokred uninstall' -EditedAgent
    Remove-Item -LiteralPath (Join-Path $claudeDir 'agents\tokred-dev.md') -Force -ErrorAction SilentlyContinue

    Step "7b. reinstall, then uninstall with the release's uninstall.ps1"
    $script:rc = Invoke-Native (Join-Path $out 'install2.out') $psHost @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $dl 'install.ps1'), '-Yes', '-Version', $version, '-Port', "$port")
    Rc-Is 0 'the second install.ps1 succeeded' 'install2'
    Assert-Eq (((& $exe --version) -join ' ').Trim()) "tokred $version" 'the reinstalled tokred --version equals the tag'
    Chk @('settings', (Join-Path $claudeDir 'settings.json'), '--hooks', 'present', '--bin', $exe, '--user', '--url', "http://127.0.0.1:$port")
    Assert-Eq (Health-Code) '200' "the reinstalled proxy answers on port $port"
    $script:rc = Invoke-Native (Join-Path $out 'uninstall2.out') $psHost @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $dl 'uninstall.ps1'))
    Rc-Is 0 "the release's uninstall.ps1 succeeded" 'uninstall2'
    Assert-Clean 'uninstall.ps1'

    Write-Host ''
    if ($script:fails -gt 0) {
        [Console]::Error.WriteLine("FAIL: $($script:fails) assertion(s) failed, $($script:asserts) passed, $($script:skips) skipped (release $Tag of $Repo)")
        $script:exitCode = 1
    }
    else {
        Write-Host "PASS: $($script:asserts) assertions passed, $($script:skips) skipped (release $Tag of $Repo)"
        $script:exitCode = 0
    }
}
catch {
    $script:exitCode = 1
    [Console]::Error.WriteLine("$_")
    $failedRun = $true
}
finally {
    if ($script:fails -gt 0 -or $failedRun) {
        Write-Host '---- tail of every log (last 30 lines) ----'
        $logs = @()
        foreach ($folder in @($work, $out, (Join-Path $dataDir 'logs'))) {
            if (Test-Path -LiteralPath $folder) { $logs += @(Get-ChildItem -LiteralPath $folder -File -ErrorAction SilentlyContinue | Where-Object { $_.Extension -in '.log', '.out' }) }
        }
        foreach ($f in $logs) {
            Write-Host "--- $($f.FullName)"
            Get-Content -LiteralPath $f.FullName -Tail 30 -ErrorAction SilentlyContinue | ForEach-Object { Write-Host $_ }
        }
        if ($env:SMOKE_LOG_DIR) {
            New-Item -ItemType Directory -Force -Path $env:SMOKE_LOG_DIR | Out-Null
            foreach ($f in $logs) { Copy-Item -LiteralPath $f.FullName -Destination $env:SMOKE_LOG_DIR -Force -ErrorAction SilentlyContinue }
        }
    }
    # A failed assertion must not leave the proxy or the fake upstream running.
    if (Test-Path -LiteralPath $exe) {
        try { $null = Invoke-Native $null $exe @('proxy', 'stop') } catch { }
    }
    if ($fakeProcess -and -not $fakeProcess.HasExited) { try { Stop-Process -Id $fakeProcess.Id -Force } catch { } }
    # Put back what could not be sandboxed: environment, user PATH, profile files.
    foreach ($name in $saved.Keys) { [Environment]::SetEnvironmentVariable($name, $saved[$name], 'Process') }
    $env:Path = $sessionPathBackup
    [Environment]::SetEnvironmentVariable('Path', $userPathBackup, 'User')
    foreach ($i in 0..1) {
        if ($null -ne $profileBackup[$i]) { [System.IO.File]::WriteAllBytes($profiles[$i], $profileBackup[$i]) }
        elseif (Test-Path -LiteralPath $profiles[$i]) { Remove-Item -LiteralPath $profiles[$i] -Force }
        if (-not $dirExisted[$i] -and (Test-Path -LiteralPath $profileDirs[$i]) -and -not (Get-ChildItem -LiteralPath $profileDirs[$i] -Force)) {
            Remove-Item -LiteralPath $profileDirs[$i] -Force -ErrorAction SilentlyContinue
        }
    }
    if ($env:SMOKE_KEEP -eq '1') { [Console]::Error.WriteLine("kept: $work") }
    elseif ($createdWork) { Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue }
}
exit $script:exitCode
