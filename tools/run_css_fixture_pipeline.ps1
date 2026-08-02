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

function Test-EditableInstallEnv {
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

function Build-WithNativeBackendOrFallback {
    if (Test-EditableInstallEnv) {
        & python -m pip install --no-build-isolation --no-deps --editable .
        if ($LASTEXITCODE -ne 0) {
            throw "native editable install failed with exit code $LASTEXITCODE"
        }
    } else {
        Write-Host "No active virtualenv/conda environment; syncing a Cargo release build into python/fullbleed."
        & cargo build -q --release --features python,svg_raster
        if ($LASTEXITCODE -ne 0) {
            throw "cargo build failed with exit code $LASTEXITCODE"
        }
        Sync-LocalPythonModule
    }
}

if ($Build) {
    Build-WithNativeBackendOrFallback
} else {
    Write-Host "Skipping build sync (use -Build to rebuild the Python extension)."
}
$env:PYTHONPATH = "python"
& python "tools/run_css_fixture_suite.py" @FixtureArgs
exit $LASTEXITCODE
