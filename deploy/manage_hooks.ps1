# Show, install, remove and report mmhmem's read-routing hook entry for one harness (Windows).
# Usage: manage_hooks.ps1 <show|install|remove|status> -Harness claude|codex -Binary <abs mmhmem.exe>
#        [-SettingsPath <file>] [-ManagedSettingsPath <file>] [-DryRun]
# Claude Code: ~/.claude/settings.json (or CLAUDE_CONFIG_DIR). Codex: CODEX_HOME/hooks.json; its
# config.toml and the managed requirements.toml are only read, and Codex's trust state is never written.
# Edits are textual splices on a position-tracking JSON parse, so every byte outside
# mmhmem's own entry stays as it was and removal undoes an install byte for byte.
param(
    [Parameter(Position = 0, Mandatory = $true)][ValidateSet('show', 'install', 'remove', 'status')][string]$Command,
    [ValidateSet('claude', 'codex')][string]$Harness = 'claude',
    [string]$Binary,
    [string]$SettingsPath,
    [string]$ManagedSettingsPath,
    [switch]$DryRun
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2

function Write-Line([string]$Text) { [Console]::Out.WriteLine($Text) }
function Stop-Helper([string]$Message) { [Console]::Error.WriteLine("[mmhmem] $Message"); exit 1 }

# --- position-tracking JSON parser -----------------------------------------------
# Nodes: K = o (object), a (array), s, n, b, z (null); S/E = start / exclusive end.
$script:T = ''; $script:P = 0
$script:StringToken = [regex]'\G"(?:[^"\\\x00-\x1f]|\\(?:["\\/bfnrt]|u[0-9a-fA-F]{4}))*"'
$script:NumberToken = [regex]'\G-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?'
function Skip-JsonSpace { while ($script:P -lt $script:T.Length -and " `t`r`n".IndexOf($script:T[$script:P]) -ge 0) { $script:P++ } }
function ConvertFrom-JsonToken([string]$Token) {
    $inner = $Token.Substring(1, $Token.Length - 2)
    return [regex]::Replace($inner, '\\(u[0-9a-fA-F]{4}|.)', {
            param($m)
            $c = $m.Groups[1].Value
            switch -CaseSensitive ($c) {
                'b' { return [string][char]8 } 'f' { return [string][char]12 } 'n' { return "`n" }
                'r' { return "`r" } 't' { return "`t" }
                default { if ($c.Length -eq 5) { return [string][char][Convert]::ToInt32($c.Substring(1), 16) } return $c }
            }
        })
}
function Read-JsonString {
    $m = $script:StringToken.Match($script:T, $script:P)
    if (-not $m.Success) { throw "invalid string at offset $($script:P)" }
    $node = @{ K = 's'; S = $script:P; E = $script:P + $m.Length; V = (ConvertFrom-JsonToken $m.Value) }
    $script:P += $m.Length
    return $node
}
function Read-JsonValue {
    Skip-JsonSpace
    if ($script:P -ge $script:T.Length) { throw 'unexpected end of file' }
    $start = $script:P; $c = $script:T[$start]
    if ($c -eq '{') {
        # Ordinal: JSON keys are case-sensitive ("Hooks" and "hooks" differ).
        $script:P++; $members = New-Object Collections.Generic.List[object]; $keys = New-Object Collections.Hashtable([StringComparer]::Ordinal)
        Skip-JsonSpace
        if ($script:P -lt $script:T.Length -and $script:T[$script:P] -eq '}') { $script:P++ }
        else {
            while ($true) {
                Skip-JsonSpace
                if ($script:P -ge $script:T.Length -or $script:T[$script:P] -ne '"') { throw "expected a key at offset $($script:P)" }
                $key = Read-JsonString
                Skip-JsonSpace
                if ($script:P -ge $script:T.Length -or $script:T[$script:P] -ne ':') { throw "expected ':' at offset $($script:P)" }
                $script:P++
                $value = Read-JsonValue
                if ($keys.ContainsKey($key.V)) { $keys[$key.V]++ } else { $keys[$key.V] = 1 }
                $members.Add(@{ Key = $key.V; S = $key.S; E = $value.E; V = $value })
                Skip-JsonSpace
                if ($script:P -lt $script:T.Length -and $script:T[$script:P] -eq ',') { $script:P++; continue }
                if ($script:P -lt $script:T.Length -and $script:T[$script:P] -eq '}') { $script:P++; break }
                throw "expected ',' or '}' at offset $($script:P)"
            }
        }
        return @{ K = 'o'; S = $start; E = $script:P; M = $members; Count = $keys }
    }
    if ($c -eq '[') {
        $script:P++; $items = New-Object Collections.Generic.List[object]
        Skip-JsonSpace
        if ($script:P -lt $script:T.Length -and $script:T[$script:P] -eq ']') { $script:P++ }
        else {
            while ($true) {
                $items.Add((Read-JsonValue))
                Skip-JsonSpace
                if ($script:P -lt $script:T.Length -and $script:T[$script:P] -eq ',') { $script:P++; continue }
                if ($script:P -lt $script:T.Length -and $script:T[$script:P] -eq ']') { $script:P++; break }
                throw "expected ',' or ']' at offset $($script:P)"
            }
        }
        return @{ K = 'a'; S = $start; E = $script:P; I = $items }
    }
    if ($c -eq '"') { return Read-JsonString }
    foreach ($literal in @(@('true', 'b', $true), @('false', 'b', $false), @('null', 'z', $null))) {
        if ([string]::CompareOrdinal($script:T, $start, $literal[0], 0, $literal[0].Length) -eq 0) {
            $script:P += $literal[0].Length
            return @{ K = $literal[1]; S = $start; E = $script:P; V = $literal[2] }
        }
    }
    $m = $script:NumberToken.Match($script:T, $start)
    if (-not $m.Success) { throw "unexpected character at offset $start" }
    $script:P += $m.Length
    return @{ K = 'n'; S = $start; E = $script:P; V = $m.Value }
}
function ConvertFrom-JsonText([string]$Text) {
    $script:T = $Text; $script:P = 0
    $root = Read-JsonValue
    Skip-JsonSpace
    if ($script:P -ne $Text.Length) { throw "unexpected content at offset $($script:P)" }
    return $root
}
# Last member wins, as in JSON.parse; callers that edit a key refuse duplicates.
function Get-JsonMember($Object, [string]$Key) {
    if ($null -eq $Object -or $Object.K -ne 'o') { return $null }
    for ($i = $Object.M.Count - 1; $i -ge 0; $i--) { if ($Object.M[$i].Key -ceq $Key) { return $Object.M[$i] } }
    return $null
}
function Assert-SingleKey($Object, [string]$Key, [string]$Where) {
    if ($Object.Count.ContainsKey($Key) -and $Object.Count[$Key] -gt 1) { throw "duplicate key '$Key' in $Where" }
}

# --- JSON writer matching JSON.stringify(value, null, unit) ----------------------
function ConvertTo-JsonString([string]$Value) {
    $sb = New-Object Text.StringBuilder; [void]$sb.Append('"')
    foreach ($ch in $Value.ToCharArray()) {
        switch ([int]$ch) {
            34 { [void]$sb.Append('\"') } 92 { [void]$sb.Append('\\') } 8 { [void]$sb.Append('\b') }
            12 { [void]$sb.Append('\f') } 10 { [void]$sb.Append('\n') } 13 { [void]$sb.Append('\r') } 9 { [void]$sb.Append('\t') }
            default { if ([int]$ch -lt 32) { [void]$sb.Append(('\u{0:x4}' -f [int]$ch)) } else { [void]$sb.Append($ch) } }
        }
    }
    [void]$sb.Append('"'); return $sb.ToString()
}
function ConvertTo-JsonText($Value, [string]$Indent, $Style) {
    if ($Value -is [Collections.Specialized.OrderedDictionary]) {
        if ($Value.Count -eq 0) { return '{}' }
        $inner = $Indent + $Style.Unit; $parts = @()
        foreach ($key in $Value.Keys) { $parts += (ConvertTo-JsonString $key) + $Style.Colon + (ConvertTo-JsonText $Value[$key] $inner $Style) }
        if (-not $Style.Pretty) { return '{' + ($parts -join ',') + '}' }
        return '{' + $Style.NL + $inner + ($parts -join (',' + $Style.NL + $inner)) + $Style.NL + $Indent + '}'
    }
    if ($Value -is [Array]) {
        if ($Value.Count -eq 0) { return '[]' }
        $inner = $Indent + $Style.Unit; $parts = @()
        foreach ($item in $Value) { $parts += ConvertTo-JsonText $item $inner $Style }
        if (-not $Style.Pretty) { return '[' + ($parts -join ',') + ']' }
        return '[' + $Style.NL + $inner + ($parts -join (',' + $Style.NL + $inner)) + $Style.NL + $Indent + ']'
    }
    if ($Value -is [string]) { return ConvertTo-JsonString $Value }
    if ($Value -is [bool]) { if ($Value) { return 'true' } return 'false' }
    return [string]$Value
}

# --- text splices that keep the surrounding formatting ----------------------------
function Get-Elements($Container) { if ($Container.K -eq 'o') { return , $Container.M } return , $Container.I }
function Get-LineIndent([string]$Text, [int]$Offset) {
    $lineStart = $Text.LastIndexOf("`n", [Math]::Max(0, $Offset - 1)) + 1
    if ($Offset -eq 0) { $lineStart = 0 }
    $m = [regex]::Match($Text.Substring($lineStart), '^[ \t]*')
    return $m.Value
}
function Get-FileStyle([string]$Text) {
    $nl = if ($Text.Contains("`r`n")) { "`r`n" } else { "`n" }
    $unit = '  '
    $m = [regex]::Match($Text, '[\{\[][ \t]*\r?\n([ \t]+)\S')
    if ($m.Success) { $unit = $m.Groups[1].Value.Substring((Get-LineIndent $Text $m.Index).Length); if (-not $unit) { $unit = '  ' } }
    return @{ NL = $nl; Unit = $unit; Pretty = $Text.Contains("`n"); Colon = ': ' }
}
# Appends an element (an object member when $Key is set) after the container's last element.
function Add-JsonElement([string]$Text, $Container, [string]$Key, $Value, $Style) {
    $elements = Get-Elements $Container
    $colonFor = { param($pretty) if ($pretty) { ': ' } else { ':' } }
    if ($elements.Count -gt 0) {
        $last = $elements[$elements.Count - 1]
        if ($elements.Count -ge 2) { $previous = $elements[$elements.Count - 2]; $separator = $Text.Substring($previous.E, $last.S - $previous.E) }
        else { $separator = ',' + $Text.Substring($Container.S + 1, $last.S - $Container.S - 1) }
        $pretty = $separator.Contains("`n")
        $indent = if ($pretty) { $separator.Substring($separator.LastIndexOf("`n") + 1) } else { '' }
        $local = @{ NL = $Style.NL; Unit = $Style.Unit; Pretty = $pretty; Colon = (& $colonFor $pretty) }
        $body = ConvertTo-JsonText $Value $indent $local
        if ($Key) { $body = (ConvertTo-JsonString $Key) + $local.Colon + $body }
        return $Text.Substring(0, $last.E) + $separator + $body + $Text.Substring($last.E)
    }
    $own = Get-LineIndent $Text $Container.S
    $pretty = $Style.Pretty
    $indent = if ($pretty) { $own + $Style.Unit } else { '' }
    $local = @{ NL = $Style.NL; Unit = $Style.Unit; Pretty = $pretty; Colon = (& $colonFor $pretty) }
    $body = ConvertTo-JsonText $Value $indent $local
    if ($Key) { $body = (ConvertTo-JsonString $Key) + $local.Colon + $body }
    if ($pretty) { $body = $Style.NL + $indent + $body + $Style.NL + $own }
    return $Text.Substring(0, $Container.S + 1) + $body + $Text.Substring($Container.E - 1)
}
# Removes one element with the separator that Add-JsonElement would have added for it.
function Remove-JsonElement([string]$Text, $Container, [int]$Index) {
    $elements = Get-Elements $Container
    if ($elements.Count -eq 1) { return $Text.Substring(0, $Container.S + 1) + $Text.Substring($Container.E - 1) }
    if ($Index -gt 0) { $from = $elements[$Index - 1].E; $to = $elements[$Index].E }
    else { $from = $elements[0].S; $to = $elements[1].S }
    return $Text.Substring(0, $from) + $Text.Substring($to)
}

# --- harness definitions (opencode is added by a later task) --------------------------
$script:Harnesses = @{
    claude = @{
        Label = 'Claude Code'
        Matcher = 'Read|Bash|PowerShell'
        Args = @('hook', 'claude')
        Timeout = 10
        DefaultManaged = 'C:\Program Files\ClaudeCode\managed-settings.json'
        PolicyRegistry = @('HKLM:\SOFTWARE\Policies\ClaudeCode', 'HKCU:\SOFTWARE\Policies\ClaudeCode')
        Approval = 'Claude Code holds back hooks until you accept the workspace trust dialog in an interactive session; review the entry with /hooks.'
        Sees = 'Before each Read, Bash or PowerShell tool call, Claude Code passes the hook the tool name and its input: the file path with any offset and limit for Read, or the command text for Bash and PowerShell. The hook looks only at that file''s size and line count; it declines (pointing the agent to mmhmem''s smart_read) only full reads of large files that are not security-critical, and only while an mmhmem worker is configured. Every other call, and any hook error or timeout, proceeds unchanged. It never returns or records file content, never reads secrets such as the worker API key, and makes no network connections.'
    }
    codex = @{
        Label = 'Codex'
        Matcher = '^(Bash|shell)$'
        Args = @('hook', 'codex')
        Timeout = 10
        # Verified in codex-cli 0.166.0: "Use %ProgramData%\OpenAI\Codex\requirements.toml for enforced settings".
        DefaultManaged = (Join-Path ([Environment]::GetFolderPath('CommonApplicationData')) 'OpenAI\Codex\requirements.toml')
        Approval = 'Codex runs a new or changed hook only after you trust it: start Codex and choose "Trust all and continue" at the "Hooks need review" prompt, or open /hooks and press t on the mmhmem entry. Until then Codex skips it (codex exec skips it silently). mmhmem never writes Codex''s trust state.'
        Reapprove = 'The entry changed, so Codex treats it as a new hook: approve it again in /hooks (re-approval is needed).'
        Sees = 'Before each shell command (Codex has no separate read tool), Codex passes the hook the shell command text and the working directory. The hook recognises only a single whole-file read such as cat, type or Get-Content of one file, looks only at that file''s size and line count, and declines (pointing the agent to mmhmem''s smart_read or a ranged read) only full reads of large files that are not security-critical, and only while an mmhmem worker is configured. Every other command, and any hook error or timeout, proceeds unchanged. It never returns or records file content, never reads secrets such as the worker API key, and makes no network connections.'
    }
}

function Get-UserConfigDir([string]$Name) {
    if ($Name -eq 'codex') { if ($env:CODEX_HOME) { return [IO.Path]::GetFullPath($env:CODEX_HOME) }; return Join-Path $HOME '.codex' }
    if ($env:CLAUDE_CONFIG_DIR) { return [IO.Path]::GetFullPath($env:CLAUDE_CONFIG_DIR) }
    return Join-Path $HOME '.claude'
}
function Get-Entry($Def, [string]$Bin) {
    if ($Def.Label -eq 'Codex') { return [ordered]@{ matcher = $Def.Matcher; hooks = @([ordered]@{ type = 'command'; command = (Get-CodexCommand $Bin); timeout = $Def.Timeout }) } }
    $handler = [ordered]@{ type = 'command'; command = $Bin; args = $Def.Args; timeout = $Def.Timeout }
    return [ordered]@{ matcher = $Def.Matcher; hooks = @($handler) }
}
function Test-OwnHandler($Handler, $Def) {
    if ($Handler.K -ne 'o') { return $false }
    if ($Def.Label -eq 'Codex') { return [bool](Get-CodexBinary $Handler) }
    $cmd = Get-JsonMember $Handler 'command'; $argList = Get-JsonMember $Handler 'args'
    if ($null -eq $cmd -or $cmd.V.K -ne 's' -or $null -eq $argList -or $argList.V.K -ne 'a') { return $false }
    $name = ($cmd.V.V -split '[\\/]')[-1]
    if ($name -ne 'mmhmem' -and $name -ne 'mmhmem.exe') { return $false }
    if ($argList.V.I.Count -ne $Def.Args.Count) { return $false }
    for ($i = 0; $i -lt $Def.Args.Count; $i++) { if ($argList.V.I[$i].K -ne 's' -or $argList.V.I[$i].V -cne $Def.Args[$i]) { return $false } }
    return $true
}
# Returns the hook containers and every identified handler; throws when the shape is not editable.
function Find-Own($Root, $Def) {
    if ($Root.K -ne 'o') { throw 'settings root is not a JSON object' }
    Assert-SingleKey $Root 'hooks' 'settings'
    $result = @{ Hooks = $null; Pre = $null; Found = New-Object Collections.Generic.List[object] }
    $hooks = Get-JsonMember $Root 'hooks'
    if ($null -eq $hooks) { return $result }
    if ($hooks.V.K -ne 'o') { throw '"hooks" is not an object' }
    Assert-SingleKey $hooks.V 'PreToolUse' 'hooks'
    $result.Hooks = $hooks
    $pre = Get-JsonMember $hooks.V 'PreToolUse'
    if ($null -eq $pre) { return $result }
    if ($pre.V.K -ne 'a') { throw '"hooks.PreToolUse" is not an array' }
    $result.Pre = $pre
    for ($g = 0; $g -lt $pre.V.I.Count; $g++) {
        $group = $pre.V.I[$g]
        $handlers = Get-JsonMember $group 'hooks'
        if ($null -eq $handlers -or $handlers.V.K -ne 'a') { continue }
        for ($h = 0; $h -lt $handlers.V.I.Count; $h++) {
            if (Test-OwnHandler $handlers.V.I[$h] $Def) { $result.Found.Add(@{ Group = $g; Handlers = $handlers.V; Index = $h; Node = $handlers.V.I[$h] }) }
        }
    }
    return $result
}
# Removes one identified handler from $Text, then its group, the PreToolUse array and the
# hooks object as each becomes empty. $Which selects the first or the last identified handler.
function Remove-OwnHandler([string]$Text, $Def, [ValidateSet('first', 'last')][string]$Which) {
    $root = ConvertFrom-JsonText $Text
    $own = Find-Own $root $Def
    $hit = if ($Which -eq 'first') { $own.Found[0] } else { $own.Found[$own.Found.Count - 1] }
    if ($hit.Handlers.I.Count -gt 1) { return Remove-JsonElement $Text $hit.Handlers $hit.Index }
    $pre = $own.Pre.V
    if ($pre.I.Count -gt 1) { return Remove-JsonElement $Text $pre $hit.Group }
    $hooks = $own.Hooks.V
    if ($hooks.M.Count -gt 1) { return Remove-JsonElement $Text $hooks $hooks.M.IndexOf($own.Pre) }
    return Remove-JsonElement $Text $root $root.M.IndexOf($own.Hooks)
}

# --- Codex: command form and read-only TOML checks (policy, trust state) --------------
# Codex runs a hook command through the user's shell (PowerShell on Windows, verified with
# codex-cli 0.166.0), so the path is single-quoted: PowerShell takes it literally.
function Get-CodexCommand([string]$Bin) { return "& '" + $Bin + "' hook codex" }
$script:CodexCommand = [regex]"^&\s*'([^']+)'\s+hook\s+codex$"
function Get-CodexBinary($Handler) {
    $cmd = Get-JsonMember $Handler 'command'
    if ($null -eq $cmd -or $cmd.V.K -ne 's') { return $null }
    $m = $script:CodexCommand.Match($cmd.V.V)
    if (-not $m.Success) { return $null }
    $name = ($m.Groups[1].Value -split '[\\/]')[-1]
    if ($name -ne 'mmhmem' -and $name -ne 'mmhmem.exe') { return $null }
    return $m.Groups[1].Value
}
# Line-based reading of the few keys checked here: table headers and key = value lines, with
# comments dropped outside quotes. Keys are returned with their table, whitespace removed.
$script:TomlKey = '(?:[A-Za-z0-9_-]+|"(?:[^"\\]|\\.)*"|''[^'']*'')'
function Convert-TomlKey([string]$Text) {
    if ($Text -notmatch ('^\s*' + $script:TomlKey + '(?:\s*\.\s*' + $script:TomlKey + ')*\s*$')) { return $null }
    $parts = @([regex]::Matches($Text, $script:TomlKey) | ForEach-Object {
        $token = $_.Value
        if ($token.StartsWith('"')) { (ConvertFrom-JsonText $token).V }
        elseif ($token.StartsWith("'")) { $token.Substring(1, $token.Length - 2) }
        else { $token }
    })
    return $parts -join '.'
}
function Test-InlineHooksOff([string]$Text) {
    foreach ($m in [regex]::Matches($Text, ('(?:^|[{,])\s*(' + $script:TomlKey + ')\s*=\s*false\b'))) {
        if ((Convert-TomlKey $m.Groups[1].Value) -ceq 'hooks') { return $true }
    }
    return $false
}
function Read-TomlLines([string]$Path) {
    $result = @{ Keys = New-Object Collections.Generic.List[object]; Tables = New-Object Collections.Generic.List[string] }
    $table = ''
    foreach ($raw in [IO.File]::ReadAllLines($Path)) {
        $line = ([regex]::Match($raw, '^(?:[^"''#]|"(?:[^"\\]|\\.)*"|''[^'']*'')*')).Value.Trim()
        if (-not $line) { continue }
        if ($line.StartsWith('[')) { $table = $line.Trim('[', ']', ' ', "`t"); $result.Tables.Add($table); continue }
        $m = [regex]::Match($line, '^([^=]+?)\s*=\s*(.*)$')
        if (-not $m.Success) { continue }
        $full = (@($table, $m.Groups[1].Value) | Where-Object { $_ }) -join '.'
        $result.Keys.Add(@{ Table = $table; Full = (Convert-TomlKey $full); Value = $m.Groups[2].Value.Trim() })
    }
    return $result
}
function Read-TomlOrNote([string]$Path) {
    if (-not [IO.File]::Exists($Path)) { return $null }
    try { return Read-TomlLines $Path }
    catch { throw "$Path cannot be read or safely parsed for hook policy; nothing was changed: $($_.Exception.Message)" }
}
# [features] hooks = false in the user config.toml or the managed requirements.toml, and
# allow_managed_hooks_only = true in requirements.toml, keep Codex from running user hooks.
function Get-CodexBlocks([string]$ConfigPath, [string]$RequirementsPath) {
    $blocks = @()
    foreach ($source in @(@{ Path = $ConfigPath; Label = 'user config' }, @{ Path = $RequirementsPath; Label = 'managed requirements' })) {
        $toml = Read-TomlOrNote $source.Path
        if (-not $toml) { continue }
        foreach ($entry in $toml.Keys) {
            $off = ($entry.Full -ceq 'features.hooks' -and $entry.Value -ceq 'false') -or ($source.Label -eq 'managed requirements' -and $entry.Full -ceq 'hooks' -and $entry.Value -ceq 'false') -or ($entry.Full -ceq 'features' -and (Test-InlineHooksOff $entry.Value))
            if ($off) { $blocks += "[features] hooks = false in $($source.Label) $($source.Path)" }
            if ($source.Label -eq 'managed requirements' -and $entry.Full -ceq 'allow_managed_hooks_only' -and $entry.Value -ceq 'true') {
                $blocks += "allow_managed_hooks_only = true in managed requirements $($source.Path)"
            }
        }
    }
    return , $blocks
}
# Inline [hooks.<Event>] tables next to hooks.json make Codex warn that it loads hooks from both.
function Get-InlineHookTables([string]$ConfigPath) {
    $toml = Read-TomlOrNote $ConfigPath
    if (-not $toml) { return , @() }
    $inline = @($toml.Tables | Where-Object { ($_ -replace '\s', '') -match '^hooks\.(?!state(\.|$))' })
    $inline += @($toml.Keys | Where-Object { $_.Full -match '^hooks\.(?!state(\.|$))' } | ForEach-Object { $_.Full })
    return , $inline
}
# Codex records trust per hook as [hooks.state.'<hooks.json>:pre_tool_use:<group>:<handler>'].
function Get-CodexTrust([string]$ConfigPath, [string]$HooksPath, [int]$Group, [int]$Index) {
    $toml = Read-TomlOrNote $ConfigPath
    if (-not $toml) { return 'not yet trusted: approve it in Codex /hooks' }
    $want = "${HooksPath}:pre_tool_use:${Group}:${Index}"
    $header = [regex]'^hooks\s*\.\s*state\s*\.\s*(?:''([^'']*)''|"((?:[^"\\]|\\.)*)")$'
    $tables = @($toml.Tables | Where-Object {
            $m = $header.Match($_)
            $m.Success -and ($(if ($m.Groups[1].Success) { $m.Groups[1].Value } else { $m.Groups[2].Value -replace '\\(.)', '$1' }) -ieq $want)
        })
    if (-not $tables.Count) { return 'not yet trusted: approve it in Codex /hooks' }
    $enabled = @($toml.Keys | Where-Object { $tables -ccontains $_.Table -and $_.Full -and $_.Full.EndsWith('.enabled') })
    if ($enabled.Count -and $enabled[-1].Value -ceq 'false') { return 'disabled in Codex /hooks' }
    return 'trust recorded in Codex (Codex re-checks it against the entry)'
}

# --- settings file I/O -------------------------------------------------------------
function Assert-PlainPath([string]$Path) {
    $current = [IO.Path]::GetFullPath($Path)
    while ($current) {
        if ((Test-Path -LiteralPath $current) -and ((Get-Item -LiteralPath $current -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw "$current is a link or junction; edit the hook entry manually"
        }
        $parent = [IO.Path]::GetDirectoryName($current)
        if ($parent -eq $current) { break }; $current = $parent
    }
}
function Read-Settings([string]$Path) {
    $state = @{ Path = $Path; Exists = [IO.File]::Exists($Path); Bytes = $null; Bom = ''; Text = ''; Root = $null }
    if ($state.Exists) {
        $state.Bytes = [IO.File]::ReadAllBytes($Path)
        try { $text = (New-Object Text.UTF8Encoding($false, $true)).GetString($state.Bytes) } catch { throw "$Path is not valid UTF-8" }
        if ($text.Length -gt 0 -and [int]$text[0] -eq 0xFEFF) { $state.Bom = [string][char]0xFEFF; $text = $text.Substring(1) }
        $state.Text = $text
    }
    if ($state.Text.Trim().Length -eq 0) { $state.Root = $null; return $state }
    try { $state.Root = ConvertFrom-JsonText $state.Text } catch { throw "$Path is not valid JSON ($($_.Exception.Message))" }
    return $state
}
function Save-Settings($State, [string]$Text) {
    $path = $State.Path
    Assert-PlainPath $path
    $bytes = (New-Object Text.UTF8Encoding($false)).GetBytes($State.Bom + $Text)
    $directory = [IO.Path]::GetDirectoryName($path)
    [IO.Directory]::CreateDirectory($directory) | Out-Null
    if ([IO.File]::Exists($path) -ne $State.Exists) { throw "$path changed while mmhmem was editing it; nothing was written" }
    if ($State.Exists -and [Convert]::ToBase64String([IO.File]::ReadAllBytes($path)) -cne [Convert]::ToBase64String($State.Bytes)) { throw "$path changed while mmhmem was editing it; nothing was written" }
    $stage = Join-Path $directory ('.mmhmem-' + [guid]::NewGuid().ToString('N') + '.tmp')
    try {
        if ($State.Exists) { [IO.File]::WriteAllBytes($path + '.mmhmem.bak', $State.Bytes) }
        [IO.File]::WriteAllBytes($stage, $bytes)
        if ($State.Exists) { [IO.File]::Replace($stage, $path, [NullString]::Value) } else { [IO.File]::Move($stage, $path) }
    } finally { if ([IO.File]::Exists($stage)) { [IO.File]::Delete($stage) } }
}

# --- policy -----------------------------------------------------------------------
function Get-PolicyBlock($Root, [string]$Source, [switch]$Managed) {
    if ($null -eq $Root -or $Root.K -ne 'o') { return $null }
    $disable = Get-JsonMember $Root 'disableAllHooks'
    if ($disable -and $disable.V.K -eq 'b' -and $disable.V.V) { return "disableAllHooks: true in $Source" }
    if (-not $Managed) { return $null }
    $only = Get-JsonMember $Root 'allowManagedHooksOnly'
    if ($only -and $only.V.K -eq 'b' -and $only.V.V) { return "allowManagedHooksOnly: true in $Source" }
    $strict = Get-JsonMember $Root 'strictPluginOnlyCustomization'
    if ($strict) {
        if ($strict.V.K -eq 'b' -and $strict.V.V) { return "strictPluginOnlyCustomization: true in $Source" }
        if ($strict.V.K -eq 'a' -and @($strict.V.I | Where-Object { $_.K -eq 's' -and $_.V -ceq 'hooks' }).Count) { return "strictPluginOnlyCustomization includes ""hooks"" in $Source" }
        # Claude Code treats a value that is neither a boolean nor an array as locking every surface.
        if ($strict.V.K -ne 'b' -and $strict.V.K -ne 'a') { return "strictPluginOnlyCustomization (non-boolean value) in $Source" }
    }
    return $null
}
# Claude Code starts without a managed source the OS denies it; say so instead of staying silent.
function Write-DeniedNote([string]$Path) {
    [Console]::Error.WriteLine("[mmhmem] note: managed settings $Path cannot be read by this user (permission denied); Claude Code also runs without it, so it was not checked.")
}
function Get-ManagedBlocks($Def, [string]$ManagedPath, [bool]$UseRegistry) {
    $blocks = @()
    $files = @()
    if ([IO.File]::Exists($ManagedPath)) { $files += $ManagedPath }
    $dropIns = Join-Path ([IO.Path]::GetDirectoryName($ManagedPath)) 'managed-settings.d'
    if ([IO.Directory]::Exists($dropIns)) {
        try { $files += @(Get-ChildItem -LiteralPath $dropIns -Filter '*.json' -File -ErrorAction Stop | Where-Object { $_.Name.EndsWith('.json', [StringComparison]::OrdinalIgnoreCase) -and -not $_.Name.StartsWith('.') } | Sort-Object Name | ForEach-Object { $_.FullName }) }
        catch {
            if ($_.Exception -is [UnauthorizedAccessException]) { Write-DeniedNote $dropIns }
            else { $blocks += "managed settings $dropIns cannot be read; Claude Code refuses to start until it is fixed" }
        }
    }
    foreach ($file in $files) {
        $source = "managed settings $file"
        try { $text = [IO.File]::ReadAllText($file) }
        catch [UnauthorizedAccessException] { Write-DeniedNote $file; continue }
        catch { $blocks += "$source cannot be parsed; Claude Code refuses to start until it is fixed"; continue }
        if ($text.Trim().Length -eq 0) { continue }  # an empty managed settings file counts as {}
        try { $root = ConvertFrom-JsonText $text }
        catch { $blocks += "$source cannot be parsed; Claude Code refuses to start until it is fixed"; continue }
        if ($root.K -ne 'o') { $blocks += "$source is not a JSON object; Claude Code refuses to start until it is fixed"; continue }
        $block = Get-PolicyBlock $root $source -Managed
        if ($block) { $blocks += $block }
    }
    if ($UseRegistry) {
        foreach ($key in $Def.PolicyRegistry) {
            $value = $null
            try { $value = (Get-ItemProperty -LiteralPath $key -Name 'Settings' -ErrorAction Stop).Settings } catch { continue }
            if (-not $value) { continue }
            # Unparsable or non-object policy blocks, as an unparsable managed file does.
            try { $root = ConvertFrom-JsonText ([string]$value) } catch { $blocks += "managed settings registry $key\Settings is not valid JSON"; continue }
            if ($root.K -ne 'o') { $blocks += "managed settings registry $key\Settings is not a JSON object; Claude Code refuses to start until it is fixed"; continue }
            $block = Get-PolicyBlock $root "managed settings registry $key\Settings" -Managed
            if ($block) { $blocks += $block }
        }
    }
    return , $blocks
}

# --- commands -----------------------------------------------------------------------
$def = $script:Harnesses[$Harness]
$isCodex = $Harness -eq 'codex'
$userDir = Get-UserConfigDir $Harness
$fileName = if ($isCodex) { 'hooks.json' } else { 'settings.json' }
$target = if ($SettingsPath) { [IO.Path]::GetFullPath($SettingsPath) } else { Join-Path $userDir $fileName }
$managedPath = if ($ManagedSettingsPath) { [IO.Path]::GetFullPath($ManagedSettingsPath) } else { $def.DefaultManaged }
$useRegistry = -not $ManagedSettingsPath
$prefix = if ($DryRun) { '[dry-run] ' } else { '' }
$targetDir = [IO.Path]::GetDirectoryName($target)
# Codex reads config.toml next to its hooks.json (one CODEX_HOME layer).
$codexConfig = Join-Path $targetDir 'config.toml'

# User scope only: a project's .claude/.codex directory or a local settings file is never a target.
$projectDir = if ($isCodex) { '.codex' } else { '.claude' }
if (([IO.Path]::GetFileName($targetDir) -ieq $projectDir) -and ($targetDir.TrimEnd('\', '/') -ine $userDir.TrimEnd('\', '/'))) {
    Stop-Helper "$target looks like project settings; mmhmem only adds hooks to the user settings file. Nothing was changed."
}
if ((-not $isCodex -and [IO.Path]::GetFileName($target) -ieq 'settings.local.json') -or ($isCodex -and [IO.Path]::GetFileName($target) -ine 'hooks.json')) {
    Stop-Helper "$target is not the user hooks file mmhmem manages ($fileName). Nothing was changed."
}

if ($Command -in @('show', 'install')) {
    if (-not $Binary) { Stop-Helper '-Binary <absolute path to mmhmem.exe> is required.' }
    # A drive or UNC root is required: "\x\mmhmem.exe" is rooted but drive-relative.
    if (-not [IO.Path]::IsPathRooted($Binary) -or [IO.Path]::GetPathRoot($Binary) -notmatch '^([A-Za-z]:\\|\\\\)') { Stop-Helper "-Binary must be an absolute path with a drive or UNC root: $Binary" }
    $Binary = [IO.Path]::GetFullPath($Binary)
    $leaf = [IO.Path]::GetFileName($Binary)
    if ($leaf -ne 'mmhmem.exe' -and $leaf -ne 'mmhmem') { Stop-Helper "-Binary must point to mmhmem.exe: $Binary" }
    # PowerShell also ends a single-quoted string at the typographic single quotes.
    if ($isCodex -and $Binary -match "['\u2018\u2019\u201A\u201B]") { Stop-Helper "-Binary must not contain a single quote for the Codex entry: $Binary" }
}
$entry = if ($Binary) { Get-Entry $def $Binary } else { $null }
$removeCommand = 'powershell -NoProfile -ExecutionPolicy Bypass -File "' + $PSCommandPath + '" remove -Harness ' + $Harness + $(if ($SettingsPath) { ' -SettingsPath "' + $target + '"' } else { '' })

function Get-Blocks($State) {
    if ($isCodex) { return , (Get-CodexBlocks $codexConfig $managedPath) }
    $blocks = @()
    $userBlock = Get-PolicyBlock $State.Root "user settings $target"
    if ($userBlock) { $blocks += $userBlock }
    $blocks += Get-ManagedBlocks $def $managedPath $useRegistry
    return , $blocks
}
# Codex: the command string carries the binary; Claude Code: the exec-form command member.
function Get-InstalledBinary($Node) {
    if ($isCodex) { return Get-CodexBinary $Node }
    return (Get-JsonMember $Node 'command').V.V
}

try { $state = Read-Settings $target } catch { Stop-Helper "$($_.Exception.Message). Nothing was changed." }
try { $own = if ($state.Root) { Find-Own $state.Root $def } else { @{ Hooks = $null; Pre = $null; Found = (New-Object Collections.Generic.List[object]) } } }
catch { Stop-Helper "$target cannot be edited safely: $($_.Exception.Message). Nothing was changed." }

switch ($Command) {
    'show' {
        Write-Line "[mmhmem] $($def.Label) read-routing hook"
        Write-Line "File: $target"
        Write-Line 'Entry appended to hooks.PreToolUse:'
        Write-Line (ConvertTo-JsonText $entry '' @{ NL = [Environment]::NewLine; Unit = '  '; Pretty = $true; Colon = ': ' })
        Write-Line "What it sees and decides: $($def.Sees)"
        Write-Line "Approval: $($def.Approval)"
        Write-Line "Remove it with: $removeCommand"
    }
    'status' {
        $blocks = Get-Blocks $state
        $present = $own.Found.Count -gt 0
        if ($blocks.Count) {
            $note = if ($present) { ' (entry present in the settings file)' } else { '' }
            Write-Line "$($Harness): policy-blocked by $($blocks -join '; ')$note"
        } elseif ($present) {
            $trust = if ($isCodex) { '; ' + (Get-CodexTrust $codexConfig $target $own.Found[0].Group $own.Found[0].Index) } else { '' }
            Write-Line "$($Harness): installed in $target ($(Get-InstalledBinary $own.Found[0].Node))$trust"
        } else {
            Write-Line "$($Harness): not installed ($target)"
        }
    }
    'install' {
        $blocks = Get-Blocks $state
        if ($blocks.Count) { Stop-Helper "$($def.Label) hooks are disabled or restricted by policy: $($blocks -join '; '). The hook was not added." }
        $text = $state.Text
        $message = ''
        $changedEntry = $false
        if ($own.Found.Count -eq 0) {
            if (-not $state.Root) { $text = ConvertTo-JsonText ([ordered]@{ hooks = [ordered]@{ PreToolUse = @($entry) } }) '' @{ NL = "`n"; Unit = '  '; Pretty = $true; Colon = ': ' }; $text += "`n" }
            else {
                $style = Get-FileStyle $text
                if ($own.Pre) { $text = Add-JsonElement $text $own.Pre.V $null $entry $style }
                elseif ($own.Hooks) { $text = Add-JsonElement $text $own.Hooks.V 'PreToolUse' @($entry) $style }
                else { $text = Add-JsonElement $text $state.Root 'hooks' ([ordered]@{ PreToolUse = @($entry) }) $style }
            }
            $message = "added the read-routing hook to $target"
        } else {
            $first = $own.Found[0]
            $cmdNode = (Get-JsonMember $first.Node 'command').V
            $installed = Get-InstalledBinary $first.Node
            if ($installed -cne $Binary) {
                $newCommand = if ($isCodex) { Get-CodexCommand $Binary } else { $Binary }
                $text = $text.Substring(0, $cmdNode.S) + (ConvertTo-JsonString $newCommand) + $text.Substring($cmdNode.E)
                $message = "replaced the hook's binary path $installed with $Binary in $target"
                $changedEntry = $true
            }
            # Duplicates (from manual edits) are removed so exactly one entry remains.
            while ($true) {
                $found = (Find-Own (ConvertFrom-JsonText $text) $def).Found
                if ($found.Count -le 1) { break }
                $text = Remove-OwnHandler $text $def last
            }
        }
        if ($text -ceq $state.Text) { Write-Line "[mmhmem] $($def.Label) hook already installed in $target (unchanged)"; break }
        if (-not $message) { $message = "removed duplicate read-routing hook entries from $target" }
        if (-not $DryRun) {
            try { Save-Settings $state $text } catch { Stop-Helper "$($_.Exception.Message)" }
        }
        Write-Line "[mmhmem] $prefix$message"
        if ($state.Exists -and -not $DryRun) { Write-Line "[mmhmem] backup: $target.mmhmem.bak" }
        if ($isCodex -and $changedEntry) { Write-Line "[mmhmem] $($def.Reapprove)" }
        if ($isCodex) {
            $inline = Get-InlineHookTables $codexConfig
            if ($inline.Count) { Write-Line "[mmhmem] note: $codexConfig also defines hooks inline ($($inline -join ', ')); Codex loads both and warns about it at startup. Move them into $target to silence the warning." }
        }
        Write-Line "[mmhmem] $($def.Approval)"
        Write-Line "[mmhmem] Remove it with: $removeCommand"
    }
    'remove' {
        $text = $state.Text
        while ($state.Root) {
            $found = (Find-Own (ConvertFrom-JsonText $text) $def).Found
            if ($found.Count -eq 0) { break }
            $text = Remove-OwnHandler $text $def first
        }
        if ($text -ceq $state.Text) { Write-Line "[mmhmem] no $($def.Label) read-routing hook in $target (unchanged)"; break }
        if (-not $DryRun) {
            try { Save-Settings $state $text } catch { Stop-Helper "$($_.Exception.Message)" }
        }
        Write-Line "[mmhmem] $($prefix)removed the read-routing hook from $target"
        if (-not $DryRun) { Write-Line "[mmhmem] backup: $target.mmhmem.bak" }
    }
}
exit 0
