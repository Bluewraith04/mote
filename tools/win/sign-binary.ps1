# Signs a PE binary (.exe/.dll) with the Mote Local Dev certificate and verifies Signature.Status == Valid.
param(
    [Parameter(Mandatory = $true, Position = 0)][string]$Path,
    [string]$Subject = "CN=Mote Local Dev",
    [switch]$MachineStore,
    [string]$TimestampServer = ""
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path $Path)) {
    throw "File not found: $Path"
}

$fullPath = (Resolve-Path $Path).Path
$thumb = (& "$PSScriptRoot\ensure-dev-cert.ps1" -Subject $Subject -MachineStore:$MachineStore | Select-Object -Last 1).Trim()
$myStore = if ($MachineStore) { "Cert:\LocalMachine\My" } else { "Cert:\CurrentUser\My" }
$cert = Get-Item (Join-Path $myStore $thumb)

$signArgs = @{
    FilePath = $fullPath
    Certificate = $cert
    HashAlgorithm = "SHA256"
}
if ($TimestampServer) {
    $signArgs["TimestampServer"] = $TimestampServer
}

$res = Set-AuthenticodeSignature @signArgs
if ($res.Status -ne "Valid") {
    throw "Set-AuthenticodeSignature failed for ${fullPath}: $($res.Status) - $($res.StatusMessage)"
}

$check = Get-AuthenticodeSignature -FilePath $fullPath
if ($check.Status -ne "Valid") {
    throw "Signature verification failed for ${fullPath}: $($check.Status) - $($check.StatusMessage)"
}

Write-Output "signed: $fullPath ($($check.Status), thumbprint=$thumb)"
