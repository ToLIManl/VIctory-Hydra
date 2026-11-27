param(
    [Parameter(Mandatory = $true)]
    [string]$Message,
    [switch]$Push,
    [switch]$SkipBuildCheck
)

$ErrorActionPreference = "Stop"
$RepoDir = Split-Path $PSScriptRoot -Parent

Push-Location $RepoDir
try {
    if (-not (Test-Path (Join-Path $RepoDir ".git"))) {
        throw "Not a git repository yet. Run git init first."
    }

    Write-Host "[1/4] Checking asset policy (cargo xtask policy)..."
    cargo run --offline -p xtask -- policy
    if ($LASTEXITCODE -ne 0) {
        throw "Asset policy check failed (exit $LASTEXITCODE). Aborting."
    }

    Write-Host "[2/4] Scanning repository files for hardcoded local paths or private artifacts..."
    $gitFiles = git ls-files --cached --others --exclude-standard
    $leakPattern = '(?i)([A-Z]:\\Users\\(?!\.\.)[^\s''"\\]+|[A-Z]:\\down\\)'
    $leaks = @()
    foreach ($rel in $gitFiles) {
        if ($rel -ieq "HANDOFF.md") {
            $leaks += "${rel}: private handoff file must stay gitignored"
            continue
        }
        if ($rel -eq "tools/push-update.ps1") { continue }
        if ($rel -match '\.(bin|png|jpg|jpeg)$') { continue }
        $full = Join-Path $RepoDir $rel
        if (-not (Test-Path $full -PathType Leaf)) { continue }
        $hits = Select-String -Path $full -Pattern $leakPattern -ErrorAction SilentlyContinue
        if ($hits) {
            foreach ($h in $hits) {
                $leaks += "${rel}:$($h.LineNumber): $($h.Line.Trim())"
            }
        }
    }
    if ($leaks.Count -gt 0) {
        Write-Host "ERROR: Potential personal path / private data leak detected:" -ForegroundColor Red
        $leaks | ForEach-Object { Write-Host "  $_" -ForegroundColor Red }
        throw "Aborting commit due to privacy check failure."
    }
    Write-Host "  Privacy scan OK ($($gitFiles.Count) files checked, 0 leaks)."

    if (-not $SkipBuildCheck) {
        Write-Host "[3/4] Running quick workspace check (cargo check --offline -p xtask)..."
        cargo check --offline -p xtask
        if ($LASTEXITCODE -ne 0) {
            throw "xtask check failed (exit $LASTEXITCODE)."
        }
    }

    Write-Host "[4/4] Staging and committing..."
    git add -A
    $status = git status --short
    if (-not $status) {
        Write-Host "Nothing to commit (working tree clean)."
        exit 0
    }
    $status | ForEach-Object { Write-Host "  $_" }

    git commit -m $Message
    if ($LASTEXITCODE -ne 0) {
        throw "git commit failed (exit $LASTEXITCODE)."
    }

    if ($Push) {
        Write-Host "Pushing to origin..."
        git push
        if ($LASTEXITCODE -ne 0) {
            throw "git push failed (exit $LASTEXITCODE)."
        }
        Write-Host "Push complete." -ForegroundColor Green
    } else {
        Write-Host "Local commit created. Run 'git push' (or re-run with -Push) when ready to publish." -ForegroundColor Yellow
    }
} finally {
    Pop-Location
}
