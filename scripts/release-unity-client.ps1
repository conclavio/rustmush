<#
.SYNOPSIS
    One-command local builder, packager, and server/release deployer for the
    RustMUSH Unity 3D standalone client (Windows x64, Linux x64, macOS Universal).

.DESCRIPTION
    1. Runs the fast .NET C# & NUnit preflight (`unity/CI~/TacSim.CI.csproj`) unless -SkipTests is set.
    2. Auto-detects the Unity Editor matching `unity/ProjectSettings/ProjectVersion.txt`.
    3. Invokes `TacSim.Editor.TacSimStandaloneBuilder.Build*Batch` in headless batchmode
       (automatically staging sanitized fallback content packs via `IPreprocessBuildWithReport`).
    4. Strips `*_DoNotShip` / `*_BackUpThisFolder_*` folders, creates zipped release bundles +
       `<archive>.zip.json` sidecar manifests + `SHA256SUMS.txt` under `unity/Builds/Releases/`.
    5. When `-DeployTarget` is passed (e.g. `root@conclav.io:/opt/rustmush/clients`), atomically
       uploads each `.zip` + `.zip.json` over SSH (`scp` to `.part` -> `mv` + `chmod 0644`) so
       Caddy serves the archive immediately and `mush-net-web` advertises it on the login page.
    6. Optionally also uploads to a GitHub Release when `-Upload` is passed.

.EXAMPLE
    .\scripts\release-unity-client.ps1 -DeployTarget "root@conclav.io:/opt/rustmush/clients" -SshPort 2222 -IdentityFile "C:\Users\faeni\antigravity\rustmush\hostinger"
    Builds the Windows x64 player, writes its sidecar manifest, and deploys it atomically to conclav.io.

.EXAMPLE
    .\scripts\release-unity-client.ps1 -OnlyIfChanged -DeployTarget "root@conclav.io:/opt/rustmush/clients" -SshPort 2222 -KeepLast 5
    Skips the build if nothing under `unity/` or `crates/tacsim/content/` changed since the last
    local build; otherwise rebuilds, deploys to conclav.io, and keeps the 5 newest zips per platform.

.EXAMPLE
    .\scripts\release-unity-client.ps1 -Targets Windows64,Linux64 -DryRun -DeployTarget "root@conclav.io:/opt/rustmush/clients" -SshPort 2222
    Previews exact Unity, scp, and ssh commands without building or uploading.
#>
[CmdletBinding()]
param(
    [ValidateSet("Windows64", "Linux64", "macOS")]
    [string[]]$Targets = @("Windows64"),

    [string]$UnityPath = "",

    [switch]$OnlyIfChanged,

    [switch]$SkipTests,

    [switch]$SkipBuild,

    [string]$DeployTarget = "",

    [int]$SshPort = 22,

    [string]$IdentityFile = "",

    [int]$KeepLast = 5,

    [switch]$Upload,

    [string]$ReleaseTag = "unity-client-latest",

    [string]$ReleaseTitle = "",

    [bool]$Prerelease = $true,

    [switch]$DryRun
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$UnityProject = Join-Path $RepoRoot "unity"
$BuildsRoot = Join-Path $UnityProject "Builds"
$ReleasesDir = Join-Path $BuildsRoot "Releases"
$LogsDir = Join-Path $UnityProject "Logs"
$LastBuildShaFile = Join-Path $BuildsRoot ".last-unity-build-sha"

$WatchPaths = @(
    "unity/Assets/_TacSim",
    "unity/Packages",
    "unity/ProjectSettings",
    "crates/tacsim/content"
)

function Write-Utf8NoBom([string]$Path, [string]$Text) {
    $enc = New-Object System.Text.UTF8Encoding($false)
    [System.IO.File]::WriteAllText($Path, $Text, $enc)
}

function Get-GitOutput([string[]]$GitArgs) {
    $out = & git -C $RepoRoot @GitArgs 2>$null
    if ($LASTEXITCODE -ne 0) { return "" }
    return (($out | Out-String).Trim())
}

function Resolve-UnityEditorVersion {
    $verFile = Join-Path $UnityProject "ProjectSettings\ProjectVersion.txt"
    if (-not (Test-Path $verFile)) {
        throw "Could not find $verFile"
    }
    foreach ($line in Get-Content $verFile) {
        if ($line -match '^m_EditorVersion:\s*(\S+)') {
            return $Matches[1]
        }
    }
    throw "Could not parse m_EditorVersion from $verFile"
}

function Resolve-UnityExecutable([string]$ExpectedVersion, [string]$ExplicitPath) {
    if (-not [string]::IsNullOrWhiteSpace($ExplicitPath)) {
        if (-not (Test-Path $ExplicitPath)) {
            throw "Explicit -UnityPath not found: $ExplicitPath"
        }
        return (Resolve-Path $ExplicitPath).Path
    }

    if (-not [string]::IsNullOrWhiteSpace($env:UNITY_EDITOR_PATH) -and (Test-Path $env:UNITY_EDITOR_PATH)) {
        return (Resolve-Path $env:UNITY_EDITOR_PATH).Path
    }

    $candidates = @(
        "C:\Program Files\Unity\Hub\Editor\$ExpectedVersion\Editor\Unity.exe",
        "C:\Program Files\Unity\$ExpectedVersion\Editor\Unity.exe",
        "D:\Program Files\Unity\Hub\Editor\$ExpectedVersion\Editor\Unity.exe"
    )
    foreach ($c in $candidates) {
        if (Test-Path $c) { return $c }
    }

    throw "Unity Editor $ExpectedVersion not found. Install it via Unity Hub or pass -UnityPath <path-to-Unity.exe> (or set UNITY_EDITOR_PATH)."
}

function Parse-DeployTarget([string]$RawTarget) {
    if ($RawTarget -notmatch '^([^:\s]+):(/[^ \t\r\n]+)$') {
        throw "-DeployTarget must have the form 'user@host:/absolute/remote/dir' (got '$RawTarget')."
    }
    $remoteHost = $Matches[1]
    $remoteDir = $Matches[2].TrimEnd('/')
    if ($remoteDir -match '\.\.' -or $remoteDir -match "['`";|&`$]") {
        throw "Unsafe remote directory in -DeployTarget: '$remoteDir'"
    }
    return @{
        Host = $remoteHost
        Dir  = $remoteDir
    }
}

$HeadFullSha = Get-GitOutput @("rev-parse", "HEAD")
$HeadShortSha = Get-GitOutput @("rev-parse", "--short=9", "HEAD")
$BranchName = Get-GitOutput @("rev-parse", "--abbrev-ref", "HEAD")
$UnityVersion = Resolve-UnityEditorVersion
$UnityExe = Resolve-UnityExecutable -ExpectedVersion $UnityVersion -ExplicitPath $UnityPath
$ParsedDeploy = if (-not [string]::IsNullOrWhiteSpace($DeployTarget)) {
    Parse-DeployTarget -RawTarget $DeployTarget
} else {
    $null
}

Write-Host "== RustMUSH Unity Standalone Release Builder ==" -ForegroundColor Cyan
Write-Host "  Repo:    $RepoRoot ($BranchName @ $HeadShortSha)"
Write-Host "  Unity:   $UnityVersion -> $UnityExe"
Write-Host "  Targets: $($Targets -join ', ')"
if ($ParsedDeploy) {
    Write-Host "  Deploy:  $($ParsedDeploy.Host):$($ParsedDeploy.Dir) (port $SshPort, keep last $KeepLast)"
}

# Optional change check against last recorded build SHA
if ($OnlyIfChanged -and (Test-Path $LastBuildShaFile)) {
    $lastSha = (Get-Content $LastBuildShaFile -Raw).Trim()
    if (-not [string]::IsNullOrWhiteSpace($lastSha)) {
        $changedFiles = & git -C $RepoRoot diff --name-only $lastSha HEAD -- @WatchPaths 2>$null
        $dirtyTracked = & git -C $RepoRoot status --porcelain -- @WatchPaths 2>$null | Where-Object { $_ -notmatch 'TacSimHumanoid\.controller$' }
        if ($LASTEXITCODE -eq 0 -and -not $changedFiles -and -not $dirtyTracked) {
            Write-Host "No Unity/content changes since last build ($lastSha). Skipping build." -ForegroundColor Yellow
            exit 0
        }
    }
}

# Guard against a live Unity GUI lockfile before starting batchmode (stale lockfiles
# left behind by an aborted batchmode run are unlocked and safe to remove).
$Lockfile = Join-Path $UnityProject "Temp\UnityLockfile"
if (-not $DryRun -and (Test-Path $Lockfile)) {
    $lockHeld = $false
    try {
        $fs = [System.IO.File]::Open($Lockfile, [System.IO.FileMode]::Open, [System.IO.FileAccess]::ReadWrite, [System.IO.FileShare]::None)
        $fs.Close()
    } catch {
        $lockHeld = $true
    }
    if ($lockHeld) {
        throw "Unity Editor currently has '$UnityProject' open (locked $Lockfile). Close the Unity Editor before running a batchmode build."
    }
    Remove-Item -Force $Lockfile -ErrorAction SilentlyContinue
}

# 1. Fast .NET CI compile & NUnit preflight
if (-not $SkipTests) {
    $ciProj = Join-Path $UnityProject "CI~\TacSim.CI.csproj"
    Write-Host "`n[1/5] Running .NET C# & NUnit preflight..." -ForegroundColor Cyan
    if ($DryRun) {
        Write-Host "  [DryRun] dotnet test `"$ciProj`" --configuration Release --verbosity minimal"
    } else {
        & dotnet test $ciProj --configuration Release --verbosity minimal
        if ($LASTEXITCODE -ne 0) {
            throw ".NET preflight failed (exit $LASTEXITCODE). Aborting Unity player build."
        }
    }
}

New-Item -ItemType Directory -Force -Path $ReleasesDir, $LogsDir | Out-Null

$TargetMeta = @{
    "Windows64" = @{
        Method  = "TacSim.Editor.TacSimStandaloneBuilder.BuildWindows64Batch"
        ExeName = "RustMUSH_TacSim.exe"
        Slug    = "windows-x64"
    }
    "Linux64" = @{
        Method  = "TacSim.Editor.TacSimStandaloneBuilder.BuildLinux64Batch"
        ExeName = "RustMUSH_TacSim.x86_64"
        Slug    = "linux-x64"
    }
    "macOS" = @{
        Method  = "TacSim.Editor.TacSimStandaloneBuilder.BuildMacOSBatch"
        ExeName = "RustMUSH_TacSim.app"
        Slug    = "macos-universal"
    }
}

$BuiltEntries = New-Object System.Collections.Generic.List[hashtable]
$TimestampUtc = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")

# 2. Build each requested target via Unity batchmode
Write-Host "`n[2/5] Building standalone target(s)..." -ForegroundColor Cyan
foreach ($t in $Targets) {
    $meta = $TargetMeta[$t]
    $stageDir = Join-Path $BuildsRoot $t
    $outExe = Join-Path $stageDir $meta.ExeName
    $logFile = Join-Path $LogsDir "release-$($meta.Slug).log"
    # Keep commit-stamped filenames whenever deploying to the multi-version server folder
    # (or versioned GitHub Releases); only collapse to rolling filename on *-latest GH Releases.
    $zipName = if ($Upload -and -not $ParsedDeploy -and $ReleaseTag -like "*-latest") {
        "RustMUSH_TacSim-$($meta.Slug).zip"
    } else {
        "RustMUSH_TacSim-$($meta.Slug)-$HeadShortSha.zip"
    }
    $zipPath = Join-Path $ReleasesDir $zipName
    $sidecarPath = "$zipPath.json"

    Write-Host "  -> $t ($($meta.Method)) => $zipName"

    if ($DryRun) {
        Write-Host "     [DryRun] TACSIM_BUILD_OUTPUT=$outExe"
        Write-Host "     [DryRun] `"$UnityExe`" -batchmode -quit -nographics -projectPath `"$UnityProject`" -executeMethod $($meta.Method) -logFile `"$logFile`""
        $BuiltEntries.Add(@{
            Target      = $t
            Slug        = $meta.Slug
            ZipName     = $zipName
            ZipPath     = $zipPath
            SidecarPath = $sidecarPath
            Sha256      = ""
            SizeBytes   = 0
        })
        continue
    }

    if ($SkipBuild) {
        Write-Host "     Skipping Unity build (-SkipBuild); packaging existing $stageDir..."
    } else {
        if (Test-Path $stageDir) {
            Remove-Item -Recurse -Force $stageDir
        }
        New-Item -ItemType Directory -Force -Path $stageDir | Out-Null

        $prevEnv = $env:TACSIM_BUILD_OUTPUT
        try {
            $env:TACSIM_BUILD_OUTPUT = $outExe
            $unityArgs = @(
                "-batchmode",
                "-quit",
                "-nographics",
                "-projectPath", $UnityProject,
                "-executeMethod", $meta.Method,
                "-logFile", $logFile
            )
            $proc = Start-Process -FilePath $UnityExe -ArgumentList $unityArgs -Wait -PassThru -NoNewWindow
            if ($proc.ExitCode -ne 0) {
                Write-Host "`nUnity build failed for $t (exit $($proc.ExitCode)). Last 40 lines of ${logFile}:" -ForegroundColor Red
                if (Test-Path $logFile) {
                    Get-Content $logFile -Tail 40 | ForEach-Object { Write-Host "  $_" }
                }
                throw "Unity standalone build failed for $t."
            }
        } finally {
            $env:TACSIM_BUILD_OUTPUT = $prevEnv
        }
    }

    if (-not (Test-Path $outExe)) {
        throw "Expected standalone build output was not found: $outExe"
    }

    # Remove Unity "DoNotShip" / backup artifacts before zipping
    Get-ChildItem -Path $stageDir -Directory | Where-Object {
        $_.Name -like "*_DoNotShip" -or $_.Name -like "*_BackUpThisFolder_*"
    } | Remove-Item -Recurse -Force

    # Stamp a small build-manifest.json inside the archive beside the executable
    $innerManifest = [ordered]@{
        product       = "RustMUSH TacSim 3D Client"
        target        = $meta.Slug
        commit        = $HeadFullSha
        short_commit  = $HeadShortSha
        branch        = $BranchName
        unity_version = $UnityVersion
        built_at_utc  = $TimestampUtc
    } | ConvertTo-Json -Depth 3
    Write-Utf8NoBom -Path (Join-Path $stageDir "build-manifest.json") -Text ($innerManifest + "`n")

    if (Test-Path $zipPath) {
        Remove-Item -Force $zipPath
    }
    # Use Windows bsdtar (`tar.exe -a -c -f`) for streaming Zip64 support: PowerShell 5.1's
    # Compress-Archive fails with "Stream was too long" once data.unity3d exceeds 2 GiB.
    Write-Host "     Compressing (Zip64) -> $zipPath"
    $tarExe = Get-Command tar.exe -ErrorAction SilentlyContinue
    if ($tarExe) {
        & $tarExe.Source -a -c -f $zipPath -C $stageDir .
        if ($LASTEXITCODE -ne 0) {
            throw "tar.exe failed while creating $zipPath (exit $LASTEXITCODE)."
        }
    } else {
        Compress-Archive -Path (Join-Path $stageDir "*") -DestinationPath $zipPath -CompressionLevel Optimal
    }

    $hash = (Get-FileHash -Path $zipPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $sizeBytes = (Get-Item $zipPath).Length

    # Write sidecar `<archive>.zip.json` read by `mush-net-web` GET /api/client-downloads
    $sidecarManifest = [ordered]@{
        product       = "RustMUSH TacSim 3D Client"
        target        = $meta.Slug
        filename      = $zipName
        commit        = $HeadFullSha
        short_commit  = $HeadShortSha
        branch        = $BranchName
        unity_version = $UnityVersion
        built_at_utc  = $TimestampUtc
        sha256        = $hash
        size_bytes    = $sizeBytes
    } | ConvertTo-Json -Depth 3
    Write-Utf8NoBom -Path $sidecarPath -Text ($sidecarManifest + "`n")

    $BuiltEntries.Add(@{
        Target      = $t
        Slug        = $meta.Slug
        ZipName     = $zipName
        ZipPath     = $zipPath
        SidecarPath = $sidecarPath
        Sha256      = $hash
        SizeBytes   = $sizeBytes
    })
}

# 3. Write SHA256SUMS.txt and record last-built SHA
Write-Host "`n[3/5] Writing checksums & build marker..." -ForegroundColor Cyan
$ChecksumFile = Join-Path $ReleasesDir "SHA256SUMS.txt"
if ($DryRun) {
    Write-Host "  [DryRun] Write sidecar .zip.json manifests, $ChecksumFile, and record $HeadFullSha -> $LastBuildShaFile"
} else {
    $sumLines = foreach ($entry in $BuiltEntries) {
        "$($entry.Sha256)  $($entry.ZipName)"
    }
    Write-Utf8NoBom -Path $ChecksumFile -Text (($sumLines -join "`n") + "`n")
    Write-Utf8NoBom -Path $LastBuildShaFile -Text ($HeadFullSha + "`n")
    foreach ($line in $sumLines) {
        Write-Host "  $line" -ForegroundColor Gray
    }
}

# 4. Optional atomic SSH/SCP deploy to conclav.io clients folder
if ($ParsedDeploy) {
    $rHost = $ParsedDeploy.Host
    $rDir = $ParsedDeploy.Dir
    Write-Host "`n[4/5] Deploying atomically to ${rHost}:${rDir}..." -ForegroundColor Cyan

    $sshBaseArgs = @("-p", "$SshPort")
    $scpBaseArgs = @("-P", "$SshPort")
    if (-not [string]::IsNullOrWhiteSpace($IdentityFile)) {
        if (-not (Test-Path $IdentityFile)) {
            throw "SSH -IdentityFile not found: $IdentityFile"
        }
        $resolvedKey = (Resolve-Path $IdentityFile).Path
        $sshBaseArgs += @("-i", $resolvedKey)
        $scpBaseArgs += @("-i", $resolvedKey)
    }

    if (-not $DryRun) {
        if (-not (Get-Command ssh -ErrorAction SilentlyContinue) -or -not (Get-Command scp -ErrorAction SilentlyContinue)) {
            throw "OpenSSH 'ssh' and 'scp' are required in PATH for -DeployTarget."
        }
        & ssh @sshBaseArgs $rHost "mkdir -p '$rDir' && chmod 755 '$rDir'"
        if ($LASTEXITCODE -ne 0) { throw "Failed to prepare remote directory $rDir on $rHost." }
    } else {
        Write-Host "  [DryRun] ssh $($sshBaseArgs -join ' ') $rHost `"mkdir -p '$rDir' && chmod 755 '$rDir'`""
    }

    foreach ($entry in $BuiltEntries) {
        $zName = $entry.ZipName
        $zPart = "$rDir/$zName.part"
        $zFinal = "$rDir/$zName"
        $mName = "$zName.json"
        $mPart = "$rDir/$mName.part"
        $mFinal = "$rDir/$mName"

        if ($DryRun) {
            Write-Host "  [DryRun] scp $($scpBaseArgs -join ' ') `"$($entry.ZipPath)`" `"${rHost}:${zPart}`""
            Write-Host "  [DryRun] scp $($scpBaseArgs -join ' ') `"$($entry.SidecarPath)`" `"${rHost}:${mPart}`""
            Write-Host "  [DryRun] ssh $($sshBaseArgs -join ' ') $rHost `"chmod 644 '$zPart' '$mPart' && mv -f '$mPart' '$mFinal' && mv -f '$zPart' '$zFinal'`""
        } else {
            & scp @scpBaseArgs $entry.ZipPath "${rHost}:${zPart}"
            if ($LASTEXITCODE -ne 0) { throw "scp failed for $($entry.ZipPath)" }

            & scp @scpBaseArgs $entry.SidecarPath "${rHost}:${mPart}"
            if ($LASTEXITCODE -ne 0) { throw "scp failed for $($entry.SidecarPath)" }

            # Move manifest first, then archive into place atomically so readers never see a zip without its manifest
            & ssh @sshBaseArgs $rHost "chmod 644 '$zPart' '$mPart' && mv -f '$mPart' '$mFinal' && mv -f '$zPart' '$zFinal'"
            if ($LASTEXITCODE -ne 0) { throw "Remote atomic finalize failed for $zName" }
        }

        if ($KeepLast -gt 0) {
            $slug = $entry.Slug
            $pruneCmd = "cd '$rDir' && ls -1t RustMUSH_TacSim-$slug-*.zip 2>/dev/null | tail -n +$($KeepLast + 1) | while read -r old; do [ `"`$old`" = '$zName' ] || rm -f `"`$old`" `"`$old.json`"; done"
            if ($DryRun) {
                Write-Host "  [DryRun] ssh $($sshBaseArgs -join ' ') $rHost `"$pruneCmd`""
            } else {
                & ssh @sshBaseArgs $rHost $pruneCmd
            }
        }
    }
} else {
    Write-Host "`n[4/5] Skipping server deploy (pass -DeployTarget 'user@conclav.io:/opt/rustmush/clients')." -ForegroundColor DarkGray
}

# 5. Optional GitHub Release upload
if ($Upload) {
    Write-Host "`n[5/5] Publishing to GitHub Release '$ReleaseTag'..." -ForegroundColor Cyan
    $title = if ([string]::IsNullOrWhiteSpace($ReleaseTitle)) {
        "Unity 3D Client ($ReleaseTag @ $HeadShortSha)"
    } else {
        $ReleaseTitle
    }
    $notes = "Built locally with Unity $UnityVersion from commit ``$HeadShortSha`` ($BranchName) at $TimestampUtc."
    $uploadFiles = @($BuiltEntries | ForEach-Object { $_.ZipPath }) + @($ChecksumFile)

    if ($DryRun) {
        Write-Host "  [DryRun] gh release view `"$ReleaseTag`" || gh release create `"$ReleaseTag`" --title `"$title`" $(if ($Prerelease) { '--prerelease' } else { '' })"
        Write-Host "  [DryRun] gh release upload `"$ReleaseTag`" $($uploadFiles -join ' ') --clobber"
    } else {
        $gh = Get-Command gh -ErrorAction SilentlyContinue
        if (-not $gh) {
            throw "GitHub CLI (`gh`) not found in PATH. Install `gh` or run without -Upload."
        }
        & gh release view $ReleaseTag *> $null
        if ($LASTEXITCODE -ne 0) {
            $createArgs = @("release", "create", $ReleaseTag, "--target", $HeadFullSha, "--title", $title, "--notes", $notes)
            if ($Prerelease) { $createArgs += "--prerelease" }
            & gh @createArgs
            if ($LASTEXITCODE -ne 0) { throw "gh release create failed (exit $LASTEXITCODE)." }
        } else {
            & gh release edit $ReleaseTag --title $title --notes $notes
        }
        & gh release upload $ReleaseTag @uploadFiles --clobber
        if ($LASTEXITCODE -ne 0) { throw "gh release upload failed (exit $LASTEXITCODE)." }
    }
} else {
    Write-Host "`n[5/5] Skipping GitHub Release upload (pass -Upload to publish via gh)." -ForegroundColor DarkGray
}

Write-Host "`nDone! Release artifacts:" -ForegroundColor Green
foreach ($entry in $BuiltEntries) {
    Write-Host "  $($entry.ZipPath)"
}
