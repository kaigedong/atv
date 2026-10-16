#!/usr/bin/env bash
set -euo pipefail
project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
# Compatibility alias; there is only one Android project/build graph.
exec "$project_dir/scripts/android-build.sh" "$@"
