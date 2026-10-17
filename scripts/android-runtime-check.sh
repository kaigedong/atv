#!/usr/bin/env bash
set -euo pipefail
project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
source "$project_dir/scripts/toolchain-env.sh"
: "${ANDROID_HOME:?Set ANDROID_HOME to your installed Android SDK}"
serial="${ATV_TEST_DEVICE:-emulator-5570}"
mode="${ATV_TEST_MODE:-smoke}"
duration="${ATV_TEST_DURATION_SECONDS:-600}"
case "$mode" in smoke|soak|lifecycle|audio-lifecycle|audio-focus-basic|retry|remote|groups|channel-idle|feedback|network|startup-network|all|component|settings|settings-basic|overlays|overlays-basic) ;; *) echo 'Unsupported native test mode.' >&2; exit 1 ;; esac
artifact_dir="${ATV_TEST_ARTIFACTS:-$project_dir/test-artifacts/native/$mode}"
adb_command="$ANDROID_HOME/platform-tools/adb"
mkdir -p "$artifact_dir"
build_options=()
if [[ "$mode" == component ]]; then build_options+=(-PatvOutputProbe=true); fi
"$project_dir/scripts/android-build.sh" assembleDebug assembleDebugAndroidTest lintDebug "${build_options[@]}"
"$adb_command" -s "$serial" install -r "$project_dir/app/build/outputs/apk/debug/app-debug.apk"
"$adb_command" -s "$serial" install -r "$project_dir/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk"
"$adb_command" -s "$serial" shell am instrument -w -e mode "$mode" -e durationSeconds "$duration" \
  cn.bobo.atv.nativeplayer.test/cn.bobo.atv.nativeplayer.NativeRuntimeInstrumentation | tee "$artifact_dir/instrumentation.txt"
python3 - "$artifact_dir/instrumentation.txt" "$artifact_dir" "$adb_command" "$serial" <<'PY'
import json
from pathlib import Path
import re
import subprocess
import sys
text = Path(sys.argv[1]).read_text()
match = re.search(r'^INSTRUMENTATION_RESULT: reportPath=((?:/storage/emulated/0/Android/data/cn\.bobo\.atv\.nativeplayer/files|/data/(?:data|user/0)/cn\.bobo\.atv\.nativeplayer/files)/runtime-native/[0-9a-f-]{36}\.json)\s*$', text, re.MULTILINE)
if not match:
    raise SystemExit('Native test did not provide a runtime report')
destination = Path(sys.argv[2]) / 'result.json'
if match.group(1).startswith('/data/'):
    content = subprocess.check_output([sys.argv[3], '-s', sys.argv[4], 'shell', 'run-as',
                                      'cn.bobo.atv.nativeplayer', 'cat', match.group(1)])
    destination.write_bytes(content.replace(b'\r\n', b'\n'))
else:
    subprocess.run([sys.argv[3], '-s', sys.argv[4], 'pull', match.group(1), str(destination)], check=True)
result = json.loads(destination.read_text())
print(json.dumps({'mode': result.get('mode'), 'passed': result.get('passed'),
                  'targetApkSha256': result.get('targetApkSha256'), 'report': str(destination)}, indent=2))
if result.get('passed') is not True:
    raise SystemExit(1)
PY
