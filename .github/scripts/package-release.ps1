param(
    [Parameter(Mandatory = $true)]
    [string]$Version,

    [string]$TargetTriple = "x86_64-pc-windows-msvc",

    [string]$CommitSha = "",

    [string]$SourceCommitSha = "",

    [string]$DistDirectory = "dist",

    [switch]$LicensePreflight
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

function Get-LicenseEvidence {
    param([object]$Package)

    $licenseExpression = [string]$Package.license
    $licenseFile = [string]$Package.license_file

    if ([string]::IsNullOrWhiteSpace($licenseExpression) -and [string]::IsNullOrWhiteSpace($licenseFile)) {
        throw "dependency $($Package.name) $($Package.version) has no declared license metadata"
    }

    $baseDirectoryName = "$($Package.name)-$($Package.version)" -replace '[^A-Za-z0-9._-]', '_'
    $packageDirectory = Split-Path -Parent ([string]$Package.manifest_path)
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

    $source = [string]$Package.source
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
                throw "license override for $($Package.name) $($Package.version) is missing ORIGIN.txt"
            }

            Get-ChildItem -LiteralPath $overrideDirectory -File |
                Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE)([-._].*)?$' } |
                ForEach-Object {
                    Add-LicenseCandidate -Candidates $candidates -Path $_.FullName
                }
        }
    }

    if ($candidates.Count -eq 0) {
        throw "dependency $($Package.name) $($Package.version) has no discoverable license or notice text"
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
    $evidenceKind = if ($null -eq $overrideOrigin) {
        "resolved package source"
    } else {
        "source-controlled verified override"
    }

    [pscustomobject]@{
        Package = $Package
        BaseDirectoryName = $baseDirectoryName
        Candidates = $candidates
        OverrideOrigin = $overrideOrigin
        DisplayLicense = $displayLicense
        DisplaySource = $displaySource
        EvidenceKind = $evidenceKind
    }
}


function Assert-InventoryField {
    param(
        [string]$Name,
        [string]$Value
    )

    if ($Value.Contains("`t") -or $Value.Contains("`r") -or $Value.Contains("`n")) {
        throw "license inventory field $Name contains unsupported control characters"
    }
}

function Get-LowerSha256 {
    param([string]$Path)

    (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Get-TextSha256 {
    param([string]$Text)

    $sha256 = [System.Security.Cryptography.SHA256]::Create()
    try {
        $bytes = [System.Text.UTF8Encoding]::new($false).GetBytes($Text)
        $hashBytes = $sha256.ComputeHash($bytes)
        ([System.BitConverter]::ToString($hashBytes)).Replace("-", "").ToLowerInvariant()
    } finally {
        $sha256.Dispose()
    }
}

function Get-CanonicalLicenseInventory {
    param(
        [object[]]$Evidence,
        [string]$Target
    )

    $body = [System.Collections.Generic.List[string]]::new()
    $evidenceFileCount = 0

    foreach ($item in $Evidence) {
        $package = $item.Package
        $packageName = [string]$package.name
        $packageVersion = [string]$package.version
        $displayLicense = [string]$item.DisplayLicense
        $displaySource = [string]$item.DisplaySource
        $evidenceKind = [string]$item.EvidenceKind

        Assert-InventoryField -Name "package name" -Value $packageName
        Assert-InventoryField -Name "package version" -Value $packageVersion
        Assert-InventoryField -Name "declared license" -Value $displayLicense
        Assert-InventoryField -Name "Cargo source" -Value $displaySource
        Assert-InventoryField -Name "evidence kind" -Value $evidenceKind

        $body.Add("package`t$packageName`t$packageVersion`t$displayLicense`t$displaySource`t$evidenceKind")

        $filesByName = @{}
        $evidencePaths = [System.Collections.Generic.List[string]]::new()
        foreach ($candidate in $item.Candidates) {
            $evidencePaths.Add([string]$candidate)
        }
        if ($null -ne $item.OverrideOrigin) {
            $evidencePaths.Add([string]$item.OverrideOrigin)
        }

        foreach ($path in $evidencePaths) {
            $fileName = [System.IO.Path]::GetFileName($path)
            Assert-InventoryField -Name "evidence file name" -Value $fileName
            $fileHash = Get-LowerSha256 -Path $path

            if ($filesByName.ContainsKey($fileName)) {
                if ([string]$filesByName[$fileName] -ne $fileHash) {
                    throw "dependency $packageName $packageVersion has conflicting license evidence named $fileName"
                }
                continue
            }

            $filesByName[$fileName] = $fileHash
        }

        foreach ($entry in @($filesByName.GetEnumerator() | Sort-Object Name)) {
            $body.Add("file`t$packageName`t$packageVersion`t$displaySource`t$($entry.Name)`t$($entry.Value)")
            $evidenceFileCount += 1
        }
    }

    $lines = [System.Collections.Generic.List[string]]::new()
    $lines.Add("zorya-license-inventory-v1")
    $lines.Add("target`t$Target")
    $lines.Add("dependencies`t$($Evidence.Count)")
    $lines.Add("evidence-files`t$evidenceFileCount")
    foreach ($line in $body) {
        $lines.Add($line)
    }

    $text = ($lines -join "`n") + "`n"
    [pscustomobject]@{
        Lines = @($lines)
        Text = $text
        DependencyCount = $Evidence.Count
        EvidenceFileCount = $evidenceFileCount
        Sha256 = Get-TextSha256 -Text $text
    }
}

function Assert-LicenseInventoryBaseline {
    param(
        [object]$Inventory,
        [string]$Target
    )

    $baselinePath = Join-Path "third_party/licenses/inventory" "$Target.baseline"
    if (-not (Test-Path -LiteralPath $baselinePath -PathType Leaf)) {
        throw "license inventory baseline is missing: $baselinePath"
    }

    $baseline = @{}
    foreach ($line in Get-Content -LiteralPath $baselinePath) {
        if ([string]::IsNullOrWhiteSpace($line)) {
            continue
        }

        $parts = $line -split "=", 2
        if ($parts.Count -ne 2 -or [string]::IsNullOrWhiteSpace($parts[0])) {
            throw "invalid license inventory baseline line: $line"
        }
        if ($baseline.ContainsKey($parts[0])) {
            throw "duplicate license inventory baseline key: $($parts[0])"
        }
        $baseline[$parts[0]] = $parts[1]
    }

    foreach ($requiredKey in @("format", "target", "dependencies", "evidence-files", "sha256")) {
        if (-not $baseline.ContainsKey($requiredKey)) {
            throw "license inventory baseline is missing key: $requiredKey"
        }
    }

    if ([string]$baseline["format"] -ne "1") {
        throw "unsupported license inventory baseline format: $($baseline["format"])"
    }
    if ([string]$baseline["target"] -ne $Target) {
        throw "license inventory baseline target $($baseline["target"]) does not match $Target"
    }

    $expectedDependencies = 0
    if (-not [int]::TryParse([string]$baseline["dependencies"], [ref]$expectedDependencies)) {
        throw "invalid dependency count in license inventory baseline"
    }
    $expectedEvidenceFiles = 0
    if (-not [int]::TryParse([string]$baseline["evidence-files"], [ref]$expectedEvidenceFiles)) {
        throw "invalid evidence-file count in license inventory baseline"
    }
    $expectedSha256 = ([string]$baseline["sha256"]).ToLowerInvariant()
    if ($expectedSha256.Length -ne 64 -or $expectedSha256 -match '[^0-9a-f]') {
        throw "invalid SHA-256 in license inventory baseline"
    }

    Write-Output "license-inventory-dependencies=$($Inventory.DependencyCount)"
    Write-Output "license-inventory-evidence-files=$($Inventory.EvidenceFileCount)"
    Write-Output "license-inventory-sha256=$($Inventory.Sha256)"

    if (
        $Inventory.DependencyCount -ne $expectedDependencies -or
        $Inventory.EvidenceFileCount -ne $expectedEvidenceFiles -or
        $Inventory.Sha256 -ne $expectedSha256
    ) {
        Write-Host "expected license inventory: dependencies=$expectedDependencies evidence-files=$expectedEvidenceFiles sha256=$expectedSha256"
        Write-Host "actual license inventory: dependencies=$($Inventory.DependencyCount) evidence-files=$($Inventory.EvidenceFileCount) sha256=$($Inventory.Sha256)"
        throw "Windows release license inventory differs from the reviewed source-controlled baseline"
    }
}

if ($Version -notmatch '^[0-9]+[.][0-9]+[.][0-9]+(?:[-+][0-9A-Za-z.-]+)?\z') {
    throw "invalid release version: $Version"
}

if ([string]::IsNullOrWhiteSpace($CommitSha)) {
    $CommitSha = (git rev-parse HEAD | Out-String).Trim()
    Assert-Success "git rev-parse HEAD"
}
if ($CommitSha -notmatch '^[0-9a-fA-F]{40}$') {
    throw "invalid build checkout commit SHA: $CommitSha"
}
$CommitSha = $CommitSha.ToLowerInvariant()

if ([string]::IsNullOrWhiteSpace($SourceCommitSha)) {
    $SourceCommitSha = $CommitSha
}
if ($SourceCommitSha -notmatch '^[0-9a-fA-F]{40}$') {
    throw "invalid source head commit SHA: $SourceCommitSha"
}
$SourceCommitSha = $SourceCommitSha.ToLowerInvariant()

$actualCheckoutCommit = (git rev-parse HEAD | Out-String).Trim().ToLowerInvariant()
Assert-Success "git rev-parse HEAD"
if ($actualCheckoutCommit -ne $CommitSha) {
    throw "build checkout commit $actualCheckoutCommit does not match expected commit $CommitSha"
}

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
$licenseErrors = [System.Collections.Generic.List[string]]::new()
$licenseEvidence = @(
    foreach ($dependency in $dependencies) {
        try {
            Get-LicenseEvidence -Package $dependency
        } catch {
            $licenseErrors.Add($_.Exception.Message)
        }
    }
)

if ($licenseErrors.Count -ne 0) {
    foreach ($licenseError in $licenseErrors) {
        Write-Host "license-evidence-error: $licenseError"
    }
    throw "release license evidence is incomplete for $($licenseErrors.Count) dependency package(s)"
}

if ($licenseEvidence.Count -ne $dependencies.Count) {
    throw "license evidence is incomplete: expected $($dependencies.Count), found $($licenseEvidence.Count)"
}

$licenseInventory = Get-CanonicalLicenseInventory -Evidence $licenseEvidence -Target $TargetTriple
Assert-LicenseInventoryBaseline -Inventory $licenseInventory -Target $TargetTriple

if ($LicensePreflight) {
    Write-Output "license-preflight=success"
    Write-Output "resolved-non-dev-packages=$($resolvedPackages.Count)"
    Write-Output "dependency-license-records=$($licenseEvidence.Count)"
    exit 0
}

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
    "Build checkout commit: $CommitSha",
    "Source head commit: $SourceCommitSha",
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
foreach ($evidence in $licenseEvidence) {
    $package = $evidence.Package
    $baseDirectoryName = [string]$evidence.BaseDirectoryName
    $directoryName = $baseDirectoryName
    $suffix = 2

    while ($directoryNames.ContainsKey($directoryName)) {
        $directoryName = "$baseDirectoryName-$suffix"
        $suffix += 1
    }
    $directoryNames[$directoryName] = $true

    $targetLicenseDirectory = Join-Path $licenseDirectory $directoryName
    New-Item -ItemType Directory -Path $targetLicenseDirectory | Out-Null

    foreach ($candidate in $evidence.Candidates) {
        $target = Join-Path $targetLicenseDirectory ([System.IO.Path]::GetFileName([string]$candidate))
        Copy-Item -LiteralPath $candidate -Destination $target -Force
    }
    if ($null -ne $evidence.OverrideOrigin) {
        Copy-Item -LiteralPath $evidence.OverrideOrigin -Destination (Join-Path $targetLicenseDirectory "ORIGIN.txt")
    }

    $index.Add("## $($package.name) $($package.version)")
    $index.Add("")
    $index.Add("- Declared license: $($evidence.DisplayLicense)")
    $index.Add("- Cargo source: $($evidence.DisplaySource)")
    $index.Add("- License evidence: $($evidence.EvidenceKind)")
    $index.Add("- License files: $directoryName/")
    $index.Add("")
}

$indexPath = Join-Path $licenseDirectory "README.md"
Set-Content -LiteralPath $indexPath -Value $index -Encoding utf8

$inventoryPath = Join-Path $licenseDirectory "INVENTORY.tsv"
[System.IO.File]::WriteAllText(
    $inventoryPath,
    $licenseInventory.Text,
    [System.Text.UTF8Encoding]::new($false)
)

$archivePath = Join-Path $DistDirectory "$packageName.zip"
Compress-Archive -Path $stageDirectory -DestinationPath $archivePath -CompressionLevel Optimal -Force

$hash = Get-FileHash -LiteralPath $archivePath -Algorithm SHA256
$hashPath = "$archivePath.sha256"
$hashLine = "$($hash.Hash.ToLowerInvariant())  $([System.IO.Path]::GetFileName($archivePath))"
Set-Content -LiteralPath $hashPath -Value $hashLine -Encoding ascii

Write-Output "package=$archivePath"
Write-Output "sha256=$hashPath"
) {
        throw "invalid SHA-256 in license inventory baseline"
    }

    Write-Output "license-inventory-dependencies=$($Inventory.DependencyCount)"
    Write-Output "license-inventory-evidence-files=$($Inventory.EvidenceFileCount)"
    Write-Output "license-inventory-sha256=$($Inventory.Sha256)"

    if (
        $Inventory.DependencyCount -ne $expectedDependencies -or
        $Inventory.EvidenceFileCount -ne $expectedEvidenceFiles -or
        $Inventory.Sha256 -ne $expectedSha256
    ) {
        Write-Host "expected license inventory: dependencies=$expectedDependencies evidence-files=$expectedEvidenceFiles sha256=$expectedSha256"
        Write-Host "actual license inventory: dependencies=$($Inventory.DependencyCount) evidence-files=$($Inventory.EvidenceFileCount) sha256=$($Inventory.Sha256)"
        throw "Windows release license inventory differs from the reviewed source-controlled baseline"
    }
}

if ($Version -notmatch '^[0-9]+[.][0-9]+[.][0-9]+(?:[-+][0-9A-Za-z.-]+)?
    throw "invalid release version: $Version"
}

if ([string]::IsNullOrWhiteSpace($CommitSha)) {
    $CommitSha = (git rev-parse HEAD | Out-String).Trim()
    Assert-Success "git rev-parse HEAD"
}
if ($CommitSha -notmatch '^[0-9a-fA-F]{40}$') {
    throw "invalid build checkout commit SHA: $CommitSha"
}
$CommitSha = $CommitSha.ToLowerInvariant()

if ([string]::IsNullOrWhiteSpace($SourceCommitSha)) {
    $SourceCommitSha = $CommitSha
}
if ($SourceCommitSha -notmatch '^[0-9a-fA-F]{40}$') {
    throw "invalid source head commit SHA: $SourceCommitSha"
}
$SourceCommitSha = $SourceCommitSha.ToLowerInvariant()

$actualCheckoutCommit = (git rev-parse HEAD | Out-String).Trim().ToLowerInvariant()
Assert-Success "git rev-parse HEAD"
if ($actualCheckoutCommit -ne $CommitSha) {
    throw "build checkout commit $actualCheckoutCommit does not match expected commit $CommitSha"
}

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
$licenseErrors = [System.Collections.Generic.List[string]]::new()
$licenseEvidence = @(
    foreach ($dependency in $dependencies) {
        try {
            Get-LicenseEvidence -Package $dependency
        } catch {
            $licenseErrors.Add($_.Exception.Message)
        }
    }
)

if ($licenseErrors.Count -ne 0) {
    foreach ($licenseError in $licenseErrors) {
        Write-Host "license-evidence-error: $licenseError"
    }
    throw "release license evidence is incomplete for $($licenseErrors.Count) dependency package(s)"
}

if ($licenseEvidence.Count -ne $dependencies.Count) {
    throw "license evidence is incomplete: expected $($dependencies.Count), found $($licenseEvidence.Count)"
}

if ($LicensePreflight) {
    Write-Output "license-preflight=success"
    Write-Output "resolved-non-dev-packages=$($resolvedPackages.Count)"
    Write-Output "dependency-license-records=$($licenseEvidence.Count)"
    exit 0
}

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
    "Build checkout commit: $CommitSha",
    "Source head commit: $SourceCommitSha",
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
foreach ($evidence in $licenseEvidence) {
    $package = $evidence.Package
    $baseDirectoryName = [string]$evidence.BaseDirectoryName
    $directoryName = $baseDirectoryName
    $suffix = 2

    while ($directoryNames.ContainsKey($directoryName)) {
        $directoryName = "$baseDirectoryName-$suffix"
        $suffix += 1
    }
    $directoryNames[$directoryName] = $true

    $targetLicenseDirectory = Join-Path $licenseDirectory $directoryName
    New-Item -ItemType Directory -Path $targetLicenseDirectory | Out-Null

    foreach ($candidate in $evidence.Candidates) {
        $target = Join-Path $targetLicenseDirectory ([System.IO.Path]::GetFileName([string]$candidate))
        Copy-Item -LiteralPath $candidate -Destination $target -Force
    }
    if ($null -ne $evidence.OverrideOrigin) {
        Copy-Item -LiteralPath $evidence.OverrideOrigin -Destination (Join-Path $targetLicenseDirectory "ORIGIN.txt")
    }

    $index.Add("## $($package.name) $($package.version)")
    $index.Add("")
    $index.Add("- Declared license: $($evidence.DisplayLicense)")
    $index.Add("- Cargo source: $($evidence.DisplaySource)")
    $index.Add("- License evidence: $($evidence.EvidenceKind)")
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
) {
    throw "invalid release version: $Version"
}

if ([string]::IsNullOrWhiteSpace($CommitSha)) {
    $CommitSha = (git rev-parse HEAD | Out-String).Trim()
    Assert-Success "git rev-parse HEAD"
}
if ($CommitSha -notmatch '^[0-9a-fA-F]{40}$') {
    throw "invalid build checkout commit SHA: $CommitSha"
}
$CommitSha = $CommitSha.ToLowerInvariant()

if ([string]::IsNullOrWhiteSpace($SourceCommitSha)) {
    $SourceCommitSha = $CommitSha
}
if ($SourceCommitSha -notmatch '^[0-9a-fA-F]{40}$') {
    throw "invalid source head commit SHA: $SourceCommitSha"
}
$SourceCommitSha = $SourceCommitSha.ToLowerInvariant()

$actualCheckoutCommit = (git rev-parse HEAD | Out-String).Trim().ToLowerInvariant()
Assert-Success "git rev-parse HEAD"
if ($actualCheckoutCommit -ne $CommitSha) {
    throw "build checkout commit $actualCheckoutCommit does not match expected commit $CommitSha"
}

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
$licenseErrors = [System.Collections.Generic.List[string]]::new()
$licenseEvidence = @(
    foreach ($dependency in $dependencies) {
        try {
            Get-LicenseEvidence -Package $dependency
        } catch {
            $licenseErrors.Add($_.Exception.Message)
        }
    }
)

if ($licenseErrors.Count -ne 0) {
    foreach ($licenseError in $licenseErrors) {
        Write-Host "license-evidence-error: $licenseError"
    }
    throw "release license evidence is incomplete for $($licenseErrors.Count) dependency package(s)"
}

if ($licenseEvidence.Count -ne $dependencies.Count) {
    throw "license evidence is incomplete: expected $($dependencies.Count), found $($licenseEvidence.Count)"
}

if ($LicensePreflight) {
    Write-Output "license-preflight=success"
    Write-Output "resolved-non-dev-packages=$($resolvedPackages.Count)"
    Write-Output "dependency-license-records=$($licenseEvidence.Count)"
    exit 0
}

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
    "Build checkout commit: $CommitSha",
    "Source head commit: $SourceCommitSha",
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
foreach ($evidence in $licenseEvidence) {
    $package = $evidence.Package
    $baseDirectoryName = [string]$evidence.BaseDirectoryName
    $directoryName = $baseDirectoryName
    $suffix = 2

    while ($directoryNames.ContainsKey($directoryName)) {
        $directoryName = "$baseDirectoryName-$suffix"
        $suffix += 1
    }
    $directoryNames[$directoryName] = $true

    $targetLicenseDirectory = Join-Path $licenseDirectory $directoryName
    New-Item -ItemType Directory -Path $targetLicenseDirectory | Out-Null

    foreach ($candidate in $evidence.Candidates) {
        $target = Join-Path $targetLicenseDirectory ([System.IO.Path]::GetFileName([string]$candidate))
        Copy-Item -LiteralPath $candidate -Destination $target -Force
    }
    if ($null -ne $evidence.OverrideOrigin) {
        Copy-Item -LiteralPath $evidence.OverrideOrigin -Destination (Join-Path $targetLicenseDirectory "ORIGIN.txt")
    }

    $index.Add("## $($package.name) $($package.version)")
    $index.Add("")
    $index.Add("- Declared license: $($evidence.DisplayLicense)")
    $index.Add("- Cargo source: $($evidence.DisplaySource)")
    $index.Add("- License evidence: $($evidence.EvidenceKind)")
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
