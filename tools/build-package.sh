#!/usr/bin/env bash
# Builds the RF-Rig .rfplugin package.
#
# Usage: tools/build-package.sh [output.rfplugin]
# The RackForge checkout is expected next to this repository, or at
# $RACKFORGE_ROOT.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
output="${1:-$repo_root/artifacts/RF-Rig.rfplugin}"
rackforge_root="${RACKFORGE_ROOT:-$(dirname "$repo_root")/rackforge}"

case "$output" in
  *.rfplugin) ;;
  *) echo "Plugin package output must end in .rfplugin" >&2; exit 1 ;;
esac
if [ -e "$output" ]; then
  echo "Refusing to overwrite existing package: $output" >&2
  exit 1
fi
if [ ! -f "$rackforge_root/Cargo.toml" ]; then
  echo "RackForge checkout not found at $rackforge_root" >&2
  exit 1
fi

cd "$repo_root"

# Metadata comes from the contract; the runtime descriptor comes from the
# manifest. Regenerating both here keeps the release version written in exactly
# one file.
cargo run --locked --release -p rf-rig-lab -- metadata
# The component is built before the tests, not after: one of them loads the
# built wasm in the host's own runtime, and running it first would have it
# certify the *previous* build. That test is the only thing standing between a
# host ABI change and a package that installs and then fails to start, so it
# has to be looking at the component this run produced.
cargo build --locked --release -p rackforge-rf-rig --target wasm32-unknown-unknown
cargo test --locked --release --workspace

mkdir -p "$(dirname "$output")"
stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
cp -R "$repo_root/plugin/package/." "$stage/"
cp "$repo_root/LICENSE" "$repo_root/NOTICE.md" "$stage/"

cargo run --manifest-path "$rackforge_root/Cargo.toml" --locked -p rackforge-store -- \
  pack-wasm "$stage" \
  "$repo_root/target/wasm32-unknown-unknown/release/rackforge_rf_rig.wasm" \
  "$output"

echo "Packed $output"
