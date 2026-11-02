$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '../deploy/check_worker.ps1')
Add-Type @'
public class WorkerCheckResponse { public int StatusCode { get; set; } }
public class WorkerCheckException : System.Exception {
    public WorkerCheckResponse Response { get; set; }
    public WorkerCheckException(int code) : base("provider fixture-key must never be printed") {
        Response = new WorkerCheckResponse { StatusCode = code };
    }
}
'@
$fixture = @{ Status = 0; Malformed = $false; Requests = 0 }
function Invoke-RestMethod {
    param($Method, $Uri, $Headers, $Body, $ContentType, $TimeoutSec, $MaximumRedirection, $ErrorAction)
    $fixture.Requests++
    $request = [Text.Encoding]::UTF8.GetString($Body) | ConvertFrom-Json
    if ($Method -ne 'Post' -or $request.model -ne 'fixture-model' -or $request.temperature -ne 0 -or $Headers.Authorization -ne 'Bearer fixture-key' -or $MaximumRedirection -ne 0) {
        throw 'Wrong worker verification request'
    }
    if ($fixture.Status) { throw (New-Object WorkerCheckException $fixture.Status) }
    if ($fixture.Malformed) { return @{ choices = @() } }
    @{ choices = @(@{ message = @{ content = 'OK' } }) }
}
$settings = @{ Model = 'fixture-model'; Url = 'https://worker.example/v1/chat/completions'; Key = 'fixture-key' }
Test-MmhmemWorker $settings
foreach ($status in @(400, 401, 403, 404, 429, 500)) {
    $fixture.Status = $status
    $failed = $false
    try { Test-MmhmemWorker $settings } catch {
        $failed = $true
        if ($_ -match 'fixture-key' -or $_ -notmatch 'No new settings were saved') { throw 'Unsafe or unclear validation error' }
    }
    if (-not $failed) { throw "HTTP $status was accepted" }
}
$fixture.Status = 0; $fixture.Malformed = $true
$failed = $false
try { Test-MmhmemWorker $settings } catch { $failed = $_ -match 'invalid chat-completions' }
if (-not $failed) { throw 'Malformed success response accepted' }
$settings.Url = 'not-a-url'
$before = $fixture.Requests
try { Test-MmhmemWorker $settings; throw 'Invalid URL accepted' } catch { if ($_ -notmatch 'Invalid worker URL') { throw } }
if ($fixture.Requests -ne $before) { throw 'Malformed URL made a network request' }
Write-Host 'PASS: worker request shape, bad credentials, model/URL errors, quota, malformed response, secret-safe errors'
