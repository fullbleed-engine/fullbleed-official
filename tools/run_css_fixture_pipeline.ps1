param(
    [switch]$Build,
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$FixtureArgs
)

$ErrorActionPreference = "Stop"

function Sync-LocalPythonModule {
    $extSuffix = (
        python -c "import sysconfig; print(sysconfig.get_config_var('EXT_SUFFIX') or '.pyd')"
    ).Trim()
    if ([string]::IsNullOrWhiteSpace($extSuffix)) {
        $extSuffix = ".pyd"
    }

    $targetPath = Join-Path "python/fullbleed" ("_fullbleed" + $extSuffix)
    Copy-Item -Force "target/release/fullbleed.dll" $targetPath
}

function Test-CommandAvailable {
    param([Parameter(Mandatory = $true)][string]$Name)
    return $null -ne (Get-Command $Name -ErrorAction SilentlyContinue)
}

function Test-MaturinDevelopEnv {
    if (-not [string]::IsNullOrWhiteSpace($env:VIRTUAL_ENV)) {
        return $true
    }
    if (-not [string]::IsNullOrWhiteSpace($env:CONDA_PREFIX)) {
        return $true
    }
    if (Test-Path ".venv" -PathType Container) {
        return $true
    }
    return $false
}

function Build-WithMaturinOrFallback {
    $useFallback = $false

    if (-not (Test-CommandAvailable -Name "maturin")) {
        Write-Host "maturin not found on PATH; falling back to cargo release build with python feature."
        $useFallback = $true
    } elseif (-not (Test-MaturinDevelopEnv)) {
        Write-Host "No active virtualenv/conda environment for maturin develop; using cargo release build with python feature."
        $useFallback = $true
    } else {
        # Prefer maturin when possible, but it requires an active venv/conda environment.
        & maturin develop --release --features python
        if ($LASTEXITCODE -ne 0) {
            Write-Host "maturin develop exited with code $LASTEXITCODE; falling back to cargo release build with python feature."
            $useFallback = $true
        }
    }

    if ($useFallback) {
        & cargo build -q --release --features python
        if ($LASTEXITCODE -ne 0) {
            throw "cargo build failed with exit code $LASTEXITCODE"
        }
        Sync-LocalPythonModule
    }
}

if ($Build) {
    Build-WithMaturinOrFallback
} else {
    Write-Host "Skipping build sync (use -Build to rebuild the Python extension)."
}
$env:PYTHONPATH = "python"
& python "tools/run_css_fixture_suite.py" @FixtureArgs
exit $LASTEXITCODE
