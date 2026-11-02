$script:LibRoot = $PSScriptRoot
$script:LibSkillsRoot = Join-Path $PSScriptRoot "deploy/skills"
$script:LibMarketplaceConfig = Join-Path $PSScriptRoot ".marketplace/skills.json"
$script:LibMarketplaceTemplates = Join-Path $PSScriptRoot ".marketplace/templates"
$script:LibMarkerName = ".mmhmem-managed.json"
# Read one scalar from SKILL.md frontmatter. Folded/block values are returned
# as one line for use in generated JSON manifests.
function Get-LibSkillField {
    param(
        [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)][string]$Name
    )
    $value = ''
    $inBlock = $false
    foreach ($line in (Get-Content -LiteralPath $Path)) {
        if (-not $inBlock) {
            if ($line -match ('^' + [regex]::Escape($Name) + ':\s*(.*)$')) {
                $rest = $Matches[1] -replace '^[>|][-+]?\s*', ''
                if ([string]::IsNullOrEmpty($rest)) {
                    $inBlock = $true
                } else {
                    return ($rest -replace '^"(.*)"\s*$', '$1' -replace "^'(.*)'\s*$", '$1')
                }
            }
            continue
        }
        if ($line -match '^---\s*$' -or $line -match '^\S') { break }
        $trimmed = $line.TrimStart()
        $value = if ($value) { "$value $trimmed" } else { $trimmed }
    }
    return $value
}

function ConvertTo-LibJson {
    param([Parameter(Mandatory)]$Value)
    return (ConvertTo-Json -InputObject $Value -Compress -Depth 20)
}

function Expand-LibTemplate {
    param(
        [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)][System.Collections.IDictionary]$Replacements
    )
    $text = Get-Content -LiteralPath $Path -Raw
    foreach ($key in $Replacements.Keys) {
        $text = $text.Replace(('{{' + $key + '}}'), [string]$Replacements[$key])
    }
    if ($text -match '{{[A-Z_][A-Z_]*}}') {
        throw "Unresolved token $($Matches[0]) in template $Path"
    }
    return $text.TrimEnd("`r", "`n")
}

function Write-LibText {
    param([Parameter(Mandatory)][string]$Path, [Parameter(Mandatory)][string]$Text)
    $parent = Split-Path -Parent $Path
    New-Item -ItemType Directory -Path $parent -Force -Confirm:$false | Out-Null
    $encoding = New-Object Text.UTF8Encoding($false)
    [IO.File]::WriteAllText($Path, ($Text.TrimEnd("`r", "`n") + "`n"), $encoding)
}

function Assert-LibMarketplaceConfig {
    param([Parameter(Mandatory)]$Config)

    if ($Config -isnot [PSCustomObject]) {
        throw 'Marketplace config root must be a JSON object'
    }
    $pluginsProperty = $Config.PSObject.Properties | Where-Object { $_.Name -ceq 'plugins' } | Select-Object -First 1
    if ($null -eq $pluginsProperty -or $pluginsProperty.Value -isnot [System.Array]) {
        throw 'Marketplace config plugins must be an array'
    }
    $plugins = @($pluginsProperty.Value)
    if ($plugins.Count -eq 0) { throw 'Marketplace config plugins cannot be empty' }

    $seen = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::Ordinal)
    foreach ($plugin in $plugins) {
        if ($plugin -isnot [PSCustomObject]) { throw 'Each marketplace plugin must be a JSON object' }
        foreach ($field in @('name', 'displayName', 'shortDescription', 'category', 'defaultPrompt')) {
            $property = $plugin.PSObject.Properties | Where-Object { $_.Name -ceq $field } | Select-Object -First 1
            if ($null -eq $property -or $property.Value -isnot [string] -or [string]::IsNullOrWhiteSpace($property.Value)) {
                throw "Marketplace plugin field '$field' must be a non-empty string"
            }
        }
        $name = [string]$plugin.name
        if ($name -cnotmatch '^(?!.*(?:--|\.\.))[a-z0-9](?:[a-z0-9.-]*[a-z0-9])?$') {
            throw "Invalid portable marketplace plugin name: $name"
        }
        if ($name.Length -gt 64) { throw "Marketplace plugin name exceeds 64 characters: $name" }
        if (-not $seen.Add($name)) { throw "Duplicate marketplace plugin name: $name" }

        $keywordsProperty = $plugin.PSObject.Properties | Where-Object { $_.Name -ceq 'keywords' } | Select-Object -First 1
        if ($null -eq $keywordsProperty -or $keywordsProperty.Value -isnot [System.Array] -or @($keywordsProperty.Value).Count -eq 0) {
            throw "Marketplace plugin '$name' keywords must be a non-empty array"
        }
        foreach ($keyword in @($keywordsProperty.Value)) {
            if ($keyword -isnot [string] -or [string]::IsNullOrWhiteSpace($keyword)) {
                throw "Marketplace plugin '$name' keywords must contain only non-empty strings"
            }
        }
    }

    $configuredNames = @($plugins | ForEach-Object { [string]$_.name } | Sort-Object)
    $skillNames = @(Get-ChildItem -LiteralPath $script:LibSkillsRoot -Directory -ErrorAction SilentlyContinue |
        Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName 'SKILL.md') -PathType Leaf } |
        ForEach-Object { $_.Name } | Sort-Object)
    $difference = @(Compare-Object -ReferenceObject $skillNames -DifferenceObject $configuredNames -CaseSensitive)
    if ($difference.Count -ne 0) {
        $details = ($difference | ForEach-Object { "$($_.SideIndicator)$($_.InputObject)" }) -join ', '
        throw "Marketplace config names must exactly match deploy/skills skill directories: $details"
    }
}

function Get-LibMarketplaceConfig {
    if (-not (Test-Path -LiteralPath $script:LibMarketplaceConfig -PathType Leaf)) {
        throw "Missing marketplace config: $script:LibMarketplaceConfig"
    }
    $config = Get-Content -LiteralPath $script:LibMarketplaceConfig -Raw | ConvertFrom-Json
    Assert-LibMarketplaceConfig -Config $config
    return $config
}

function New-LibPluginPackages {
    $config = Get-LibMarketplaceConfig
    $pluginsRoot = Join-Path $script:LibRoot 'plugins'
    New-Item -ItemType Directory -Path $pluginsRoot -Force -Confirm:$false | Out-Null
    $pluginsRootFull = [IO.Path]::GetFullPath($pluginsRoot).TrimEnd('\', '/')
    foreach ($plugin in $config.plugins) {
        $name = [string]$plugin.name
        $source = Join-Path $script:LibSkillsRoot $name
        $skillFile = Join-Path $source 'SKILL.md'
        if (-not (Test-Path -LiteralPath $skillFile -PathType Leaf)) { throw "Marketplace skill missing: $skillFile" }

        $manifestName = Get-LibSkillField -Path $skillFile -Name 'name'
        $version = [string]$config.version
        $description = Get-LibSkillField -Path $skillFile -Name 'description'
        if ($name -cnotmatch '^[a-z0-9]+(-[a-z0-9]+)*$' -or $name.Length -gt 64) { throw "Invalid Agent Skills name: $name" }
        if ([string]::IsNullOrWhiteSpace($description) -or $description.Length -gt 1024) { throw "Invalid skill description: $name" }
        $frontmatter = Get-Content -LiteralPath $skillFile -Raw
        if ($frontmatter -notmatch '\A---\r?\n([\s\S]*?)\r?\n---\r?\n') { throw "Missing skill frontmatter: $name" }
        foreach ($line in ($Matches[1] -split '\r?\n')) {
            if ($line -match '^([a-zA-Z][a-zA-Z-]*):' -and $Matches[1] -notin @('name','description','license','compatibility','metadata','allowed-tools')) {
                throw "Unsupported skill field: $($Matches[1])"
            }
        }
        if ($manifestName -cne $name) { throw "$name folder/frontmatter name mismatch: $manifestName" }
        if ($version -notmatch '^\d+\.\d+\.\d+(?:[+-][0-9A-Za-z.-]+)?$') { throw "$name has invalid semver: $version" }

        $pluginRoot = Join-Path $pluginsRoot $name
        $stageRoot = Join-Path $pluginsRoot ('.' + $name + '.stage.' + [Guid]::NewGuid().ToString('N'))
        $backupRoot = Join-Path $pluginsRoot ('.' + $name + '.backup.' + [Guid]::NewGuid().ToString('N'))
        foreach ($workPath in @($pluginRoot, $stageRoot, $backupRoot)) {
            $workFull = [IO.Path]::GetFullPath($workPath)
            if ((Split-Path -Parent $workFull).TrimEnd('\', '/') -cne $pluginsRootFull) {
                throw "Unsafe plugin work path: $workFull"
            }
        }

        $oldMoved = $false
        try {
            $skillDestination = Join-Path $stageRoot "skills/$name"
            New-Item -ItemType Directory -Path (Split-Path -Parent $skillDestination) -Force -Confirm:$false | Out-Null
            Copy-Item -LiteralPath $source -Destination $skillDestination -Recurse -Force -Confirm:$false
            Remove-Item -LiteralPath (Join-Path $skillDestination "$name.skill") -Force -Confirm:$false -ErrorAction SilentlyContinue
            Remove-Item -LiteralPath (Join-Path $skillDestination $script:LibMarkerName) -Force -Confirm:$false -ErrorAction SilentlyContinue
            Get-ChildItem -LiteralPath $skillDestination -Filter '.DS_Store' -Recurse -Force -ErrorAction SilentlyContinue |
                Remove-Item -Force -Confirm:$false -ErrorAction SilentlyContinue

            $common = [ordered]@{
                NAME_JSON = ConvertTo-LibJson $name
                VERSION_JSON = ConvertTo-LibJson $version
                DESCRIPTION_JSON = ConvertTo-LibJson $description
                SOURCE_PATH = $name
            }
            $codex = [ordered]@{}
            foreach ($key in $common.Keys) { $codex[$key] = $common[$key] }
            $codex.DISPLAY_NAME_JSON = ConvertTo-LibJson ([string]$plugin.displayName)
            $codex.SHORT_DESCRIPTION_JSON = ConvertTo-LibJson ([string]$plugin.shortDescription)
            $codex.CATEGORY_JSON = ConvertTo-LibJson ([string]$plugin.category)
            $codex.KEYWORDS_JSON = ConvertTo-LibJson @($plugin.keywords)
            $codex.DEFAULT_PROMPT_JSON = ConvertTo-LibJson ([string]$plugin.defaultPrompt)

            $portableText = Expand-LibTemplate -Path (Join-Path $script:LibMarketplaceTemplates 'portable-plugin.json.tpl') -Replacements $codex
            Write-LibText -Path (Join-Path $stageRoot 'plugin.json') -Text $portableText
            $codexText = Expand-LibTemplate -Path (Join-Path $script:LibMarketplaceTemplates 'codex-plugin.json.tpl') -Replacements $codex
            Write-LibText -Path (Join-Path $stageRoot '.codex-plugin/plugin.json') -Text $codexText
            $claudeText = Expand-LibTemplate -Path (Join-Path $script:LibMarketplaceTemplates 'claude-plugin.json.tpl') -Replacements $common
            Write-LibText -Path (Join-Path $stageRoot '.claude-plugin/plugin.json') -Text $claudeText
            $geminiText = Expand-LibTemplate -Path (Join-Path $script:LibMarketplaceTemplates 'gemini-plugin.json.tpl') -Replacements $codex
            Write-LibText -Path (Join-Path $stageRoot '.gemini-plugin/plugin.json') -Text $geminiText
            foreach ($manifest in @('plugin.json', '.codex-plugin/plugin.json', '.claude-plugin/plugin.json', '.gemini-plugin/plugin.json')) {
                Get-Content -LiteralPath (Join-Path $stageRoot $manifest) -Raw | ConvertFrom-Json | Out-Null
            }

            if (Test-Path -LiteralPath $pluginRoot) {
                Move-Item -LiteralPath $pluginRoot -Destination $backupRoot -Confirm:$false
                $oldMoved = $true
            }
            Move-Item -LiteralPath $stageRoot -Destination $pluginRoot -Confirm:$false
            if ($oldMoved) { Remove-Item -LiteralPath $backupRoot -Recurse -Force -Confirm:$false }
        } catch {
            if ($oldMoved -and (Test-Path -LiteralPath $backupRoot)) {
                if (Test-Path -LiteralPath $pluginRoot) { Remove-Item -LiteralPath $pluginRoot -Recurse -Force -Confirm:$false }
                Move-Item -LiteralPath $backupRoot -Destination $pluginRoot -Confirm:$false
            }
            throw
        } finally {
            if (Test-Path -LiteralPath $stageRoot) { Remove-Item -LiteralPath $stageRoot -Recurse -Force -Confirm:$false }
        }
        Write-Host ("  plugin {0,-24} -> plugins/{0}" -f $name)
    }
}

function Assert-LibMarketplaceRef {
    param([Parameter(Mandatory)][string]$Ref)
    if ([string]::IsNullOrWhiteSpace($Ref) -or $Ref -match '\s') {
        throw "Marketplace ref must be non-empty and contain no whitespace: $Ref"
    }
}

function New-LibMarketplaceCatalogs {
    param([Parameter(Mandatory)][string]$Ref)
    Assert-LibMarketplaceRef -Ref $Ref
    $config = Get-LibMarketplaceConfig
    $codexEntries = @()
    $claudeEntries = @()
    $geminiEntries = @()
    foreach ($plugin in $config.plugins) {
        $common = [ordered]@{
            NAME_JSON = ConvertTo-LibJson ([string]$plugin.name)
            DISPLAY_NAME_JSON = ConvertTo-LibJson ([string]$plugin.displayName)
            SHORT_DESCRIPTION_JSON = ConvertTo-LibJson ([string]$plugin.shortDescription)
            CATEGORY_JSON = ConvertTo-LibJson ([string]$plugin.category)
            KEYWORDS_JSON = ConvertTo-LibJson @($plugin.keywords)
            REF_JSON = ConvertTo-LibJson $Ref
            SOURCE_PATH = [string]$plugin.name
        }
        $codexEntry = Expand-LibTemplate -Path (Join-Path $script:LibMarketplaceTemplates 'codex-marketplace-entry.json.tpl') -Replacements $common
        $claudeEntry = Expand-LibTemplate -Path (Join-Path $script:LibMarketplaceTemplates 'claude-marketplace-entry.json.tpl') -Replacements $common
        $geminiEntry = Expand-LibTemplate -Path (Join-Path $script:LibMarketplaceTemplates 'gemini-marketplace-entry.json.tpl') -Replacements $common
        $codexEntries += (($codexEntry -split '\r?\n' | ForEach-Object { "    $_" }) -join "`n")
        $claudeEntries += (($claudeEntry -split '\r?\n' | ForEach-Object { "    $_" }) -join "`n")
        $geminiEntries += (($geminiEntry -split '\r?\n' | ForEach-Object { "    $_" }) -join "`n")
    }
    $catalogReplacement = [ordered]@{ PLUGIN_ENTRIES = ($codexEntries -join ",`n") }
    $codexCatalog = Expand-LibTemplate -Path (Join-Path $script:LibMarketplaceTemplates 'codex-marketplace.json.tpl') -Replacements $catalogReplacement
    Write-LibText -Path (Join-Path $script:LibRoot '.agents/plugins/marketplace.json') -Text $codexCatalog
    $catalogReplacement.PLUGIN_ENTRIES = ($claudeEntries -join ",`n")
    $claudeCatalog = Expand-LibTemplate -Path (Join-Path $script:LibMarketplaceTemplates 'claude-marketplace.json.tpl') -Replacements $catalogReplacement
    Write-LibText -Path (Join-Path $script:LibRoot '.claude-plugin/marketplace.json') -Text $claudeCatalog
    $catalogReplacement.PLUGIN_ENTRIES = ($geminiEntries -join ",`n")
    $geminiCatalog = Expand-LibTemplate -Path (Join-Path $script:LibMarketplaceTemplates 'gemini-marketplace.json.tpl') -Replacements $catalogReplacement
    Write-LibText -Path (Join-Path $script:LibRoot '.gemini-plugin/marketplace.json') -Text $geminiCatalog
    Get-Content -LiteralPath (Join-Path $script:LibRoot '.agents/plugins/marketplace.json') -Raw | ConvertFrom-Json | Out-Null
    Get-Content -LiteralPath (Join-Path $script:LibRoot '.claude-plugin/marketplace.json') -Raw | ConvertFrom-Json | Out-Null
    Get-Content -LiteralPath (Join-Path $script:LibRoot '.gemini-plugin/marketplace.json') -Raw | ConvertFrom-Json | Out-Null
    Write-Host "  catalogs -> $Ref (Codex, Claude Code, Gemini)"
}

function New-LibMarketplace {
    param([Parameter(Mandatory)][string]$Ref)
    New-LibPluginPackages
    New-LibMarketplaceCatalogs -Ref $Ref
}

function Update-LibMarketplaceRelease {
    param([Parameter(Mandatory)][string]$Ref)
    Assert-LibMarketplaceRef -Ref $Ref
    & git -C $script:LibRoot rev-parse --verify --quiet "$Ref^{commit}" | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "Unknown release ref: $Ref" }
    $config = Get-LibMarketplaceConfig
    foreach ($plugin in $config.plugins) {
        $name = [string]$plugin.name
        foreach ($vendor in @('codex', 'claude', 'gemini')) {
            $spec = "${Ref}:plugins/$name/.$vendor-plugin/plugin.json"
            & git -C $script:LibRoot cat-file -e $spec
            if ($LASTEXITCODE -ne 0) { throw "$Ref does not contain the $vendor manifest for $name" }
        }
        $spec = "${Ref}:plugins/$name/plugin.json"
        & git -C $script:LibRoot cat-file -e $spec
        if ($LASTEXITCODE -ne 0) { throw "$Ref does not contain the root plugin.json for $name" }
    }
    New-LibMarketplaceCatalogs -Ref $Ref
}
