# Live smoke test: a few small calls on the signed-in Claude account (or
# ANTHROPIC_API_KEY). Run it by hand before a release (AGENTS.md: tests that
# call real services are not part of the gates). It proves what the scripted
# tests cannot: that the API accepts the request bodies. The Claude side of the
# find tool shipped with a body the API refused (HTTP 400) because no live
# turn with a tool had been run.
#
#   pwsh scripts/live-smoke.ps1 [-Exe target/release/fast-harness.exe] [-Model claude-haiku-5-5]
#
# Sessions land in the usual place (~/.fast-harness/sessions/<temp folder>),
# so a failed step can be read with `fast-harness trace <id> --cwd <folder>`;
# the temp folder is kept when a step fails.
param(
    [string]$Exe = "$PSScriptRoot/../target/debug/fast-harness.exe",
    [string]$Model = 'claude-haiku-5-5'
)
$ErrorActionPreference = 'Stop'
$bin = (Resolve-Path $Exe).Path
$dir = Join-Path ([IO.Path]::GetTempPath()) "fast-harness-smoke-$PID"
New-Item -ItemType Directory $dir | Out-Null
Set-Content (Join-Path $dir 'notes.txt') "The secret word is lantern.`nSecond line."
$script:failed = 0

# Runs one command in the temp folder; passes when it exits 0 and its stdout matches.
function Step([string]$Name, [string]$Expect, [string[]]$Command) {
    $err = Join-Path $dir 'stderr.txt'
    $out = (& $bin @Command --cwd $dir 2>$err) -join "`n"
    $code = $LASTEXITCODE
    if ($code -eq 0 -and $out -match $Expect) {
        Write-Host "ok    $Name"
        return
    }
    $script:failed++
    Write-Host "FAIL  $Name (exit $code, expected /$Expect/)"
    Write-Host "      stdout: $out"
    Get-Content $err | Select-Object -Last 5 | ForEach-Object { Write-Host "      $_" }
}

$turn = @('--model', $Model, '--effort', 'low')
Step 'plain turn' 'ok' (@('-p', 'Reply with the single word: ok') + $turn)
Step 'find, read, write, shell in the same session' 'LANTERN' (@('-c', '-p',
        'Read notes.txt, write out.txt containing only the secret word in upper case, then print out.txt with a shell command. Answer with that word only.') + $turn)
Step 'find result with text beside the loaded tool' 'Unknown: frobnicate_tool' (@('-p',
        'Call find_tools once with names ["grep","frobnicate_tool"], then quote the line of its result that starts with "Unknown".') + $turn)
Step 'manual compaction' '.' @('compact', 'latest')
Step 'tool use after the compaction' 'Second line' (@('-c', '-p',
        'Use grep to find the lines of notes.txt containing "line" and answer with that line only.') + $turn)
Step 'subagent' '(?i)lantern' (@('-p',
        'Use the subagent tool once: the child reads notes.txt and reports the secret word. Answer with that word only.') + $turn)

if ($script:failed -eq 0 -and $dir.StartsWith([IO.Path]::GetTempPath())) {
    Remove-Item $dir -Recurse -Force
} else {
    Write-Host "kept $dir"
}
Write-Host "$($script:failed) failed"
exit $script:failed
