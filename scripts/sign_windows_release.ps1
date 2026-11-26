param(
    [string]$Thumbprint = $env:LS_SIGNING_THUMBPRINT,
    [string]$TimestampUrl = 'http://timestamp.digicert.com'
)

$ErrorActionPreference = 'Stop'
if ($Thumbprint -notmatch '\A[0-9a-fA-F]{40}\z') {
    throw 'Set LS_SIGNING_THUMBPRINT to the code-signing certificate thumbprint.'
}
$certificate = Get-Item -LiteralPath "Cert:\CurrentUser\My\$Thumbprint" -ErrorAction SilentlyContinue
if (!$certificate -or !$certificate.HasPrivateKey) {
    throw 'The signing account needs the configured certificate and access to its private key.'
}
$signtool = Get-Command signtool.exe -CommandType Application -ErrorAction SilentlyContinue |
    Select-Object -First 1 -ExpandProperty Source
if (!$signtool) {
    $sdkBin = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
    $signtool = Get-ChildItem -LiteralPath $sdkBin -Directory -ErrorAction SilentlyContinue |
        Sort-Object Name -Descending |
        ForEach-Object { Join-Path $_.FullName 'x64\signtool.exe' } |
        Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } |
        Select-Object -First 1
}
if (!$signtool) {
    throw 'SignTool from the Windows SDK is required on the signing runner.'
}

function Sign-AndVerify([string]$File) {
    & $signtool sign /s My /sha1 $Thumbprint /fd SHA256 /tr $TimestampUrl /td SHA256 $File
    if ($LASTEXITCODE -ne 0) { throw "Signing failed for $(Split-Path $File -Leaf)." }
    & $signtool verify /pa /all /tw $File
    if ($LASTEXITCODE -ne 0) { throw "SignTool verification failed for $(Split-Path $File -Leaf)." }
    $signature = Get-AuthenticodeSignature -LiteralPath $File
    if ($signature.Status -ne 'Valid' -or !$signature.TimeStamperCertificate -or
        $signature.SignerCertificate.Thumbprint -ne $Thumbprint) {
        throw "Expected a valid, timestamped signature for $(Split-Path $File -Leaf)."
    }
}

$root = Split-Path -Parent $PSScriptRoot
$dist = Join-Path $root 'Resources\dist'
$stage = Join-Path $dist 'rust\labsentinel-server'
$version = (Get-Content -LiteralPath (Join-Path $root 'VERSION') -Raw).Trim()
$stageVersion = (Get-Content -LiteralPath (Join-Path $stage 'VERSION') -Raw).Trim()
if ($version -notmatch '\A\d+\.\d+\.\d+\z' -or $stageVersion -ne $version) {
    throw 'The release and staging versions must match.'
}
$server = (Get-Item -LiteralPath (Join-Path $stage 'labsentinel-server.exe')).FullName
$installer = Join-Path $dist "LabSentinel-setup-$version.exe"
$zipPattern = "labsentinel-rust-$version-*.zip"
if ((Test-Path -LiteralPath $installer) -or (Get-ChildItem -LiteralPath $dist -Filter $zipPattern)) {
    throw 'Existing release files for this version must be moved before signing.'
}

Sign-AndVerify $server
Push-Location $PSScriptRoot
try {
    python -B -c "from build_rust import zip_it; zip_it()"
    if ($LASTEXITCODE -ne 0) { throw 'ZIP assembly failed.' }
    python -B build_setup_exe.py
    if ($LASTEXITCODE -ne 0) { throw 'Installer assembly failed.' }
} finally {
    Pop-Location
}
Sign-AndVerify $installer

$zip = @(Get-ChildItem -LiteralPath $dist -Filter $zipPattern -File)
if ($zip.Count -ne 1) { throw 'Expected one signed-payload ZIP.' }
$serverHash = (Get-FileHash -LiteralPath $server -Algorithm SHA256).Hash
$archive = [System.IO.Compression.ZipFile]::OpenRead($zip[0].FullName)
try {
    $entry = $archive.GetEntry('labsentinel-server.exe')
    if (!$entry) { throw 'ZIP is missing labsentinel-server.exe.' }
    $stream = $entry.Open()
    try {
        if ((Get-FileHash -InputStream $stream -Algorithm SHA256).Hash -ne $serverHash) {
            throw 'ZIP executable differs from the signed staging file.'
        }
    } finally {
        $stream.Dispose()
    }
} finally {
    $archive.Dispose()
}
$sums = foreach ($artifact in @($zip[0].FullName, $installer)) {
    "$((Get-FileHash -LiteralPath $artifact -Algorithm SHA256).Hash.ToLowerInvariant())  $(Split-Path $artifact -Leaf)"
}
$sums | Set-Content -LiteralPath (Join-Path $dist 'SHA256SUMS-rust.txt') -Encoding ascii
