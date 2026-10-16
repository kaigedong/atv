#!/usr/bin/env bash
set -euo pipefail
project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
source "$project_dir/scripts/toolchain-env.sh"
# Optional user-local development plugins; no system configuration is changed.
atv_plugin_dir="${XDG_DATA_HOME:-$HOME/.local/share}/atv-desktop/plugins"
if [[ -d "$atv_plugin_dir" ]]; then
  export GST_PLUGIN_PATH="$atv_plugin_dir${GST_PLUGIN_PATH:+:$GST_PLUGIN_PATH}"
fi
if [[ "${1:-}" == --build-only ]]; then
  shift
  exec cargo build --locked --manifest-path "$project_dir/native/Cargo.toml" -p atv-desktop "$@"
fi
exec cargo run --locked --manifest-path "$project_dir/native/Cargo.toml" -p atv-desktop -- "$@"
