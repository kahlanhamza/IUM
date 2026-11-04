#!/usr/bin/env bash
# kaggle_push_runner.sh — dispatch the self-hosted GitHub Actions runner
# to Kaggle compute.
#
# Usage: bash scripts/kaggle_push_runner.sh [TIMEOUT_SECS]
#   TIMEOUT_SECS defaults to 7200 (2 h session cap; jobs run sequentially).
#
# Pipeline:
#   1. tar the pre-registered runner config (dotfiles only — binaries are
#      downloaded by the kernel from GitHub releases) into a small
#      runner-config.tar.gz;
#   2. upsert the private dataset hamzakhl/ium-runner (create or version);
#   3. bake the kernel (scripts/kaggle/ium_runner_kernel.py) with the
#      pinned runner version + timeout and push hamzakhl/ium-runner-pool
#      (the push triggers the session);
#   4. poll `kaggle kernels status hamzakhl/ium-runner-pool`.
#
# The runner registers under label `ium-kaggle`; GitHub dispatches queued
# jobs to it while the session is alive. Verify with:
#   python3 - <<'PY'  (runners API) — ium-kaggle-01 should be online
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
STAGE="$ROOT/.kaggle-stage"
TIMEOUT_SECS="${1:-7200}"  # 2 h: one runner serves the whole ubuntu lane sequentially
RUNNER_VER="$(cat "$STAGE/.runner_ver")"

# credentials: the CLI reads the KGAT token from ~/.kaggle/access_token
# (written from .secrets/kaggle_token — never committed, never embedded
# in the kernel).
command -v kaggle >/dev/null 2>&1 || pip install -q kaggle
if [ ! -f "$HOME/.kaggle/access_token" ]; then
  mkdir -p "$HOME/.kaggle"
  cp /home/z/my-project/.secrets/kaggle_token "$HOME/.kaggle/access_token"
  chmod 600 "$HOME/.kaggle/access_token"
fi

# --- 1. pack the runner config (dotfiles only) -----------------------------
ARDIR="$STAGE/actions-runner"
for f in .runner .credentials .credentials_rsaparams; do
  [ -f "$ARDIR/$f" ] || { echo "missing $ARDIR/$f — register the runner first" >&2; exit 2; }
done
tar czf "$STAGE/runner-config.tar.gz" -C "$ARDIR" \
  .runner .credentials .credentials_rsaparams .env .path
echo "[runner-push] config packed: $(du -h "$STAGE/runner-config.tar.gz" | cut -f1)"

# --- 2. upsert the private dataset -----------------------------------------
DS="$STAGE/dataset-runner"
rm -rf "$DS" && mkdir -p "$DS"
cp "$STAGE/runner-config.tar.gz" "$DS/"
if kaggle datasets status hamzakhl/ium-runner >/dev/null 2>&1; then
  cat > "$DS/dataset-metadata.json" <<JSON
{ "id": "hamzakhl/ium-runner", "title": "ium-runner", "is_private": true }
JSON
  kaggle datasets version -p "$DS" -m "runner config refresh" -r zip -q
  echo "[runner-push] dataset versioned"
else
  cat > "$DS/dataset-metadata.json" <<JSON
{
  "id": "hamzakhl/ium-runner",
  "title": "ium-runner",
  "is_private": true,
  "licenses": [{ "name": "CC0-1.0" }]
}
JSON
  kaggle datasets create -p "$DS" -r zip -q
  echo "[runner-push] dataset created"
fi
for i in $(seq 1 60); do
  ST=$(kaggle datasets status hamzakhl/ium-runner 2>&1 | tail -1)
  if echo "$ST" | grep -q "ready"; then echo "[runner-push] dataset ready"; break; fi
  echo "[runner-push] dataset status: $ST — waiting"; sleep 10
done

# --- 3. bake + push the kernel ---------------------------------------------
KDIR="$STAGE/kernel-runner"
rm -rf "$KDIR" && mkdir -p "$KDIR"
sed -e "s/RUNNER_VER_PLACEHOLDER/$RUNNER_VER/" \
    -e "s/TIMEOUT_SECS_PLACEHOLDER/$TIMEOUT_SECS/" \
    "$HERE/kaggle/ium_runner_kernel.py" > "$KDIR/ium_runner_kernel.py"
python3 - "$KDIR" <<'PY'
import json, sys
from pathlib import Path
kd = sys.argv[1]
meta = {
    "id": "hamzakhl/ium-runner-pool",
    "title": "ium-runner-pool",
    "code_file": "ium_runner_kernel.py",
    "language": "python",
    "kernel_type": "script",
    "is_private": "true",
    "enable_gpu": "false",
    "enable_internet": "true",
    "dataset_sources": ["hamzakhl/ium-runner"],
    "competition_sources": [],
    "kernel_sources": [],
}
(Path(kd) / "kernel-metadata.json").write_text(json.dumps(meta, indent=2))
PY
kaggle kernels push -p "$KDIR"
echo "[runner-push] pushed — poll: kaggle kernels status hamzakhl/ium-runner-pool"
