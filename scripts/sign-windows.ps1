param(
    [Parameter(Mandatory = $true)]
    [string]$FilePath,
    [switch]$VerifyOnly
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$resolved = (Resolve-Path -LiteralPath $FilePath).Path
if ([string]::IsNullOrWhiteSpace($env:WINDOWS_PUBLISHER_NAME)) {
    throw 'WINDOWS_PUBLISHER_NAME is required; see docs/RELEASING.md.'
}

if (-not $VerifyOnly) {
    foreach ($name in @('ARTIFACT_SIGNING_ENDPOINT', 'ARTIFACT_SIGNING_ACCOUNT', 'ARTIFACT_SIGNING_PROFILE')) {
        if ([string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($name))) {
            throw "Missing release signing configuration: $name"
        }
    }
    Import-Module ArtifactSigning -RequiredVersion 0.1.20
    # azure/login obtains an OIDC-backed CLI session immediately before bundling.
    # Use only that session; never discover a developer's ambient credentials.
    $parameters = @{
        Endpoint = $env:ARTIFACT_SIGNING_ENDPOINT
        CodeSigningAccountName = $env:ARTIFACT_SIGNING_ACCOUNT
        CertificateProfileName = $env:ARTIFACT_SIGNING_PROFILE
        Files = $resolved
        FileDigest = 'SHA256'
        TimestampRfc3161 = 'http://timestamp.acs.microsoft.com'
        TimestampDigest = 'SHA256'
        ExcludeEnvironmentCredential = $true
        ExcludeWorkloadIdentityCredential = $true
        ExcludeManagedIdentityCredential = $true
        ExcludeSharedTokenCacheCredential = $true
        ExcludeVisualStudioCredential = $true
        ExcludeVisualStudioCodeCredential = $true
        ExcludeAzureCliCredential = $false
        ExcludeAzurePowerShellCredential = $true
        ExcludeAzureDeveloperCliCredential = $true
        ExcludeInteractiveBrowserCredential = $true
    }
    Invoke-ArtifactSigning @parameters
}

$signature = Get-AuthenticodeSignature -LiteralPath $resolved
if ($signature.Status -ne 'Valid' -or $null -eq $signature.SignerCertificate) {
    throw "Authenticode verification failed for $resolved : $($signature.Status)"
}
$publisher = $signature.SignerCertificate.GetNameInfo(
    [System.Security.Cryptography.X509Certificates.X509NameType]::SimpleName, $false
)
if ($publisher -cne $env:WINDOWS_PUBLISHER_NAME) {
    throw "Unexpected signing publisher for $resolved : $publisher"
}
if ($null -eq $signature.TimeStamperCertificate) {
    throw "Missing trusted timestamp for $resolved"
}
Write-Host "Verified Authenticode signature and timestamp: $resolved ($publisher)"
