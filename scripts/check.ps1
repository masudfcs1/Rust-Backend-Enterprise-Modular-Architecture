$ErrorActionPreference = 'Stop'
if (Test-Path -LiteralPath (Join-Path $PSScriptRoot '..\.tooling\use-rust.ps1')) {
    . (Join-Path $PSScriptRoot '..\.tooling\use-rust.ps1')
}
cargo fmt --all -- --check
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
cargo clippy --locked --all-targets -- -D warnings
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
cargo test --locked
exit $LASTEXITCODE
