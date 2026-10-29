<#
.SYNOPSIS
  tokred installer for Windows.

.DESCRIPTION
  irm https://raw.githubusercontent.com/sostenes198/tokred/main/install/install.ps1 | iex

  1. downloads tokred-<target>.zip and its .sha256 from the GitHub release and checks the digest;
  2. installs tokred.exe into %LOCALAPPDATA%\tokred\bin (or $env:TOKRED_HOME\bin);
  3. adds that folder to your user PATH and defines a `claude` function in your PowerShell
     profile (one marked block, so it can be replaced or removed cleanly);
  4. runs `tokred configure` (the hooks and the agents), then `tokred proxy enable`, which asks
     for the proxy port and whether conversation content may be recorded for the dashboard,
     saves the answers, starts the proxy and points Claude Code at it.
  It never needs administrator rights and never writes outside the tokred folder, your user PATH
  and your PowerShell profile files. cmd.exe has no persistent aliases: the wrapper covers
  PowerShell only.

  `irm | iex` cannot pass parameters, so every option also has an environment variable:
    -Yes             TOKRED_YES=1             do not ask anything
    -Port N          TOKRED_PORT=N            proxy port (1024-65535)
    -Version X       TOKRED_VERSION=X         install release vX instead of the latest
    -NoModifyPath    TOKRED_NO_MODIFY_PATH=1  do not touch the user PATH
    -NoAlias         TOKRED_NO_ALIAS=1        do not define the claude wrapper
    -NoProxy         TOKRED_NO_PROXY=1        hooks and agents only: do not enable the proxy
    -NoConfigure     TOKRED_NO_CONFIGURE=1    only install the binary
  Also: TOKRED_HOME, TOKRED_RELEASE_BASE_URL (an https:// or file:// folder with the release
  assets; used by the tests and for mirrors).

  If the wrapper does not load in new terminals, your execution policy may be Restricted:
  run  Set-ExecutionPolicy -Scope CurrentUser RemoteSigned  once.
#>
param(
    [switch]$Yes,
    [int]$Port = 0,
    [string]$Version = '',
    [switch]$NoModifyPath,
    [switch]$NoConfigure,
    [switch]$NoAlias,
    [switch]$NoProxy
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$Repo = 'sostenes198/tokred'
$BlockBegin = '# >>> tokred >>>'
$BlockEnd = '# <<< tokred <<<'

function Test-On([string]$Value) {
    return [bool]($Value -and $Value -ne '0')
}

function Write-Step([string]$Message) {
    Write-Host "==> $Message"
}

# ---------------------------------------------------------------- options

if (Test-On $env:TOKRED_YES) { $Yes = $true }
if (Test-On $env:TOKRED_NO_MODIFY_PATH) { $NoModifyPath = $true }
if (Test-On $env:TOKRED_NO_ALIAS) { $NoAlias = $true }
if (Test-On $env:TOKRED_NO_PROXY) { $NoProxy = $true }
if (Test-On $env:TOKRED_NO_CONFIGURE) { $NoConfigure = $true }
if (-not $Version -and $env:TOKRED_VERSION) { $Version = $env:TOKRED_VERSION }
if ($Port -eq 0 -and $env:TOKRED_PORT) {
    $parsed = 0
    if (-not [int]::TryParse($env:TOKRED_PORT, [ref]$parsed)) {
        throw "TOKRED_PORT must be a number between 1024 and 65535, not '$($env:TOKRED_PORT)'"
    }
    $Port = $parsed
}
if ($Port -ne 0 -and ($Port -lt 1024 -or $Port -gt 65535)) {
    throw "the port must be a number between 1024 and 65535, not '$Port'"
}

# ---------------------------------------------------------------- helpers

function Get-Asset([string]$Url, [string]$Destination) {
    if ($Url.StartsWith('file://')) {
        $path = ([System.Uri]$Url).LocalPath
        Copy-Item -LiteralPath $path -Destination $Destination -Force
    }
    elseif ($Url.StartsWith('https://')) {
        [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
        Invoke-WebRequest -UseBasicParsing -Uri $Url -OutFile $Destination
    }
    else {
        throw "unsupported download address: $Url (only https:// and file:// are allowed)"
    }
}

function Remove-ProfileBlock([string]$Text) {
    $pattern = '(?s)' + [regex]::Escape($BlockBegin) + '.*?' + [regex]::Escape($BlockEnd) + '\r?\n?'
    return [regex]::Replace($Text, $pattern, '')
}

function Get-ProfileBlock {
    $function = 'function claude { if (Get-Command tokred -ErrorAction SilentlyContinue) { tokred initialize }; $real = Get-Command claude -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1; if ($real) { & $real.Source @args } else { Write-Error ''claude not found'' } }'
    $lines = @(
        $BlockBegin,
        '# Managed by the tokred installer. Remove it with the tokred uninstall script.',
        $function,
        $BlockEnd
    )
    return ($lines -join "`r`n") + "`r`n"
}

function Set-ProfileBlock([string]$Path, [string]$Block) {
    $directory = Split-Path -Parent $Path
    if (-not (Test-Path -LiteralPath $directory)) {
        New-Item -ItemType Directory -Force -Path $directory | Out-Null
    }
    $text = ''
    if (Test-Path -LiteralPath $Path) {
        $text = [System.IO.File]::ReadAllText($Path)
    }
    $text = Remove-ProfileBlock $text
    if ($text.Length -gt 0 -and -not $text.EndsWith("`n")) {
        $text += "`r`n"
    }
    [System.IO.File]::WriteAllText($Path, $text + $Block, (New-Object System.Text.UTF8Encoding($true)))
    Write-Host "    updated $Path"
}

function Add-ToUserPath([string]$Directory) {
    $current = [Environment]::GetEnvironmentVariable('Path', 'User')
    $entries = @()
    if ($current) {
        $entries = @($current.Split(';') | Where-Object { $_ })
    }
    $present = @($entries | Where-Object { $_.TrimEnd('\') -ieq $Directory.TrimEnd('\') })
    if ($present.Count -eq 0) {
        [Environment]::SetEnvironmentVariable('Path', ((@($entries) + $Directory) -join ';'), 'User')
        Write-Host "    added $Directory to your user PATH"
    }
    $session = @($env:Path.Split(';') | Where-Object { $_.TrimEnd('\') -ieq $Directory.TrimEnd('\') })
    if ($session.Count -eq 0) {
        $env:Path = "$Directory;$env:Path"
    }
}

# ---------------------------------------------------------------- install

$arch = $env:PROCESSOR_ARCHITEW6432
if (-not $arch) { $arch = $env:PROCESSOR_ARCHITECTURE }
switch ($arch) {
    'AMD64' { $cpu = 'x86_64' }
    # No native Windows ARM build is published (the `ring` crate does not cross-compile there in
    # CI); Windows on ARM runs the x86_64 build through its emulation.
    'ARM64' { $cpu = 'x86_64' }
    default { throw "unsupported CPU architecture: $arch" }
}
$target = "$cpu-pc-windows-msvc"
$asset = "tokred-$target.zip"

if ($env:TOKRED_RELEASE_BASE_URL) {
    $base = $env:TOKRED_RELEASE_BASE_URL.TrimEnd('/')
}
elseif ($Version) {
    $base = "https://github.com/$Repo/releases/download/v$($Version.TrimStart('v'))"
}
else {
    $base = "https://github.com/$Repo/releases/latest/download"
}

if ($env:TOKRED_HOME) {
    $tokredHome = $env:TOKRED_HOME
}
else {
    if (-not $env:LOCALAPPDATA) { throw 'LOCALAPPDATA is not set' }
    $tokredHome = Join-Path $env:LOCALAPPDATA 'tokred'
}
$binDir = Join-Path $tokredHome 'bin'
$exe = Join-Path $binDir 'tokred.exe'

$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ('tokred-install-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
try {
    Write-Step "Installing tokred for $target into $binDir"
    Write-Step "Downloading $asset"
    $zip = Join-Path $tmp $asset
    $sidecar = "$zip.sha256"
    try {
        Get-Asset "$base/$asset" $zip
        Get-Asset "$base/$asset.sha256" $sidecar
    }
    catch {
        throw "could not download $base/$asset (is there a release for this platform?): $($_.Exception.Message)"
    }

    Write-Step 'Verifying the checksum'
    $expectedText = (Get-Content -LiteralPath $sidecar -Raw).Trim()
    if (-not $expectedText) { throw 'the checksum file is empty' }
    $expected = ($expectedText -split '\s+')[0].TrimStart('*').ToLowerInvariant()
    $actual = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($expected -ne $actual) {
        throw "checksum mismatch (expected $expected, got $actual); the download was discarded"
    }

    Write-Step 'Unpacking'
    $unpacked = Join-Path $tmp 'unpacked'
    Expand-Archive -LiteralPath $zip -DestinationPath $unpacked -Force
    $found = Get-ChildItem -LiteralPath $unpacked -Recurse -Filter 'tokred.exe' -File | Select-Object -First 1
    if (-not $found) { throw 'the archive does not contain tokred.exe' }

    New-Item -ItemType Directory -Force -Path $binDir | Out-Null
    $staged = "$exe.new"
    $old = "$exe.old"
    Copy-Item -LiteralPath $found.FullName -Destination $staged -Force
    if (Test-Path -LiteralPath $exe) {
        # A running tokred.exe can be renamed but not overwritten.
        if (Test-Path -LiteralPath $old) { Remove-Item -LiteralPath $old -Force -ErrorAction SilentlyContinue }
        Move-Item -LiteralPath $exe -Destination $old -Force
    }
    Move-Item -LiteralPath $staged -Destination $exe -Force
    $installed = (& $exe --version) -join ' '
    if ($LASTEXITCODE -ne 0) { throw 'the installed binary does not run on this machine' }
    Write-Host "    installed $installed at $exe"
}
finally {
    Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
}

if ($NoModifyPath -and $NoAlias) {
    Write-Step 'Leaving your PATH and PowerShell profiles alone (-NoModifyPath and -NoAlias)'
}
else {
    if (-not $NoModifyPath) {
        Write-Step 'Updating your user PATH'
        Add-ToUserPath $binDir
    }
    if (-not $NoAlias) {
        Write-Step 'Updating your PowerShell profiles'
        $documents = [Environment]::GetFolderPath('MyDocuments')
        $profiles = @((Join-Path $documents 'WindowsPowerShell\profile.ps1'))
        $coreDir = Join-Path $documents 'PowerShell'
        if ((Test-Path -LiteralPath $coreDir) -or (Get-Command pwsh -ErrorAction SilentlyContinue)) {
            $profiles += (Join-Path $coreDir 'profile.ps1')
        }
        $block = Get-ProfileBlock
        foreach ($profilePath in $profiles) {
            Set-ProfileBlock $profilePath $block
        }
    }
}

# Nobody can answer questions with -Yes or when stdin is redirected (then the defaults apply).
$canAsk = (-not $Yes) -and (-not [Console]::IsInputRedirected)

$configured = $false
if ($NoConfigure) {
    Write-Step 'Skipping tokred configure (-NoConfigure); run it yourself when you are ready'
}
else {
    Write-Step 'Running tokred configure'
    $configureArgs = @('configure')
    if (-not $canAsk) {
        if (-not $Yes) {
            Write-Host '    no terminal to ask on: applying the defaults (--yes)'
        }
        $configureArgs += '--yes'
    }
    & $exe @configureArgs
    if ($LASTEXITCODE -eq 0) {
        $configured = $true
    }
    elseif ($LASTEXITCODE -eq 2) {
        # The plan was declined: nothing was changed, and the proxy is left alone too.
        Write-Host '    configure was declined: leaving Claude Code and the proxy alone'
    }
    else {
        Write-Warning "tokred is installed, but 'tokred configure' did not finish (is Claude Code installed?). Run it again when you are ready."
    }
}

# The proxy is a separate step: `tokred configure` only touches hooks and agents.
if ($configured) {
    if ($NoProxy) {
        Write-Step "Skipping the proxy (-NoProxy); run 'tokred proxy enable' when you want it"
    }
    else {
        Write-Step 'Running tokred proxy enable'
        $proxyArgs = @('proxy', 'enable')
        if ($Port -gt 0) { $proxyArgs += @('--port', "$Port") }
        if (-not $canAsk) { $proxyArgs += '--yes' }
        & $exe @proxyArgs
        if ($LASTEXITCODE -ne 0) {
            Write-Warning "tokred is installed and configured, but 'tokred proxy enable' did not finish. Run it again when you are ready."
        }
    }
}

Write-Host ''
Write-Host 'tokred is installed.'
if (-not $NoModifyPath) {
    Write-Host '  Open a new terminal to have tokred on your PATH.'
}
else {
    Write-Host "  PATH was not changed; run tokred as $exe."
}
if (-not $NoAlias) {
    Write-Host "  New PowerShell windows have claude wrapped: it runs 'tokred initialize' first. 'tokred proxy disable' / 'tokred proxy enable' is the switch; 'Get-Command claude -CommandType Application' finds the real binary. cmd.exe is not wrapped."
}
Write-Host '  Check it with: tokred doctor      Remove it with the uninstall script (see the README).'
