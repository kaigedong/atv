#!/usr/bin/env python3
"""Install a local pre-push guard that survives checking out private old branches."""
from pathlib import Path
import shutil
import subprocess

root = Path(__file__).resolve().parent.parent
git_dir = Path(subprocess.check_output(["git", "rev-parse", "--absolute-git-dir"], cwd=root, text=True).strip())
hook = git_dir / "hooks/pre-push"
marker = "ATV-publication-guard"
if hook.exists() and marker not in hook.read_text():
    raise SystemExit("An existing pre-push hook needs manual integration; it was not overwritten")
shutil.copyfile(root / "scripts/check-publication.py", git_dir / "atv-publication-check.py")
hook.parent.mkdir(exist_ok=True)
hook.write_text('''#!/usr/bin/env bash
# ATV-publication-guard: inspect every pushed ref, including tags/private branches.
set -euo pipefail
root="$(git rev-parse --show-toplevel)"
git_dir="$(git rev-parse --absolute-git-dir)"
python_command="${ATV_PUBLICATION_PYTHON:-python3}"
while read -r local_ref local_sha remote_ref remote_sha; do
  if [[ "$local_sha" =~ ^0+$ ]]; then continue; fi
  "$python_command" "$git_dir/atv-publication-check.py" --root "$root" --history --ref "$local_sha"
done
''')
hook.chmod(0o755)
print("Installed the local publication guard; only reviewed reachable history may be pushed")
