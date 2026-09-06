param(
    [Parameter(Mandatory = $true)]
    [string]$Version,

    [string]$TargetTriple = "x86_64-pc-windows-msvc",

    [string]$CommitSha = "",

    [string]$DistDirectory = "dist"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Assert-Success {
    param([string]$Operation)

    if ($LASTEXITCODE -ne 0) {
        throw "$Operation failed with exit code $LASTEXITCODE"
    }
}

function Add-LicenseCandidate {
    param(
        [System.Collections.Generic.List[string]]$Candidates,
        [string]$Path
    )

    if (Test-Path -LiteralPath $Path -PathType Leaf) {
        $resolved = (Resolve-Path -LiteralPath $Path).Path
        if (-not $Candidates.Contains($resolved)) {
            $Candidates.Add($resolved)
        }
    }
}

if ($Version -notmatch '^[0-9]+[.][0-9]+[.][0-9]+(?:[-+][0-9A-Za-z.-]+)?$') {
    throw "invalid release version: $Version"
}

if ([string]::IsNullOrWhiteSpace($CommitSha)) {
    $CommitSha = (git rev-parse HEAD | Out-String).Trim()
    Assert-Success "git rev-parse HEAD"
}
if ($CommitSha -notmatch '^[0-9a-fA-F]{40}$') {
    throw "invalid source commit SHA: $CommitSha"
}
$CommitSha = $CommitSha.ToLowerInvariant()

if ($env:RUNNER_ARCH -and $env:RUNNER_ARCH -ne "X64") {
    throw "release packaging requires an X64 runner; got $($env:RUNNER_ARCH)"
}

$metadataJson = cargo metadata --locked --format-version 1 --filter-platform $TargetTriple
Assert-Success "cargo metadata"
$metadata = $metadataJson | ConvertFrom-Json

if ($null -eq $metadata.resolve) {
    throw "cargo metadata did not return a resolved dependency graph"
}

$rootPackages = @(
    $metadata.packages | Where-Object {
        $_.name -eq "zorya" -and $null -eq $_.source
    }
)
if ($rootPackages.Count -ne 1) {
    throw "expected exactly one workspace zorya package; found $($rootPackages.Count)"
}

$rootPackage = $rootPackages[0]
$rootPackageId = [string]$rootPackage.id
if ([string]$rootPackage.version -ne $Version) {
    throw "requested version $Version does not match Cargo package version $($rootPackage.version)"
}

$nodesById = @{}
foreach ($node in $metadata.resolve.nodes) {
    $nodesById[[string]$node.id] = $node
}

$reachableIds = [System.Collections.Generic.HashSet[string]]::new()
$pendingIds = [System.Collections.Generic.Queue[string]]::new()
$pendingIds.Enqueue($rootPackageId)

while ($pendingIds.Count -gt 0) {
    $packageId = $pendingIds.Dequeue()
    if (-not $reachableIds.Add($packageId)) {
        continue
    }
    if (-not $nodesById.ContainsKey($packageId)) {
        throw "resolved package $packageId is missing its metadata node"
    }

    $node = $nodesById[$packageId]
    foreach ($dependency in $node.deps) {
        $includeDependency = $false
        foreach ($kind in $dependency.dep_kinds) {
            if ([string]$kind.kind -ne "dev") {
                $includeDependency = $true
                break
            }
        }
        if ($includeDependency) {
            $pendingIds.Enqueue([string]$dependency.pkg)
        }
    }
}

$resolvedPackages = @(
    $metadata.packages |
        Where-Object {
            $packageId = [string]$_.id
            $reachableIds.Contains($packageId)
        } |
        Sort-Object name, version, id
)

if ($resolvedPackages.Count -ne $reachableIds.Count) {
    throw "resolved package metadata is incomplete: expected $($reachableIds.Count), found $($resolvedPackages.Count)"
}

$dependencies = @(
    $resolvedPackages | Where-Object { [string]$_.id -ne $rootPackageId }
)

$rarogCommits = @(
    $resolvedPackages |
        Where-Object { [string]$_.source -like "git+https://github.com/StanleyLl0yd/rarog*" } |
        ForEach-Object {
            $source = [string]$_.source
            if ($source -notmatch '#([0-9a-fA-F]{40})$') {
                throw "Rarog package source does not expose an exact commit: $source"
            }
            $Matches[1].ToLowerInvariant()
        } |
        Sort-Object -Unique
)
if ($rarogCommits.Count -ne 1) {
    throw "expected exactly one resolved Rarog commit; found $($rarogCommits.Count)"
}
$rarogCommit = $rarogCommits[0]

$rustcVersion = (rustc --version | Out-String).Trim()
Assert-Success "rustc version"
$cargoVersion = (cargo --version | Out-String).Trim()
Assert-Success "cargo version"

$executable = Join-Path "target/release" "zorya.exe"
if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) {
    throw "release executable does not exist: $executable"
}

$versionOutput = (& $executable --version | Out-String).Trim()
Assert-Success "release executable version check"
$expectedVersionOutput = "Zorya $Version"
if ($versionOutput -ne $expectedVersionOutput) {
    throw "release executable reports '$versionOutput'; expected '$expectedVersionOutput'"
}

if (Test-Path -LiteralPath $DistDirectory) {
    Remove-Item -LiteralPath $DistDirectory -Recurse -Force
}
New-Item -ItemType Directory -Path $DistDirectory | Out-Null

$packageName = "zorya-$Version-windows-x86_64"
$stageDirectory = Join-Path $DistDirectory $packageName
$licenseDirectory = Join-Path $stageDirectory "THIRD_PARTY_LICENSES"
New-Item -ItemType Directory -Path $licenseDirectory -Force | Out-Null

Copy-Item -LiteralPath $executable -Destination (Join-Path $stageDirectory "zorya.exe")
Copy-Item -LiteralPath "LICENSE" -Destination (Join-Path $stageDirectory "LICENSE")
Copy-Item -LiteralPath "README-TECHNICAL-PREVIEW.md" -Destination (Join-Path $stageDirectory "README.md")

$buildInfo = @(
    "Zorya version: $Version",
    "Source commit: $CommitSha",
    "Target: $TargetTriple",
    "Build profile: release",
    "Rarog commit: $rarogCommit",
    "Rust compiler: $rustcVersion",
    "Cargo: $cargoVersion",
    "Resolved non-dev packages: $($resolvedPackages.Count)"
)
Set-Content -LiteralPath (Join-Path $stageDirectory "BUILD-INFO.txt") -Value $buildInfo -Encoding utf8

$index = [System.Collections.Generic.List[string]]::new()
$index.Add("# Third-party licenses")
$index.Add("")
$index.Add("This directory contains license and notice files for the non-development packages reachable from Zorya in Cargo's Windows x86-64 filtered resolve graph.")
$index.Add("")
$index.Add("Generated from cargo metadata --locked --filter-platform $TargetTriple.")
$index.Add("")

$directoryNames = @{}
foreach ($package in $dependencies) {
    $licenseExpression = [string]$package.license
    $licenseFile = [string]$package.license_file

    if ([string]::IsNullOrWhiteSpace($licenseExpression) -and [string]::IsNullOrWhiteSpace($licenseFile)) {
        throw "dependency $($package.name) $($package.version) has no declared license metadata"
    }

    $baseDirectoryName = "$($package.name)-$($package.version)" -replace '[^A-Za-z0-9._-]', '_'
    $packageDirectory = Split-Path -Parent ([string]$package.manifest_path)
    $candidates = [System.Collections.Generic.List[string]]::new()
    $overrideOrigin = $null

    if (-not [string]::IsNullOrWhiteSpace($licenseFile)) {
        $declaredLicense = if ([System.IO.Path]::IsPathRooted($licenseFile)) {
            $licenseFile
        } else {
            Join-Path $packageDirectory $licenseFile
        }
        Add-LicenseCandidate -Candidates $candidates -Path $declaredLicense
    }

    Get-ChildItem -LiteralPath $packageDirectory -File |
        Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE)([-._].*)?$' } |
        ForEach-Object {
            Add-LicenseCandidate -Candidates $candidates -Path $_.FullName
        }

    $source = [string]$package.source
    if ($source -like "git+https://github.com/StanleyLl0yd/rarog*") {
        $rarogRoot = (Resolve-Path (Join-Path $packageDirectory "../..")).Path
        Get-ChildItem -LiteralPath $rarogRoot -File |
            Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE)([-._].*)?$' } |
            ForEach-Object {
                Add-LicenseCandidate -Candidates $candidates -Path $_.FullName
            }
    }

    if ($candidates.Count -eq 0) {
        $overrideDirectory = Join-Path "third_party/licenses" $baseDirectoryName
        if (Test-Path -LiteralPath $overrideDirectory -PathType Container) {
            $overrideOrigin = Join-Path $overrideDirectory "ORIGIN.txt"
            if (-not (Test-Path -LiteralPath $overrideOrigin -PathType Leaf)) {
                throw "license override for $($package.name) $($package.version) is missing ORIGIN.txt"
            }

            Get-ChildItem -LiteralPath $overrideDirectory -File |
                Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE)([-._].*)?$' } |
                ForEach-Object {
                    Add-LicenseCandidate -Candidates $candidates -Path $_.FullName
                }
        }
    }

    if ($candidates.Count -eq 0) {
        throw "dependency $($package.name) $($package.version) has no discoverable license or notice text"
    }

    $directoryName = $baseDirectoryName
    $suffix = 2
    while ($directoryNames.ContainsKey($directoryName)) {
        $directoryName = "$baseDirectoryName-$suffix"
        $suffix += 1
    }
    $directoryNames[$directoryName] = $true

    $targetLicenseDirectory = Join-Path $licenseDirectory $directoryName
    New-Item -ItemType Directory -Path $targetLicenseDirectory | Out-Null

    foreach ($candidate in $candidates) {
        $target = Join-Path $targetLicenseDirectory ([System.IO.Path]::GetFileName($candidate))
        Copy-Item -LiteralPath $candidate -Destination $target -Force
    }
    if ($null -ne $overrideOrigin) {
        Copy-Item -LiteralPath $overrideOrigin -Destination (Join-Path $targetLicenseDirectory "ORIGIN.txt")
    }

    $displayLicense = if ([string]::IsNullOrWhiteSpace($licenseExpression)) {
        "license-file"
    } else {
        $licenseExpression
    }
    $displaySource = if ([string]::IsNullOrWhiteSpace($source)) {
        "workspace"
    } else {
        $source
    }
    $licenseEvidence = if ($null -eq $overrideOrigin) {
        "resolved package source"
    } else {
        "source-controlled verified override"
    }

    $index.Add("## $($package.name) $($package.version)")
    $index.Add("")
    $index.Add("- Declared license: $displayLicense")
    $index.Add("- Cargo source: $displaySource")
    $index.Add("- License evidence: $licenseEvidence")
    $index.Add("- License files: $directoryName/")
    $index.Add("")
}

$indexPath = Join-Path $licenseDirectory "README.md"
Set-Content -LiteralPath $indexPath -Value $index -Encoding utf8

$archivePath = Join-Path $DistDirectory "$packageName.zip"
Compress-Archive -Path $stageDirectory -DestinationPath $archivePath -CompressionLevel Optimal -Force

$hash = Get-FileHash -LiteralPath $archivePath -Algorithm SHA256
$hashPath = "$archivePath.sha256"
$hashLine = "$($hash.Hash.ToLowerInvariant())  $([System.IO.Path]::GetFileName($archivePath))"
Set-Content -LiteralPath $hashPath -Value $hashLine -Encoding ascii

Write-Output "package=$archivePath"
Write-Output "sha256=$hashPath"
