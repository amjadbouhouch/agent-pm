# apm (agent-pm) installer for Windows.
#
#   irm https://raw.githubusercontent.com/amjadbouhouch/agent-pm/main/install.ps1 | iex
#   & ([scriptblock]::Create((irm https://raw.githubusercontent.com/amjadbouhouch/agent-pm/main/install.ps1))) -Version v0.3.0
#
# Installs a single self-contained binary. Re-running upgrades in place.
# Environment variables and parameters: see Show-Usage below, or run with -Help.
#
# Runs on Windows PowerShell 5.1 and PowerShell 7. Under `irm | iex` there is no
# file and no parameter binding, so every parameter also has an environment
# variable. Only a run of this file itself ends with `exit 1` on failure; run
# from text (iex, a scriptblock) it throws instead, because `exit` there would
# close the user's shell or end the script that called it.
# ASCII only: PowerShell 5.1 reads a script without a BOM in the ANSI code page.

param(
  [string]$Version = $env:APM_VERSION,
  [string]$Dir = $env:APM_INSTALL_DIR,
  [string]$Repo = $env:APM_REPO,
  [string]$BaseUrl = $env:APM_BASE_URL,
  [switch]$Local,
  [switch]$Uninstall,
  [switch]$ModifyPath,
  [switch]$NoModifyPath,
  [switch]$Help
)

# Everything runs inside this scriptblock, so under `irm | iex` none of its
# functions or variables are left behind in the user's session.
& {
Set-StrictMode -Version 2.0
$ErrorActionPreference = 'Stop'

# The file this code was parsed from: install.ps1 when it runs as a file,
# empty when it runs from text (iex, [scriptblock]::Create).
$scriptFile = {}.File

# Printed by -Help. Not read from this file: under `irm | iex` there is no file.
function Show-Usage {
  Say @'
apm (agent-pm) installer for Windows.

  irm https://raw.githubusercontent.com/amjadbouhouch/agent-pm/main/install.ps1 | iex
  & ([scriptblock]::Create((irm <same url>))) -Version v0.3.0
  .\install.ps1 -Version v0.3.0

Installs a single self-contained binary. Re-running upgrades in place.
Under `irm | iex` parameters cannot be passed: set the environment variables.

Environment:
  APM_VERSION      version to install, e.g. v0.3.0 (default: latest release)
  APM_INSTALL_DIR  where to put apm.exe (default: ~\.agent-pm\bin)
  APM_REPO         GitHub repo to download releases from
  APM_BASE_URL     download from a mirror (URL or local folder) instead of GitHub releases
  APM_MODIFY_PATH  yes/no: edit the user PATH without asking (default: ask)

Parameters (they win over the environment):
  -Local            build from the current source tree instead of downloading
  -Uninstall        remove apm.exe and its PATH entry (your database in ~\.agent-pm is kept)
  -Version X        install a specific version
  -Dir DIR          install into DIR
  -Repo OWNER/NAME  download releases from another GitHub repo
  -BaseUrl URL      download from a mirror (URL or local folder)
  -ModifyPath       add the install dir to your user PATH without asking
  -NoModifyPath     never touch the user PATH
'@
}

# "<tag> <msg>" with the tag coloured, only when the stream is a console, so
# redirected output stays clean. Warnings and errors go to stderr.
function Write-Tagged($tag, $msg, $color, [switch]$Stderr) {
  $colour = -not $env:NO_COLOR
  if (-not $Stderr) {
    if ($colour -and -not [Console]::IsOutputRedirected) {
      Write-Host $tag -ForegroundColor $color -NoNewline
      Write-Host " $msg"
    } else {
      Write-Host "$tag $msg"
    }
    return
  }
  if ($colour -and -not [Console]::IsErrorRedirected) { [Console]::ForegroundColor = $color }
  [Console]::Error.Write($tag)
  [Console]::ResetColor()
  [Console]::Error.WriteLine(" $msg")
}

function Say($msg) { Write-Host $msg }
function Info($msg) { Say "  $msg" }
function Warn($msg) { Write-Tagged '!' $msg Yellow -Stderr }
function Ok($msg) { Write-Tagged 'ok' $msg Green }
# Stops the install; the message is printed once, where the run ends.
function Fail($msg) { throw $msg }

# ---------------------------------------------------------------------- PATH
# The user PATH is read and written in the registry directly: going through
# [Environment]::GetEnvironmentVariable would expand %VARS% in the other
# entries and write them back expanded. $env:APM_PATH_REG_KEY points it at
# another HKCU key; the tests use that so they never touch the real PATH.
function Get-EnvKey {
  $name = if ($env:APM_PATH_REG_KEY) { $env:APM_PATH_REG_KEY } else { 'Environment' }
  [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey($name)
}

function Get-UserPath {
  $key = Get-EnvKey
  try { $key.GetValue('Path', '', 'DoNotExpandEnvironmentNames') } finally { $key.Close() }
}

function Set-UserPath($value) {
  $key = Get-EnvKey
  try {
    if ($value) { $key.SetValue('Path', $value, 'ExpandString') } else { $key.DeleteValue('Path', $false) }
  } finally { $key.Close() }
  # Tell running programs (Explorer, new terminals) that the environment
  # changed: setting a variable through .NET broadcasts WM_SETTINGCHANGE.
  if (-not $env:APM_PATH_REG_KEY) {
    [Environment]::SetEnvironmentVariable('APM_INSTALL_PATH_CHANGED', '1', 'User')
    [Environment]::SetEnvironmentVariable('APM_INSTALL_PATH_CHANGED', $null, 'User')
  }
}

# Compares PATH entries: %VARS% expanded, case and trailing '\' ignored.
function Get-PathId($entry) {
  [Environment]::ExpandEnvironmentVariables($entry).Trim().TrimEnd('\').ToLowerInvariant()
}

function Test-InPath($pathValue, $dir) {
  $id = Get-PathId $dir
  foreach ($entry in ($pathValue -split ';')) {
    if ($entry.Trim() -and (Get-PathId $entry) -eq $id) { return $true }
  }
  $false
}

# Written with %USERPROFILE% rather than the expanded path, like install.sh's $HOME.
function Get-PathEntry($dir) {
  $userProfile = $env:USERPROFILE.TrimEnd('\')
  if ($dir.StartsWith("$userProfile\", 'OrdinalIgnoreCase')) {
    return '%USERPROFILE%' + $dir.Substring($userProfile.Length)
  }
  $dir
}

# Appends the entry unless it is already there, so re-running is a no-op.
function Add-UserPath($dir) {
  $current = Get-UserPath
  if (Test-InPath $current $dir) {
    Info 'Your user PATH already has it.'
    return
  }
  $entry = Get-PathEntry $dir
  Set-UserPath (($current.TrimEnd(';'), $entry | Where-Object { $_ }) -join ';')
  # Under `irm | iex` this is the user's own session, so apm works right away.
  if (-not (Test-InPath $env:Path $dir)) { $env:Path = $env:Path.TrimEnd(';') + ";$dir" }
  Ok "added $dir to your user PATH"
  Say ''
  Info 'Open a new terminal to pick it up.'
}

function Remove-UserPath($dir) {
  $current = Get-UserPath
  if (-not (Test-InPath $current $dir)) { return }
  $id = Get-PathId $dir
  $kept = $current -split ';' | Where-Object { $_.Trim() -and (Get-PathId $_) -ne $id }
  Set-UserPath ($kept -join ';')
  Info "removed $dir from your user PATH"
}

# Only a console someone is typing into gets a question; CI, pipes and
# `powershell -NonInteractive` get the instruction instead.
function Test-Interactive {
  if (-not [Environment]::UserInteractive -or [Console]::IsInputRedirected) { return $false }
  foreach ($arg in [Environment]::GetCommandLineArgs()) {
    if ($arg -like '-NonI*') { return $false }
  }
  $true
}

# ------------------------------------------------------------------- helpers
function Get-FullPath($path) {
  $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($path)
}

function Get-ApmVersion($exe) {
  try {
    $out = & $exe --version 2>$null
    if ($LASTEXITCODE -eq 0 -and $out) { return ([string]$out).Trim() }
  } catch {
    # Not a runnable program: same as a failing one.
  }
  $null
}

# $true when fetched, $false when the source has no such file (HTTP 404, or
# not in the folder). Anything else (network, server errors) stops the
# install rather than silently skipping a file. A folder (or file:// URL) is
# copied from, for mirrors on a share and for the tests.
function Get-Asset($base, $name, $dest) {
  if ($base -notmatch '^https?://') {
    $src = Join-Path $base $name
    if (-not (Test-Path -LiteralPath $src -PathType Leaf)) { return $false }
    Copy-Item -LiteralPath $src -Destination $dest -Force
    return $true
  }
  $url = "$base/$name"
  for ($try = 1; ; $try++) {
    # WebClient, not Invoke-WebRequest: 5.1's -OutFile reads [ ] in a path
    # as a wildcard.
    $client = New-Object Net.WebClient
    try {
      $client.DownloadFile($url, $dest)
      return $true
    } catch {
      $e = $_.Exception
      if ($e.InnerException) { $e = $e.InnerException }
      if ($e -is [Net.WebException] -and $e.Response -and [int]$e.Response.StatusCode -eq 404) {
        return $false
      }
      if ($try -ge 3) { Fail "cannot download $url`: $($e.Message)" }
      Start-Sleep -Seconds $try
    } finally {
      $client.Dispose()
    }
  }
}

# Windows cannot delete or overwrite a running .exe, but it can rename one.
# So a binary is first moved aside to "<name>.old-<pid>", then deleted; one
# still running stays there until a later run removes it. A file that cannot
# even be renamed is held open by something else.
function Move-Aside($path) {
  $old = "$path.old-$PID"
  try { [IO.File]::Move($path, $old) } catch {
    Fail "$path is held open by another program. Close it, then re-run."
  }
  $old
}

# $true when deleted; $false while apm.exe is still running from it.
function Remove-Aside($old) {
  Remove-Item -LiteralPath $old -Force -ErrorAction SilentlyContinue
  -not (Test-Path -LiteralPath $old)
}

# Copies left by earlier runs whose apm was still running; gone once it exits.
function Remove-OldCopies($target) {
  $dir = Split-Path -Parent $target
  if (-not (Test-Path -LiteralPath $dir)) { return }
  Get-ChildItem -LiteralPath $dir -Filter "$(Split-Path -Leaf $target).old-*" |
    Remove-Item -Force -ErrorAction SilentlyContinue
}

# Two renames on one volume, so a concurrent `apm` never sees a half-written
# binary: it runs either the old one or the new one.
function Move-Into($staged, $target) {
  Remove-OldCopies $target
  $old = $null
  if (Test-Path -LiteralPath $target) { $old = Move-Aside $target }
  try { [IO.File]::Move($staged, $target) } catch {
    $reason = $_.Exception.Message
    if ($old) {
      try { [IO.File]::Move($old, $target) } catch { Warn "could not restore $target from $old" }
    }
    Fail "cannot write $target`: $reason"
  }
  if ($old -and -not (Remove-Aside $old)) {
    Info "the previous apm.exe is still running; the next install removes $old"
  }
}

# ------------------------------------------------------------------ install
function Install-Apm {
  if ($Help) { Show-Usage; return }

  if ([Environment]::OSVersion.Platform -ne 'Win32NT') {
    Fail 'install.ps1 is for Windows; on macOS and Linux use install.sh'
  }

  $binName = 'apm.exe'
  $defaultDir = Join-Path $env:USERPROFILE '.agent-pm\bin'
  $installDir = if ($Dir) { Get-FullPath $Dir } else { $defaultDir }
  $target = Join-Path $installDir $binName
  $repoName = if ($Repo) { $Repo } else { 'amjadbouhouch/agent-pm' }
  $tag = if ($Version) { $Version } else { 'latest' }
  # $null = ask when interactive; 'yes'/'no' = decided by parameter, else env.
  $modify = $null
  if ($NoModifyPath) { $modify = 'no' }
  elseif ($ModifyPath) { $modify = 'yes' }
  elseif ($env:APM_MODIFY_PATH -in 'yes', 'no') { $modify = $env:APM_MODIFY_PATH }

  # -------------------------------------------------------------- uninstall
  # ~\.agent-pm also holds the database (apm.db), so only apm.exe goes, and
  # only the bin dir is removed, and only when it is empty.
  if ($Uninstall) {
    $removed = $false
    foreach ($candidate in ($target, (Join-Path $defaultDir $binName) | Select-Object -Unique)) {
      Remove-OldCopies $candidate
      if (Test-Path -LiteralPath $candidate) {
        $old = Move-Aside $candidate
        Info "removed $candidate"
        if (-not (Remove-Aside $old)) {
          Info "it is still running: delete $old once it exits"
        }
        $removed = $true
      }
    }
    if (-not $removed) { Warn 'no installation found' }
    if ($modify -ne 'no') {
      foreach ($d in ($installDir, $defaultDir | Select-Object -Unique)) { Remove-UserPath $d }
    }
    if ((Test-Path -LiteralPath $defaultDir) -and -not (Get-ChildItem -LiteralPath $defaultDir -Force)) {
      Remove-Item -LiteralPath $defaultDir
    }
    Ok "uninstalled (data in $(Split-Path $defaultDir) kept)"
    return
  }

  # --------------------------------------------------------------- download
  Say ''
  Say 'Installing apm'
  Say ''

  $previous = $null
  if (Test-Path -LiteralPath $target) {
    $previous = Get-ApmVersion $target
    if (-not $previous) { $previous = 'unknown' }
  }
  New-Item -ItemType Directory -Force -Path $installDir | Out-Null
  # Staged next to the target so the final move stays on one volume. Ends in
  # .exe so it can run before it replaces anything.
  $staged = Join-Path $installDir ".apm-$PID.tmp.exe"
  $sumFile = "$staged.sha256"

  try {
    if ($Local) {
      $root = if ($scriptFile) { Split-Path -Parent $scriptFile } else { $null }
      if (-not $root -or -not (Test-Path -LiteralPath (Join-Path $root 'Cargo.toml'))) {
        Fail '-Local must run from inside the agent-pm source tree: .\install.ps1 -Local'
      }
      if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        Fail 'cargo is required to build from source: https://rustup.rs'
      }
      Info "building from $root"
      # Same command as CI's release build.
      Push-Location -LiteralPath $root
      try { & cargo build --release --locked --quiet } finally { Pop-Location }
      if ($LASTEXITCODE -ne 0) { Fail 'cargo build failed' }
      Copy-Item -LiteralPath (Join-Path $root "target\release\$binName") -Destination $staged
    } else {
      # Releases are x64 only; Windows 11 on ARM runs them under emulation.
      $arch = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
      if ($arch -notin 'AMD64', 'ARM64') {
        Fail "unsupported architecture: $arch (build from source with -Local)"
      }
      Info 'platform  windows-x64'
      # GitHub requires TLS 1.2, which 5.1 does not enable by default.
      [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

      if ($BaseUrl) {
        # Mirror, share or air-gapped host serving the same asset names.
        $base = $BaseUrl.TrimEnd('/', '\')
        if ($base -match '^file:') { $base = ([Uri]$base).LocalPath }
        elseif ($base -notmatch '^https?://') { $base = Get-FullPath $base }
      } elseif ($tag -eq 'latest') {
        $base = "https://github.com/$repoName/releases/latest/download"
      } else {
        $base = "https://github.com/$repoName/releases/download/$tag"
      }
      $asset = 'apm-windows-x64.exe'
      $source = if ($base -match '^https?://') { "$base/$asset" } else { Join-Path $base $asset }
      Info "source    $source"

      if (-not (Get-Asset $base $asset $staged)) {
        Fail "download failed. No release asset at $source.
      If you have the source tree, build it instead:  .\install.ps1 -Local"
      }

      # Verify the checksum when the release publishes one.
      if (Get-Asset $base "$asset.sha256" $sumFile) {
        $expected = ((Get-Content -LiteralPath $sumFile -Raw).Trim() -split '\s+')[0].ToLowerInvariant()
        $actual = (Get-FileHash -LiteralPath $staged -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actual -ne $expected) { Fail "checksum mismatch: expected $expected, got $actual" }
        Info 'checksum  verified'
      } else {
        Warn 'release published no .sha256 checksum; skipping verification'
      }
    }

    # Prove the binary runs before it replaces a working installation.
    $newVersion = Get-ApmVersion $staged
    if (-not $newVersion) { Fail 'the downloaded binary failed to run' }

    Move-Into $staged $target
  } finally {
    Remove-Item -LiteralPath $staged, $sumFile -Force -ErrorAction SilentlyContinue
  }

  Say ''
  if ($previous -and $previous -ne $newVersion) {
    Ok "upgraded $previous -> $newVersion"
  } elseif ($previous) {
    Ok "reinstalled $newVersion"
  } else {
    Ok "installed $newVersion"
  }
  Info $target

  # ---------------------------------------------------------- shadow check
  # An older copy earlier on PATH would silently win, which looks exactly like
  # "the upgrade did nothing". Name it and say how to fix it.
  $shadow = Get-Command apm -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
  if ($shadow -and (Get-PathId $shadow.Source) -ne (Get-PathId $target)) {
    $shadowVersion = Get-ApmVersion $shadow.Source
    if (-not $shadowVersion) { $shadowVersion = 'unknown' }
    Say ''
    Warn "another apm ($shadowVersion) is earlier on your PATH and will win:"
    Info "  $($shadow.Source)"
    Say ''
    Info "Remove it, or put $installDir first on PATH:"
    Info "  Remove-Item '$($shadow.Source)'"
  }

  # ------------------------------------------------------------------ PATH
  Say ''
  if (Test-InPath $env:Path $installDir) {
    Info 'Run apm guide to get started.'
  } elseif ($modify -eq 'no') {
    Info "Add $installDir to your PATH, or re-run with -ModifyPath."
  } elseif ($modify -eq 'yes') {
    Add-UserPath $installDir
  } else {
    Warn "$installDir is not on your PATH."
    Say ''
    if (Test-Interactive) {
      $reply = Read-Host "  Add it to your user PATH now? [Y/n]"
      if ($reply -match '^(n|no)$') {
        Info "Add $installDir to your PATH, or re-run with -ModifyPath."
      } else {
        Add-UserPath $installDir
      }
    } else {
      Info "Add $installDir to your PATH, or re-run with -ModifyPath."
    }
  }
  Say ''
}

try {
  Install-Apm
} catch {
  $msg = $_.Exception.Message
  # A run of this file: print the error and exit 1, like install.sh.
  if ($scriptFile) {
    Write-Tagged 'error' $msg Red -Stderr
    exit 1
  }
  # From text (iex, a scriptblock): a terminating error, which stops the
  # caller's script (or makes `powershell -Command` exit 1) without closing
  # an interactive shell.
  throw "apm install failed: $msg"
}
}
