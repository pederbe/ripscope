# irm https://ripscope.com/install.ps1 | iex
# Uninstall: & ([scriptblock]::Create((irm https://ripscope.com/install.ps1))) -Uninstall
# Settings: RIPSCOPE_VERSION, RIPSCOPE_INSTALL_DIR, RIPSCOPE_RELEASES (mirror),
# RIPSCOPE_EDITORS (all, the default, or none to leave the editors alone).
# From a checkout, as the editors' install commands run it: install.ps1 -Plugin installs
# the checkout's version for its plugins, unless the ripscope.exe found is that version
# or newer, or not one this installer put there. -Plugin -Force installs anyway.
# Dot sourcing defines the filesystem helpers for the native installer checks.
function Get-RipscopeAttributes([string] $Path) {
    try { return [IO.File]::GetAttributes($Path) }
    catch {
        $cause = $_.Exception.GetBaseException()
        if ($cause -is [IO.FileNotFoundException] -or $cause -is [IO.DirectoryNotFoundException]) { return $null }
        throw
    }
}
function Test-RipscopeRegularFile([string] $Path) {
    $attributes = Get-RipscopeAttributes $Path
    return ($null -ne $attributes -and
        ($attributes -band ([IO.FileAttributes]::Directory -bor [IO.FileAttributes]::ReparsePoint)) -eq 0)
}
function Get-RipscopeHash([string] $Path) {
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256 -ErrorAction Stop).Hash.ToLowerInvariant()
}
function Restore-RipscopeExecutable([string] $Destination, [string] $Backup, [string] $OldHash) {
    # ReplaceFileW error 1177 can leave the old executable at Backup and no destination.
    # Restore only a proved old regular file, using a move that cannot overwrite.
    if ($null -ne (Get-RipscopeAttributes $Destination)) { return $false }
    if (-not $OldHash -or -not (Test-RipscopeRegularFile $Backup)) { return $false }
    if ((Get-RipscopeHash $Backup) -ne $OldHash) { return $false }
    [IO.File]::Move($Backup, $Destination)
    return $true
}
function Publish-RipscopeExecutable([string] $Stage, [string] $Destination, [string] $Backup, $State) {
    if (-not (Test-RipscopeRegularFile $Stage)) { throw "Nonregular staged executable: $Stage" }
    if ((Get-RipscopeHash $Stage) -ne $State.NewHash) { throw 'Staged executable identity changed.' }
    if ($null -ne (Get-RipscopeAttributes $Backup)) { throw "Backup path already exists: $Backup" }
    $attributes = Get-RipscopeAttributes $Destination
    if ($null -ne $attributes -and -not (Test-RipscopeRegularFile $Destination)) {
        throw "Refusing directory or reparse destination: $Destination"
    }
    $State.OldHash = if ($null -ne $attributes) { Get-RipscopeHash $Destination } else { $null }
    $State.Attempted = $true
    try {
        if ($State.OldHash) { [IO.File]::Replace($Stage, $Destination, $Backup, $false) }
        else { [IO.File]::Move($Stage, $Destination) }
        $State.Committed = $true
    }
    catch {
        $failure = $_
        $restored = $false
        try { $restored = Restore-RipscopeExecutable $Destination $Backup $State.OldHash }
        catch { Write-Warning "Recovery failed: $_" }
        throw "Executable replacement failed: $failure. Old executable restored: $restored. Inspect destination $Destination, backup $Backup and staging $Stage."
    }
}
function Complete-RipscopePublication([string] $Destination, [string] $Backup, [string] $NewHash) {
    if (-not (Test-RipscopeRegularFile $Destination) -or (Get-RipscopeHash $Destination) -ne $NewHash) {
        throw "Replacement committed but verification failed. Retained backup: $Backup; inspect $Destination."
    }
    if ($null -ne (Get-RipscopeAttributes $Backup)) {
        try { [IO.File]::Delete($Backup) }
        catch {
            Write-Warning "Installed successfully; backup cleanup failed. Retained backup: $Backup. $_"
            return $false
        }
    }
    return $true
}
function Test-RipscopeOlder([string] $Have, [string] $Wanted) {
    # Whether release Have comes before Wanted: by the numbers before any suffix, then a
    # prerelease, with a suffix, before its release. An unknown version comes first.
    # Two prereleases of one release count as equal: the release branch carries none.
    if (-not $Have) { return $true }
    $a = $Have -replace '[^0-9.].*$', ''
    $b = $Wanted -replace '[^0-9.].*$', ''
    $x = $a.Split('.')
    $y = $b.Split('.')
    for ($i = 0; $i -lt [Math]::Max($x.Count, $y.Count); $i++) {
        $p = if ($i -lt $x.Count -and $x[$i]) { [decimal] $x[$i] } else { [decimal] 0 }
        $q = if ($i -lt $y.Count -and $y[$i]) { [decimal] $y[$i] } else { [decimal] 0 }
        if ($p -ne $q) { return $p -lt $q }
    }
    return ($Have -ne $a) -and ($Wanted -eq $b)
}
function Get-RipscopeEditorPaths {
    # Where Vim and Neovim load packages from on their own, and Emacs's integration.
    if (-not $env:LOCALAPPDATA) { throw 'LOCALAPPDATA is not set.' }
    $vimHome = if ($env:HOME) { $env:HOME } else { $env:USERPROFILE }
    $neovimData = if ($env:XDG_DATA_HOME) { Join-Path $env:XDG_DATA_HOME 'nvim-data' } else { Join-Path $env:LOCALAPPDATA 'nvim-data' }
    return @{
        Vim = Join-Path $vimHome 'vimfiles\pack\ripscope'
        Neovim = Join-Path $neovimData 'site\pack\ripscope'
        Lazy = Join-Path $neovimData 'lazy\lazy.nvim'
        Emacs = Join-Path $env:LOCALAPPDATA 'ripscope\emacs'
    }
}
function Set-RipscopeEditorDirectory([string] $Source, [string] $Target) {
    # Staged two levels up, where no editor loads it, then moved into place.
    $parent = [IO.Path]::GetDirectoryName($Target)
    foreach ($path in @($Target, $parent)) {
        $attributes = Get-RipscopeAttributes $path
        if ($null -ne $attributes -and ($attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "Refusing reparse-point editor directory: $path"
        }
    }
    [IO.Directory]::CreateDirectory($parent) | Out-Null
    $next = Join-Path ([IO.Path]::GetDirectoryName($parent)) ('.ripscope-editor.' + [Guid]::NewGuid().ToString('N'))
    [IO.Directory]::CreateDirectory($next) | Out-Null
    foreach ($child in Get-ChildItem -LiteralPath $Source -Force) { Copy-Item -LiteralPath $child.FullName -Destination $next -Recurse }
    if ([IO.Directory]::Exists($Target)) { Remove-Item -LiteralPath $Target -Recurse -Force }
    [IO.Directory]::Move($next, $Target)
}
function Uninstall-Ripscope([string] $InstallDir) {
    if (-not $InstallDir) { $InstallDir = Join-Path $HOME '.local\bin' }
    $executable = Join-Path $InstallDir 'ripscope.exe'
    $inUse = $false
    if (Test-RipscopeRegularFile $executable) {
        try {
            [IO.File]::Delete($executable)
            Write-Output "Removed $executable"
        }
        catch {
            $inUse = $true
            Write-Warning "$executable is in use: close the editors running Ripscope, then uninstall again to remove it."
        }
    }
    $editors = Get-RipscopeEditorPaths
    $directories = @()
    if ([IO.Directory]::Exists($InstallDir)) {
        $directories += @(Get-ChildItem -LiteralPath $InstallDir -Force -Directory -Filter '.ripscope-notices-*' | ForEach-Object { $_.FullName })
    }
    $directories += @($editors.Vim, $editors.Neovim, $editors.Emacs)
    foreach ($directory in $directories) {
        $attributes = Get-RipscopeAttributes $directory
        if ($null -ne $attributes -and ($attributes -band [IO.FileAttributes]::Directory) -ne 0 -and
            ($attributes -band [IO.FileAttributes]::ReparsePoint) -eq 0) {
            try {
                Remove-Item -LiteralPath $directory -Recurse -Force
                Write-Output "Removed $directory"
            }
            catch { Write-Warning "Could not remove ${directory}: $_" }
        }
    }
    # A drive or UNC path, as Ripscope takes one; \cache alone names no drive.
    $cache = if ($env:RIPSCOPE_CACHE_DIR -match '^([A-Za-z]:[\\/]|\\\\)') { $env:RIPSCOPE_CACHE_DIR } else { Join-Path $env:LOCALAPPDATA 'ripscope' }
    Write-Output "Indexes the editors made remain in $cache (RIPSCOPE_CACHE_DIR names another folder when set). Delete the .db files there too if you no longer need them."
    if ($inUse) { throw "Ripscope's executable is in use and was not removed: close the editors running Ripscope and uninstall again." }
}
function Install-Ripscope {
    [CmdletBinding()]
    param(
        [string] $Version = $env:RIPSCOPE_VERSION,
        [string] $InstallDir = $env:RIPSCOPE_INSTALL_DIR,
        [string] $Releases = $env:RIPSCOPE_RELEASES,
        [string] $Editors = $env:RIPSCOPE_EDITORS,
        [switch] $Uninstall,
        [switch] $Plugin,
        [switch] $Force
    )
    $ErrorActionPreference = 'Stop'
    Set-StrictMode -Version 2
    if ($Uninstall) { Uninstall-Ripscope $InstallDir; return }
    if (-not $Editors) { $Editors = 'all' }
    if ($Plugin) {
        $manifest = if ($PSScriptRoot) { Join-Path $PSScriptRoot 'Cargo.toml' } else { '' }
        if (-not $manifest -or -not (Test-RipscopeRegularFile $manifest)) { throw '-Plugin installs for a checkout of the repository.' }
        # The plugins are in place already, and want the executable of their own version.
        $Editors = 'none'
        $line = Select-String -LiteralPath $manifest -Pattern '^version = "([^"]+)"$' | Select-Object -First 1
        $Version = if ($line) { $line.Matches[0].Groups[1].Value } else { '' }
    }
    if ($Editors -notin @('all', 'none')) { throw 'Set RIPSCOPE_EDITORS to all or none.' }
    if ($Plugin -and -not $Force) {
        # The ripscope.exe the editors find: on PATH, else where this installer puts it.
        $destination = Join-Path $(if ($InstallDir) { $InstallDir } else { Join-Path $HOME '.local\bin' }) 'ripscope.exe'
        $found = Get-Command 'ripscope.exe' -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty Source
        if (-not $found -and (Test-RipscopeRegularFile $destination)) { $found = $destination }
        if ($found) {
            $have = ''
            try {
                $reported = @(& $found --version 2>$null)
                if ($LASTEXITCODE -eq 0 -and $reported.Count -gt 0 -and $reported[0] -match '^ripscope (\S+)') { $have = $Matches[1] }
            }
            catch { }
            # A system package's or a build's ripscope.exe stays as it is.
            $ours = [IO.Path]::GetFullPath($found) -eq [IO.Path]::GetFullPath($destination)
            if (-not $ours -or -not (Test-RipscopeOlder $have ($Version -replace '^v', ''))) {
                Write-Output "Ripscope $(if ($have) { $have } else { 'of unknown version' }) is installed: $found"
                return
            }
        }
    }
    if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT -or
        -not [Environment]::Is64BitOperatingSystem -or $env:PROCESSOR_ARCHITECTURE -ne 'AMD64') {
        throw 'The prebuilt archive supports x86-64 Windows.'
    }
    if (-not $Releases) { $Releases = 'https://github.com/pederbe/ripscope/releases' }
    $Releases = $Releases.TrimEnd('/')
    $secure = ([Uri] $Releases).Scheme -eq 'https'
    if (-not $secure -and -not $env:RIPSCOPE_RELEASES -and -not $PSBoundParameters.ContainsKey('Releases')) { throw 'A non-HTTPS mirror must be explicit.' }
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    function Get-ResponseUri($Response) {
        if ($Response.BaseResponse -is [Net.HttpWebResponse]) { return $Response.BaseResponse.ResponseUri }
        return $Response.BaseResponse.RequestMessage.RequestUri
    }
    function Receive-Release([string] $Url, [string] $Path) {
        $response = Invoke-WebRequest -UseBasicParsing -Uri $Url -OutFile $Path -PassThru
        if ($secure -and (Get-ResponseUri $response).Scheme -ne 'https') { throw 'HTTPS download redirected to an insecure URL.' }
    }
    if (-not $Version) {
        $response = Invoke-WebRequest -UseBasicParsing -Uri "$Releases/latest" -Method Head
        $uri = Get-ResponseUri $response
        if ($secure -and $uri.Scheme -ne 'https') { throw 'HTTPS release lookup redirected to an insecure URL.' }
        if ($uri.AbsolutePath -notmatch '/tag/v([^/]+)$') { throw 'Cannot find the latest release; set RIPSCOPE_VERSION.' }
        $Version = $Matches[1]
    }
    $Version = $Version -replace '^v', ''
    if ($Version -notmatch '^[0-9]+\.[0-9]+\.[0-9]+(?:[-.][0-9A-Za-z]+)*$') { throw 'Invalid release version.' }
    if (-not $InstallDir) { $InstallDir = Join-Path $HOME '.local\bin' }
    [IO.Directory]::CreateDirectory($InstallDir) | Out-Null
    $InstallDir = [IO.Path]::GetFullPath($InstallDir)
    $destination = Join-Path $InstallDir 'ripscope.exe'
    if ($null -ne (Get-RipscopeAttributes $destination) -and -not (Test-RipscopeRegularFile $destination)) {
        throw "Refusing directory or reparse destination: $destination"
    }
    $name = "ripscope-$Version-x86_64-windows-msvc"
    $download = Join-Path ([IO.Path]::GetTempPath()) ("ripscope-download-" + [Guid]::NewGuid().ToString('N'))
    [IO.Directory]::CreateDirectory($download) | Out-Null
    $stage = $null
    $backup = Join-Path $InstallDir ('.ripscope-backup-' + [Guid]::NewGuid().ToString('N'))
    $state = @{ Attempted = $false; Committed = $false; OldHash = $null; NewHash = $null }
    try {
        $archive = Join-Path $download 'archive.zip'
        $checksum = Join-Path $download 'checksum'
        Receive-Release "$Releases/download/v$Version/$name.zip" $archive
        Receive-Release "$Releases/download/v$Version/$name.zip.sha256" $checksum
        $record = [IO.File]::ReadAllText($checksum).TrimEnd("`r", "`n")
        if ($record -notmatch ('^([0-9a-fA-F]{64})  ' + [Regex]::Escape("$name.zip") + '$')) { throw 'Invalid archive checksum record.' }
        if ((Get-RipscopeHash $archive) -ne $Matches[1].ToLowerInvariant()) { throw 'Archive checksum mismatch; previous installation preserved.' }
        Add-Type -AssemblyName System.IO.Compression.FileSystem
        $unpacked = Join-Path $download 'unpacked'
        [IO.Directory]::CreateDirectory($unpacked) | Out-Null
        function Assert-SafePath([string] $Path) {
            if ($Path -notmatch '^[A-Za-z0-9_+./-]+$' -or $Path.StartsWith('/')) { throw "Unsafe archive path: $Path" }
            foreach ($part in $Path.TrimEnd('/').Split('/')) {
                if (-not $part -or $part -eq '.' -or $part -eq '..' -or $part.EndsWith('.') -or
                    $part -match '^(CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(?:\.|$)') { throw "Unsafe archive path: $Path" }
            }
        }
        $zip = [IO.Compression.ZipFile]::OpenRead($archive)
        try {
            $seen = New-Object 'Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
            foreach ($entry in $zip.Entries) {
                Assert-SafePath $entry.FullName
                if (-not $entry.FullName.StartsWith("$name/", [StringComparison]::Ordinal)) { throw 'Unexpected archive root.' }
                if (-not $seen.Add($entry.FullName)) { throw 'Duplicate archive entry.' }
                $kind = ($entry.ExternalAttributes -shr 16) -band 0xF000
                if ($kind -notin @(0, 0x8000, 0x4000) -or ($entry.ExternalAttributes -band 0x400) -ne 0) { throw 'Linked or special ZIP entry.' }
            }
            foreach ($entry in $zip.Entries) {
                $path = Join-Path $unpacked $entry.FullName
                if ($entry.FullName.EndsWith('/')) { [IO.Directory]::CreateDirectory($path) | Out-Null }
                else {
                    [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($path)) | Out-Null
                    [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $path, $false)
                }
            }
        }
        finally { $zip.Dispose() }
        $package = Join-Path $unpacked $name
        foreach ($file in @('bin/ripscope.exe', 'LICENSE-MIT', 'LICENSE-APACHE', 'DEPENDENCIES.json', 'BUILD.json',
            'MANIFEST.sha256', 'licenses/SQLITE-NOTICE.txt', 'licenses/rust/COPYRIGHT-library.html', 'contrib/emacs/ripscope.el',
            'contrib/vim/plugin/ripscope.vim', 'contrib/vim/plugin/ripscope.lua', 'contrib/vim/lua/ripscope.lua',
            'contrib/vim/lua/ripscope/session.lua', 'contrib/vim/lua/ripscope/project.lua')) {
            $path = Join-Path $package $file
            if (-not (Test-RipscopeRegularFile $path) -or (Get-Item -LiteralPath $path).Length -eq 0) { throw "Missing mandatory archive file: $file" }
        }
        foreach ($tree in @('rust', 'dependencies')) {
            if (-not [IO.Directory]::Exists((Join-Path $package "licenses/$tree"))) { throw "Missing $tree licence notices." }
            $notices = @(Get-ChildItem -LiteralPath (Join-Path $package "licenses/$tree") -File -Recurse -Force)
            if (@($notices | Where-Object { $_.Name -ne 'COPYRIGHT-library.html' -and $_.Length -gt 0 }).Count -eq 0) {
                throw "Missing nonempty $tree licence payloads."
            }
            foreach ($notice in $notices) {
                if (-not (Test-RipscopeRegularFile $notice.FullName) -or $notice.Length -eq 0) { throw "Empty or nonregular $tree licence payload: $($notice.FullName)" }
            }
        }
        $manifest = New-Object 'Collections.Generic.Dictionary[string,string]' ([StringComparer]::OrdinalIgnoreCase)
        foreach ($line in [IO.File]::ReadAllLines((Join-Path $package 'MANIFEST.sha256'))) {
            if ($line -notmatch '^([0-9a-f]{64})  (.+)$') { throw 'Invalid archive manifest.' }
            $hash, $relative = $Matches[1], $Matches[2]
            Assert-SafePath $relative
            $manifest.Add($relative, $hash)
        }
        $files = @(Get-ChildItem -LiteralPath $package -File -Recurse -Force | Where-Object { $_.FullName -ne (Join-Path $package 'MANIFEST.sha256') })
        if ($files.Count -ne $manifest.Count) { throw 'Archive inventory mismatch.' }
        foreach ($file in $files) {
            $relative = $file.FullName.Substring($package.Length + 1).Replace('\', '/')
            if (-not $manifest.ContainsKey($relative) -or (Get-RipscopeHash $file.FullName) -ne $manifest[$relative]) { throw 'Archive file checksum mismatch.' }
        }
        $state.NewHash = Get-RipscopeHash (Join-Path $package 'bin/ripscope.exe')
        if ((Get-Content -LiteralPath (Join-Path $package 'BUILD.json') -Raw | ConvertFrom-Json).binary_sha256 -ne $state.NewHash) { throw 'Binary identity differs from BUILD.json.' }
        $stage = Join-Path $InstallDir (".ripscope-notices-$Version." + [Guid]::NewGuid().ToString('N'))
        [IO.Directory]::CreateDirectory($stage) | Out-Null
        foreach ($child in Get-ChildItem -LiteralPath $package -Force) { Copy-Item -LiteralPath $child.FullName -Destination $stage -Recurse }
        if (@(Get-ChildItem -LiteralPath $stage -File -Recurse -Force).Count -ne $manifest.Count + 1) { throw 'Staged inventory mismatch.' }
        foreach ($entry in Get-ChildItem -LiteralPath $stage -Recurse -Force) {
            if (($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'Copied support material contains a reparse point.' }
            if (-not $entry.PSIsContainer -and $entry.Name -ne 'MANIFEST.sha256') {
                $relative = $entry.FullName.Substring($stage.Length + 1).Replace('\', '/')
                if (-not $manifest.ContainsKey($relative) -or (Get-RipscopeHash $entry.FullName) -ne $manifest[$relative]) { throw 'Staged file checksum mismatch.' }
            }
        }
        $stagedExecutable = Join-Path $stage 'bin/ripscope.exe'
        $reported = & $stagedExecutable --version
        if ($LASTEXITCODE -ne 0 -or $reported -ne "ripscope $Version") { throw 'Staged executable version mismatch.' }
        Move-Item -LiteralPath (Join-Path $stage 'MANIFEST.sha256') -Destination (Join-Path $stage 'ORIGINAL-ARCHIVE-MANIFEST.sha256')
        "Original archive manifest: bin/ripscope.exe moved to $destination`nExecutable SHA-256: $($state.NewHash)" |
            Set-Content -LiteralPath (Join-Path $stage 'INSTALLATION.txt') -Encoding UTF8
        Publish-RipscopeExecutable $stagedExecutable $destination $backup $state
        $clean = Complete-RipscopePublication $destination $backup $state.NewHash
        try { [IO.Directory]::Delete((Join-Path $stage 'bin')) }
        catch { Write-Warning "Installed successfully; empty staging directory retained: $stage\bin. $_" }
        Write-Output "Installed Ripscope ${Version}: $destination"
        if ($Editors -eq 'all') {
            $paths = Get-RipscopeEditorPaths
            Set-RipscopeEditorDirectory (Join-Path $stage 'contrib\vim') (Join-Path $paths.Vim 'start\ripscope')
            Set-RipscopeEditorDirectory (Join-Path $stage 'contrib\vim') (Join-Path $paths.Neovim 'start\ripscope')
            Set-RipscopeEditorDirectory (Join-Path $stage 'contrib\emacs') $paths.Emacs
            $shown = (Join-Path $paths.Emacs 'ripscope').Replace('\', '/').Replace('"', '\"')
            Write-Output 'Vim and Neovim load Ripscope by themselves. Open a C file in your project and'
            Write-Output 'search with :cs find g NAME in Vim, or :RipscopeFind g NAME in Neovim.'
            Write-Output 'For Emacs, add this line to your init file:'
            Write-Output "  (when (load `"$shown`" t) (ripscope-setup))"
            # lazy.nvim resets the package path, so Neovim loads only what its list names.
            if ([IO.Directory]::Exists($paths.Lazy)) {
                Write-Output 'Neovim with lazy.nvim loads only the plugins in its list. Add this to it:'
                Write-Output "  { dir = vim.fn.stdpath('data') .. '/site/pack/ripscope/start/ripscope' },"
            }
        }
        Write-Output "Add $InstallDir to your user PATH if needed."
    }
    finally {
        if ($stage) {
            if ($state.Attempted) { Write-Output "Retained notices and editor files: $stage" }
            else {
                try { Remove-Item -LiteralPath $stage -Recurse -Force }
                catch { Write-Warning "Could not remove owned staging: $stage. $_" }
            }
        }
        try { Remove-Item -LiteralPath $download -Recurse -Force }
        catch { Write-Warning "Could not remove download directory: $download. $_" }
    }
}
if ($MyInvocation.InvocationName -ne '.') {
    # -Uninstall, -Plugin and -Force arrive as plain arguments from both a script block and -File.
    if ($args -contains '-Uninstall') { Install-Ripscope -Uninstall }
    elseif ($args -contains '-Plugin') { Install-Ripscope -Plugin -Force:($args -contains '-Force') }
    else { Install-Ripscope @args }
}
