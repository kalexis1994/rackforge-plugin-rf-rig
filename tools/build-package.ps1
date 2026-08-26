param(
    [string]$Output = "",
    [string]$RackForgeRoot = "",
    [string]$Toolchain = "stable-x86_64-pc-windows-msvc"
)

$ErrorActionPreference = "Stop"
$rigRepoRoot = Split-Path -Parent $PSScriptRoot

if ([string]::IsNullOrWhiteSpace($Output)) {
    $Output = Join-Path $rigRepoRoot "artifacts\RF-Rig.rfplugin"
}
if ([string]::IsNullOrWhiteSpace($RackForgeRoot)) {
    $RackForgeRoot = Join-Path (Split-Path -Parent $rigRepoRoot) "rackforge"
}
if (-not $Output.EndsWith(".rfplugin")) {
    throw "Plugin package output must end in .rfplugin"
}
if (Test-Path -LiteralPath $Output) {
    # The packager refuses to overwrite, and so does this script: a stale
    # package that silently survives a failed build is how a "fixed" bug comes
    # back.
    throw "Refusing to overwrite existing package: $Output"
}
if (-not (Test-Path -LiteralPath (Join-Path $RackForgeRoot "Cargo.toml"))) {
    throw "RackForge checkout not found at $RackForgeRoot"
}

Push-Location $rigRepoRoot
try {
    # The metadata is generated from the contract, and the runtime descriptor is
    # generated from the manifest. Regenerating here means the version in
    # rackforge-plugin.toml is the only place a release version is written.
    cargo "+$Toolchain" run --locked --release -p rf-rig-lab -- metadata
    if ($LASTEXITCODE -ne 0) { throw "RF-Rig metadata generation failed" }

    cargo "+$Toolchain" test --locked --release --workspace
    if ($LASTEXITCODE -ne 0) { throw "RF-Rig tests failed" }

    cargo "+$Toolchain" build --locked --release -p rackforge-rf-rig --target wasm32-unknown-unknown
    if ($LASTEXITCODE -ne 0) { throw "RF-Rig WebAssembly build failed" }

    $outputParent = Split-Path -Parent $Output
    New-Item -ItemType Directory -Path $outputParent -Force | Out-Null
    $stage = Join-Path ([System.IO.Path]::GetTempPath()) ("rf-rig-package-" + [guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Path $stage | Out-Null
    try {
        Copy-Item -Path (Join-Path $rigRepoRoot "plugin\package\*") -Destination $stage -Recurse
        Copy-Item -LiteralPath (Join-Path $rigRepoRoot "LICENSE") -Destination $stage
        Copy-Item -LiteralPath (Join-Path $rigRepoRoot "NOTICE.md") -Destination $stage
        $component = Join-Path $rigRepoRoot "target\wasm32-unknown-unknown\release\rackforge_rf_rig.wasm"
        cargo "+$Toolchain" run --manifest-path (Join-Path $RackForgeRoot "Cargo.toml") --locked -p rackforge-store -- pack-wasm $stage $component $Output
        if ($LASTEXITCODE -ne 0) { throw "RackForge packaging failed" }
    }
    finally {
        if (Test-Path -LiteralPath $stage) {
            Remove-Item -LiteralPath $stage -Recurse -Force
        }
    }
}
finally {
    Pop-Location
}

Write-Output "Packed $Output"
