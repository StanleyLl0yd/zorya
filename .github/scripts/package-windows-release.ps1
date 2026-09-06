[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$')]
    [string]$Version,

    [string]$OutputDirectory = "dist"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$outputRoot = if ([System.IO.Path]::IsPathRooted($OutputDirectory)) {
    $OutputDirectory
} else {
    Join-Path $repoRoot $OutputDirectory
}

Push-Location $repoRoot
try {
    $metadataJson = & cargo metadata --locked --format-version 1
    if ($LASTEXITCODE -ne 0) {
        throw "cargo metadata failed"
    }
    $metadata = $metadataJson | ConvertFrom-Json

    $zoryaPackages = @(
        $metadata.packages |
            Where-Object { $_.name -eq "zorya" -and $null -eq $_.source }
    )
    if ($zoryaPackages.Count -ne 1) {
        throw "expected exactly one workspace package named zorya"
    }
    if ($zoryaPackages[0].version -ne $Version) {
        throw "requested version $Version does not match Cargo.toml version $($zoryaPackages[0].version)"
    }

    $hostLine = (& rustc -vV | Select-String '^host: ').Line
    if (-not $hostLine) {
        throw "could not determine Rust host target"
    }
    $hostTarget = $hostLine.Substring(6)
    if ($hostTarget -ne "x86_64-pc-windows-msvc") {
        throw "release packaging requires x86_64-pc-windows-msvc, got $hostTarget"
    }

    $binary = Join-Path $repoRoot "target\release\zorya.exe"
    if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
        throw "release binary is missing: $binary"
    }

    $releaseNotes = Join-Path $repoRoot "docs\releases\$Version.md"
    if (-not (Test-Path -LiteralPath $releaseNotes -PathType Leaf)) {
        throw "release notes are missing: $releaseNotes"
    }

    New-Item -ItemType Directory -Force -Path $outputRoot | Out-Null

    $packageName = "Zorya-$Version-windows-x64"
    $stage = Join-Path $outputRoot $packageName
    $archive = Join-Path $outputRoot "$packageName.zip"
    $checksum = "$archive.sha256"

    Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $archive -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $checksum -Force -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force -Path $stage | Out-Null

    Copy-Item -LiteralPath $binary -Destination (Join-Path $stage "zorya.exe")
    Copy-Item -LiteralPath (Join-Path $repoRoot "README.md") -Destination $stage
    Copy-Item -LiteralPath (Join-Path $repoRoot "SECURITY.md") -Destination $stage
    Copy-Item -LiteralPath (Join-Path $repoRoot "LICENSE") -Destination $stage
    Copy-Item -LiteralPath (Join-Path $repoRoot "Cargo.lock") -Destination $stage
    Copy-Item -LiteralPath $releaseNotes -Destination (Join-Path $stage "RELEASE-NOTES.md")

    $thirdPartyRoot = Join-Path $stage "THIRD_PARTY_LICENSES"
    New-Item -ItemType Directory -Force -Path $thirdPartyRoot | Out-Null

    $inventory = [System.Collections.Generic.List[string]]::new()
    $inventory.Add("# Third-party notices")
    $inventory.Add("")
    $inventory.Add("This Technical Preview includes third-party Rust packages. The inventory below is generated from the exact locked Cargo dependency graph used for the release. Discovered license and notice files are copied into THIRD_PARTY_LICENSES.")
    $inventory.Add("")

    foreach ($package in @($metadata.packages | Sort-Object name, version)) {
        if ($package.name -eq "zorya" -and $null -eq $package.source) {
            continue
        }

        $license = if ($null -ne $package.license -and "$($package.license)".Length -gt 0) {
            "$($package.license)"
        } else {
            "(not declared in Cargo metadata)"
        }
        $repository = if ($null -ne $package.repository -and "$($package.repository)".Length -gt 0) {
            "$($package.repository)"
        } else {
            "(not declared)"
        }
        $source = if ($null -ne $package.source -and "$($package.source)".Length -gt 0) {
            "$($package.source)"
        } else {
            "(workspace/git checkout)"
        }

        $safeName = ("$($package.name)-$($package.version)" -replace '[^A-Za-z0-9._-]', '_')
        $packageLicenseRoot = Join-Path $thirdPartyRoot $safeName
        $manifestRoot = Split-Path -Parent "$($package.manifest_path)"
        $licenseFiles = [System.Collections.Generic.Dictionary[string, string]]::new(
            [System.StringComparer]::OrdinalIgnoreCase
        )

        if ($null -ne $package.license_file -and "$($package.license_file)".Length -gt 0) {
            $declaredLicense = "$($package.license_file)"
            if (-not [System.IO.Path]::IsPathRooted($declaredLicense)) {
                $declaredLicense = Join-Path $manifestRoot $declaredLicense
            }
            if (Test-Path -LiteralPath $declaredLicense -PathType Leaf) {
                $licenseFiles[(Resolve-Path -LiteralPath $declaredLicense).Path] = (Split-Path -Leaf $declaredLicense)
            }
        }

        $scanRoot = $manifestRoot
        for ($depth = 0; $depth -lt 4; $depth++) {
            if (-not (Test-Path -LiteralPath $scanRoot -PathType Container)) {
                break
            }

            foreach ($candidate in @(Get-ChildItem -LiteralPath $scanRoot -File -ErrorAction SilentlyContinue)) {
                if ($candidate.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE)([-_.].*)?$') {
                    if (-not $licenseFiles.ContainsKey($candidate.FullName)) {
                        $licenseFiles[$candidate.FullName] = $candidate.Name
                    }
                }
            }

            if ((Test-Path -LiteralPath (Join-Path $scanRoot ".git") -PathType Container) -or
                (Test-Path -LiteralPath (Join-Path $scanRoot "Cargo.lock") -PathType Leaf)) {
                break
            }

            $parent = Split-Path -Parent $scanRoot
            if (-not $parent -or $parent -eq $scanRoot) {
                break
            }
            $scanRoot = $parent
        }

        $inventory.Add("## $($package.name) $($package.version)")
        $inventory.Add("")
        $inventory.Add("- License expression: $license")
        $inventory.Add("- Repository: $repository")
        $inventory.Add("- Cargo source: $source")

        if ($licenseFiles.Count -gt 0) {
            New-Item -ItemType Directory -Force -Path $packageLicenseRoot | Out-Null
            $copiedNames = [System.Collections.Generic.HashSet[string]]::new(
                [System.StringComparer]::OrdinalIgnoreCase
            )
            foreach ($entry in $licenseFiles.GetEnumerator() | Sort-Object Key) {
                $destinationName = $entry.Value
                if (-not $copiedNames.Add($destinationName)) {
                    $stem = [System.IO.Path]::GetFileNameWithoutExtension($destinationName)
                    $extension = [System.IO.Path]::GetExtension($destinationName)
                    $destinationName = "$stem-$($copiedNames.Count)$extension"
                    [void]$copiedNames.Add($destinationName)
                }
                Copy-Item -LiteralPath $entry.Key -Destination (Join-Path $packageLicenseRoot $destinationName)
            }
            $inventory.Add("- Included license/notice files: $($copiedNames.Count)")
        } else {
            $inventory.Add("- Included license/notice files: none discovered in the package checkout")
        }
        $inventory.Add("")
    }

    $inventoryPath = Join-Path $stage "THIRD_PARTY_NOTICES.md"
    Set-Content -LiteralPath $inventoryPath -Value $inventory -Encoding utf8

    Compress-Archive -LiteralPath $stage -DestinationPath $archive -CompressionLevel Optimal

    $hash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    $checksumLine = "$hash  $([System.IO.Path]::GetFileName($archive))"
    Set-Content -LiteralPath $checksum -Value $checksumLine -Encoding ascii

    Write-Host "Packaged $archive"
    Write-Host "SHA-256 $hash"

    if ($env:GITHUB_OUTPUT) {
        Add-Content -LiteralPath $env:GITHUB_OUTPUT -Value "archive=$archive"
        Add-Content -LiteralPath $env:GITHUB_OUTPUT -Value "checksum=$checksum"
        Add-Content -LiteralPath $env:GITHUB_OUTPUT -Value "package_name=$packageName"
        Add-Content -LiteralPath $env:GITHUB_OUTPUT -Value "sha256=$hash"
    }
}
finally {
    Pop-Location
}
