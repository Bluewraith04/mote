# Ensures an idempotent ad-hoc Authenticode CodeSigning certificate exists and is trusted.
# Default store scope is CurrentUser (no elevation required); pass -MachineStore for LocalMachine (elevated).
param(
    [string]$Subject = "CN=Mote Local Dev",
    [switch]$MachineStore
)

$ErrorActionPreference = "Stop"

$myStore = if ($MachineStore) { "Cert:\LocalMachine\My" } else { "Cert:\CurrentUser\My" }
$rootStore = if ($MachineStore) { "Cert:\LocalMachine\Root" } else { "Cert:\CurrentUser\Root" }
$tpStore = if ($MachineStore) { "Cert:\LocalMachine\TrustedPublisher" } else { "Cert:\CurrentUser\TrustedPublisher" }

$now = Get-Date
$cert = Get-ChildItem -Path $myStore | Where-Object {
    $_.Subject -eq $Subject -and
    $_.HasPrivateKey -and
    $_.NotAfter -gt $now.AddDays(7) -and
    ($_.EnhancedKeyUsageList | Where-Object { $_.ObjectId -eq "1.3.6.1.5.5.7.3.3" })
} | Sort-Object NotAfter -Descending | Select-Object -First 1

if (-not $cert) {
    $cert = New-SelfSignedCertificate `
        -Type CodeSigningCert `
        -Subject $Subject `
        -FriendlyName "Mote Local Dev Code Signing" `
        -CertStoreLocation $myStore `
        -KeyAlgorithm RSA `
        -KeyLength 2048 `
        -HashAlgorithm SHA256 `
        -NotAfter $now.AddYears(5)
}

$thumb = $cert.Thumbprint

function Ensure-CertInStore([string]$TargetStore, $SourceCert) {
    $exists = Get-ChildItem -Path $TargetStore | Where-Object { $_.Thumbprint -eq $SourceCert.Thumbprint } | Select-Object -First 1
    if (-not $exists) {
        $tmpCer = Join-Path ([System.IO.Path]::GetTempPath()) ("mote-dev-" + $SourceCert.Thumbprint + ".cer")
        try {
            $null = Export-Certificate -Cert $SourceCert -FilePath $tmpCer -Force
            $null = Import-Certificate -FilePath $tmpCer -CertStoreLocation $TargetStore
        } finally {
            Remove-Item -Force -ErrorAction SilentlyContinue $tmpCer
        }
    }
}

Ensure-CertInStore $rootStore $cert
Ensure-CertInStore $tpStore $cert

Write-Output $thumb
