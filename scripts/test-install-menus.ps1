$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$engine = [IO.File]::ReadAllText((Join-Path $root 'deploy/install_local.ps1'))
$start = $engine.IndexOf('if (-not $Agent -and $Interactive) {')
$end = $engine.IndexOf('$Registered =', $start)
$selection = [scriptblock]::Create($engine.Substring($start, $end - $start))
$answers = New-Object 'System.Collections.Generic.Queue[string]'
function Read-Host([string]$Prompt) {
    if ($Prompt -notmatch '\[1\] desktop' -or $Prompt -notmatch '\[2\] codex') { throw "Menu numbering mismatch: $Prompt" }
    if ($answers.Count -eq 0) { throw 'Unexpected repeated prompt' }
    $answers.Dequeue()
}
$Agent = ''; $Interactive = $true; $Global = $true; $Remove = $false
$HasClaude = $false; $HasOpenCode = $false; $HasDesktop = $true; $HasAntigravity = $false; $HasCodex = $true
foreach ($case in @(@('9', '2'), @('1,2'), @('a'))) {
    foreach ($answer in $case) { $answers.Enqueue($answer) }
    . $selection
    $expected = switch ($case[-1]) { '2' { 'codex' }; '1,2' { 'desktop,codex' }; 'a' { 'all' } }
    if (($Agents -join ',') -ne $expected) { throw "Wrong selection: $Agents; expected $expected" }
}
# Exercise optional setup independently, including hidden API-key input.
$start = $engine.IndexOf('# --- optional worker setup')
$end = $engine.IndexOf('# --- end optional worker setup', $start)
$workerSetup = [scriptblock]::Create($engine.Substring($start, $end - $start))
function Read-Host([string]$Prompt, [switch]$AsSecureString) {
    if ($answers.Count -eq 0) { throw 'Unexpected worker prompt' }
    $answer = $answers.Dequeue()
    if ($AsSecureString) { return ConvertTo-SecureString $answer -AsPlainText -Force }
    if ($Prompt -like '*API key*') { throw 'API key input was not hidden' }
    $answer
}
$WorkerModel = ''; $WorkerUrl = ''; $WorkerApiKey = ''
foreach ($answer in @('y', 'test-model', 'https://worker.example/v1/', 'fixture-key')) { $answers.Enqueue($answer) }
. $workerSetup
if ($WorkerModel -ne 'test-model' -or $WorkerUrl -ne 'https://worker.example/v1/chat/completions' -or $WorkerApiKey -ne 'fixture-key') {
    throw 'Worker setup did not collect or normalize the connection settings'
}
$WorkerModel = ''; $WorkerUrl = 'https://existing.example/v1/chat/completions'; $WorkerApiKey = 'existing-key'
$answers.Enqueue('n')
. $workerSetup
if ($WorkerModel -or $WorkerUrl -ne 'https://existing.example/v1/chat/completions' -or $WorkerApiKey -ne 'existing-key') { throw 'Skipping worker setup changed settings' }
Write-Host 'PASS: optional worker setup, base URL normalization, hidden API key, skip preserves settings'
$originalLocal = $env:LOCALAPPDATA
$originalRoaming = $env:APPDATA
$scratch = Join-Path $root ('tmp/menu-tests-' + [Guid]::NewGuid().ToString('N'))
try {
    $env:LOCALAPPDATA = Join-Path $scratch 'local'
    $env:APPDATA = Join-Path $scratch 'roaming'
    $msix = Join-Path $env:LOCALAPPDATA 'Packages/Claude_fixture/LocalCache/Roaming/Claude'
    New-Item -ItemType Directory -Force -Path $msix | Out-Null
    $shell = if ($PSVersionTable.PSEdition -eq 'Core') { 'pwsh.exe' } else { 'powershell.exe' }
    $output = & (Join-Path $PSHOME $shell) -NoProfile -File (Join-Path $root 'deploy/install_local.ps1') -Global -Agent desktop -DryRun -NonInteractive -Binary (Join-Path $scratch 'missing.exe') 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0 -or $output -notmatch 'Claude_fixture.*claude_desktop_config.json' -or $output -notmatch 'claude-desktop') {
        throw "MSIX registration failed: $output"
    }
    Write-Host 'PASS: sparse menu numbering, invalid input retry, multiple selections, all, MSIX Desktop dry run'
} finally {
    $env:LOCALAPPDATA = $originalLocal
    $env:APPDATA = $originalRoaming
}
