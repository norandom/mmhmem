function Get-MmhmemWorkerSettings([string]$ConfigFile, [string]$Model, [string]$Url, [string]$Key) {
    $saved = @{}
    if (Test-Path -LiteralPath $ConfigFile) {
        try { $saved = Get-Content -LiteralPath $ConfigFile -Raw | ConvertFrom-Json }
        catch { throw 'Cannot read worker settings. Correct the JSON configuration first.' }
    }
    if (-not $Model) { $Model = $saved.worker_model }
    if (-not $Url) { $Url = $saved.worker_api_url }
    if (-not $Key) { $Key = $saved.worker_api_key }
    if (-not $Model) { $Model = $env:WORKER_MODEL }
    if (-not $Url) { $Url = $env:WORKER_API_URL }
    if (-not $Key) { $Key = $env:WORKER_API_KEY }
    if (-not $Url) { $Url = 'http://127.0.0.1:11434/v1/chat/completions' }
    @{ Model = $Model; Url = $Url; Key = $Key }
}

function Test-MmhmemWorker($Settings) {
    if (-not $Settings.Model) { return }
    $uri = $null
    if (-not [Uri]::TryCreate($Settings.Url, [UriKind]::Absolute, [ref]$uri) -or $uri.Scheme -notin @('http', 'https') -or $uri.UserInfo) {
        throw 'Invalid worker URL. Enter an HTTP(S) base URL without embedded credentials.'
    }
    Write-Host "`n  Testing worker connection with a short 'Reply OK' request."
    Write-Host '  No project files are sent. Provider usage charges may apply.'
    $headers = @{}
    if ($Settings.Key) { $headers.Authorization = 'Bearer ' + $Settings.Key }
    $body = @{ model = $Settings.Model; temperature = 0.0; messages = @(
        @{ role = 'system'; content = 'You are a connection test.' },
        @{ role = 'user'; content = 'Reply OK.' }
    ) } | ConvertTo-Json -Depth 5 -Compress
    try {
        $response = Invoke-RestMethod -Method Post -Uri $uri -Headers $headers -Body ([Text.Encoding]::UTF8.GetBytes($body)) -ContentType 'application/json' -TimeoutSec 30 -MaximumRedirection 0 -ErrorAction Stop
    } catch {
        $status = 0
        if ($_.Exception.Response) { $status = [int]$_.Exception.Response.StatusCode }
        $reason = switch ($status) {
            401 { 'Authentication failed: the API key is invalid or expired.' }
            403 { 'Access denied: check the API key and model permissions.' }
            404 { 'Endpoint or model not found: check the base URL and model ID.' }
            400 { 'Request rejected: check the model ID and OpenAI compatibility.' }
            429 { 'Provider quota or rate limit reached. Check billing or retry later.' }
            0 { 'Connection failed or timed out. Check the URL, network, and TLS certificate.' }
            default { "Provider returned HTTP $status. Check the endpoint and retry." }
        }
        throw "Worker check failed.`n$reason`nNo new settings were saved.`nRe-run to correct them."
    }
    if (-not $response.choices -or $response.choices[0].message.content -isnot [string] -or -not $response.choices[0].message.content.Trim()) {
        throw 'Worker returned an invalid chat-completions response. No new settings were saved.'
    }
    Write-Host '  Worker connection verified.' -ForegroundColor Green
}
