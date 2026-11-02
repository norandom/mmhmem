#requires -Version 5
<#
.SYNOPSIS
Generate marketplace plugin packages and catalogs on Windows.

.EXAMPLE
.\build-marketplace.ps1 -Ref v2.0.0
.\build-marketplace.ps1 -ManifestsOnly
.\build-marketplace.ps1 -Release v2.0.0
#>
[CmdletBinding(DefaultParameterSetName = 'Build')]
param(
    [Parameter(ParameterSetName = 'Build')]
    [string]$Ref = 'main',

    [Parameter(ParameterSetName = 'Manifests')]
    [switch]$ManifestsOnly,

    [Parameter(Mandatory, ParameterSetName = 'Release')]
    [string]$Release
)

$ErrorActionPreference = 'Stop'
$ConfirmPreference = 'None'
. (Join-Path $PSScriptRoot 'lib.ps1')

switch ($PSCmdlet.ParameterSetName) {
    'Manifests' { New-LibPluginPackages }
    'Release'   { Update-LibMarketplaceRelease -Ref $Release }
    default     { New-LibMarketplace -Ref $Ref }
}
