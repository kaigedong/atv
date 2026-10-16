#!/usr/bin/env bash
set -euo pipefail
project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_dir"
source "$project_dir/scripts/toolchain-env.sh"
if [[ $# -eq 0 ]]; then set -- assembleDebug lintDebug; fi
# Root Gradle owns Rust compilation and the mandatory native APK inspection.
exec ./gradlew "$@"
