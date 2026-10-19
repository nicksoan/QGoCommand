[CmdletBinding()]
param(
    [string]$OutputDirectory = (Join-Path $PSScriptRoot "..\artifacts"),
    [string]$AppInstallerBaseUri = "https://qgostorage.blob.core.windows.net/downloads",
    [string]$PackageIdentityName = "9744e81a-c1d5-47aa-a94b-5e8127bb2575",
    [string]$CertificateThumbprint,
    [string]$Publisher,
    [string]$TimestampUrl = "http://time.certum.pl",
    [switch]$SkipBuild,
    [switch]$SkipSigning,
    [switch]$Upload,
    [string]$StorageAccount = "qgostorage",
    [string]$Container = "downloads"
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$cargoToml = Get-Content (Join-Path $repoRoot "Cargo.toml") -Raw
$versionMatch = [regex]::Match(
    $cargoToml,
    '(?ms)^\[workspace\.package\]\s*.*?^version\s*=\s*"(?<version>\d+\.\d+\.\d+)"'
)
if (-not $versionMatch.Success) {
    throw "Could not read workspace.package.version from Cargo.toml."
}

$version = [version]$versionMatch.Groups["version"].Value
$packageVersion = "$($version.Major).$($version.Minor).$($version.Build).0"
$baseUri = $AppInstallerBaseUri.TrimEnd("/")
$outputPath = [System.IO.Path]::GetFullPath($OutputDirectory)
$stagePath = Join-Path $outputPath ".msix-stage"
$packageName = "QGo_$($packageVersion)_x64.msix"
$packagePath = Join-Path $outputPath $packageName
$appInstallerName = "QGo.Installer.appinstaller"
$appInstallerPath = Join-Path $outputPath $appInstallerName
$checksumPath = Join-Path $outputPath "SHA256SUMS.txt"

$certificate = $null
if (-not $SkipSigning) {
    if (-not $CertificateThumbprint) {
        throw "CertificateThumbprint is required unless -SkipSigning is used."
    }

    $thumbprint = $CertificateThumbprint.Replace(" ", "")
    $certificate = Get-Item "Cert:\CurrentUser\My\$thumbprint" -ErrorAction SilentlyContinue
    if (-not $certificate) {
        $certificate = Get-Item "Cert:\LocalMachine\My\$thumbprint" -ErrorAction SilentlyContinue
    }
    if (-not $certificate) {
        throw "The signing certificate was not found in CurrentUser\My or LocalMachine\My."
    }

    if ($Publisher -and $Publisher -cne $certificate.Subject) {
        throw "Publisher must exactly match the signing certificate subject."
    }
    $Publisher = $certificate.Subject
}
elseif (-not $Publisher) {
    $Publisher = "CN=QGo Development"
}

New-Item -ItemType Directory -Force -Path $outputPath | Out-Null
if (Test-Path $stagePath) {
    Remove-Item -Recurse -Force $stagePath
}
New-Item -ItemType Directory -Force -Path (Join-Path $stagePath "Assets") | Out-Null

if (-not $SkipBuild) {
    Push-Location $repoRoot
    try {
        & cargo build --locked --release -p qgo-app --target x86_64-pc-windows-msvc
        if ($LASTEXITCODE -ne 0) {
            throw "cargo build failed with exit code $LASTEXITCODE."
        }
    }
    finally {
        Pop-Location
    }
}

$executablePath = Join-Path $repoRoot "target\x86_64-pc-windows-msvc\release\qgo.exe"
if (-not (Test-Path $executablePath)) {
    throw "Release executable not found at $executablePath."
}
Copy-Item $executablePath (Join-Path $stagePath "qgo.exe")

Add-Type -AssemblyName System.Drawing
$sourceLogo = Join-Path $repoRoot "qgo-app\assets\QGoLogoSmall.png"
function Write-ResizedPng {
    param(
        [Parameter(Mandatory)] [string]$Source,
        [Parameter(Mandatory)] [string]$Destination,
        [Parameter(Mandatory)] [int]$Size
    )

    $sourceImage = [System.Drawing.Image]::FromFile($Source)
    try {
        $bitmap = New-Object System.Drawing.Bitmap($Size, $Size)
        try {
            $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
            try {
                $graphics.InterpolationMode =
                    [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
                $graphics.DrawImage($sourceImage, 0, 0, $Size, $Size)
            }
            finally {
                $graphics.Dispose()
            }
            $bitmap.Save($Destination, [System.Drawing.Imaging.ImageFormat]::Png)
        }
        finally {
            $bitmap.Dispose()
        }
    }
    finally {
        $sourceImage.Dispose()
    }
}

Write-ResizedPng $sourceLogo (Join-Path $stagePath "Assets\StoreLogo.png") 50
Write-ResizedPng $sourceLogo (Join-Path $stagePath "Assets\Square44x44Logo.png") 44
Write-ResizedPng $sourceLogo (Join-Path $stagePath "Assets\Square150x150Logo.png") 150

$escapedPublisher = [System.Security.SecurityElement]::Escape($Publisher)
$manifest = Get-Content (Join-Path $PSScriptRoot "Package.appxmanifest.template") -Raw
$manifest = $manifest.
    Replace("{{IDENTITY_NAME}}", $PackageIdentityName).
    Replace("{{PUBLISHER}}", $escapedPublisher).
    Replace("{{VERSION}}", $packageVersion)
Set-Content (Join-Path $stagePath "AppxManifest.xml") $manifest -Encoding utf8

$sdkRoot = "${env:ProgramFiles(x86)}\Windows Kits\10\bin"
$sdkVersion = Get-ChildItem $sdkRoot -Directory |
    Where-Object { $_.Name -match '^\d+\.\d+\.\d+\.\d+$' } |
    Sort-Object { [version]$_.Name } -Descending |
    Where-Object { Test-Path (Join-Path $_.FullName "x64\makeappx.exe") } |
    Select-Object -First 1
if (-not $sdkVersion) {
    throw "Windows SDK makeappx.exe was not found."
}

$makeAppx = Join-Path $sdkVersion.FullName "x64\makeappx.exe"
& $makeAppx pack /d $stagePath /p $packagePath /o
if ($LASTEXITCODE -ne 0) {
    throw "makeappx failed with exit code $LASTEXITCODE."
}

if (-not $SkipSigning) {
    $signTool = Join-Path $sdkVersion.FullName "x64\signtool.exe"
    & $signTool sign /sha1 $certificate.Thumbprint /fd SHA256 /tr $TimestampUrl /td SHA256 $packagePath
    if ($LASTEXITCODE -ne 0) {
        throw "signtool failed with exit code $LASTEXITCODE."
    }
}

$appInstallerUri = "$baseUri/$appInstallerName"
$packageUri = "$baseUri/$packageName"
$appInstaller = Get-Content (Join-Path $PSScriptRoot "QGo.Installer.appinstaller.template") -Raw
$appInstaller = $appInstaller.
    Replace("{{IDENTITY_NAME}}", $PackageIdentityName).
    Replace("{{PUBLISHER}}", $escapedPublisher).
    Replace("{{VERSION}}", $packageVersion).
    Replace("{{APPINSTALLER_URI}}", $appInstallerUri).
    Replace("{{PACKAGE_URI}}", $packageUri)
Set-Content $appInstallerPath $appInstaller -Encoding utf8

$hash = (Get-FileHash $packagePath -Algorithm SHA256).Hash.ToLowerInvariant()
Set-Content $checksumPath "$hash  $packageName" -Encoding ascii

if ($Upload) {
    if (-not (Get-Command az -ErrorAction SilentlyContinue)) {
        throw "Azure CLI is required when -Upload is used."
    }

    $uploads = @(
        @{
            Path = $packagePath
            Name = $packageName
            ContentType = "application/msix"
            CacheControl = "public, max-age=31536000, immutable"
        },
        @{
            Path = $appInstallerPath
            Name = $appInstallerName
            ContentType = "application/appinstaller"
            CacheControl = "no-cache"
        },
        @{
            Path = $checksumPath
            Name = "SHA256SUMS.txt"
            ContentType = "text/plain"
            CacheControl = "no-cache"
        }
    )

    foreach ($upload in $uploads) {
        & az storage blob upload `
            --account-name $StorageAccount `
            --container-name $Container `
            --name $upload.Name `
            --file $upload.Path `
            --auth-mode login `
            --overwrite true `
            --content-type $upload.ContentType `
            --content-cache-control $upload.CacheControl `
            --only-show-errors
        if ($LASTEXITCODE -ne 0) {
            throw "Azure upload failed for $($upload.Name) with exit code $LASTEXITCODE."
        }
    }
}

Remove-Item -Recurse -Force $stagePath

Write-Host "Created $packagePath"
Write-Host "Created $appInstallerPath"
Write-Host "Created $checksumPath"
if ($SkipSigning) {
    Write-Warning "The MSIX is unsigned and is only suitable for package validation."
}
