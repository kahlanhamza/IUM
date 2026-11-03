#!/usr/bin/env bash
# kaggle_push.sh — dispatch an IUM intensive program to Kaggle compute.
#
# Usage: bash scripts/kaggle_push.sh [smoke|stress|longhaul]
#
# Pipeline: pack the working tree into the private hamzakhl/ium-source
# dataset -> bake the scenario into the kernel -> push (the push triggers
# the run). Artifacts land in the kernel output; poll with
# `kaggle kernels status hamzakhl/ium-intensive` and pull with
# `kaggle kernels output hamzakhl/ium-intensive -p <dir>`.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
SCENARIO="${1:-smoke}"
case "$SCENARIO" in smoke|stress|longhaul) ;; *) echo "unknown scenario: $SCENARIO" >&2; exit 2;; esac

# credentials: the CLI reads the KGAT token from ~/.kaggle/access_token
# (written once from .secrets/kaggle_token — never committed, never
# embedded in the kernel).
command -v kaggle >/dev/null 2>&1 || pip install -q kaggle
if [ ! -f "$HOME/.kaggle/access_token" ]; then
  mkdir -p "$HOME/.kaggle"
  cp /home/z/my-project/.secrets/kaggle_token "$HOME/.kaggle/access_token"
  chmod 600 "$HOME/.kaggle/access_token"
fi

python3 "$HERE/kaggle/prepare_source_dataset.py"

# The dataset must be FULLY processed before a kernel referencing it can
# mount it — a push during processing runs without the input (seen live:
# /kaggle/input missing at t=0.7 s). Poll until ready.
for i in $(seq 1 60); do
  ST=$(kaggle datasets status hamzakhl/ium-source 2>&1 | tail -1)
  if echo "$ST" | grep -q "ready"; then
    echo "[kaggle] dataset ready"
    break
  fi
  echo "[kaggle] dataset status: $ST — waiting"
  sleep 10
done

KDIR="$ROOT/.kaggle-stage/kernel-$SCENARIO"
rm -rf "$KDIR" && mkdir -p "$KDIR"
sed "s/SCENARIO_PLACEHOLDER/$SCENARIO/" "$HERE/kaggle/ium_intensive.py" > "$KDIR/ium_intensive.py"

python3 - "$KDIR" <<'PY'
import json, sys
from pathlib import Path
kd = sys.argv[1]
meta = {
    "id": "hamzakhl/ium-intensive",
    "title": "ium-intensive",
    "code_file": "ium_intensive.py",
    "language": "python",
    "kernel_type": "script",
    "is_private": "true",
    "enable_gpu": "false",
    "enable_internet": "true",
    "dataset_sources": ["hamzakhl/ium-source"],
    "competition_sources": [],
    "kernel_sources": [],
}
(Path(kd) / "kernel-metadata.json").write_text(json.dumps(meta, indent=2))
PY

kaggle kernels push -p "$KDIR"
echo "[kaggle] pushed scenario=$SCENARIO — poll: kaggle kernels status hamzakhl/ium-intensive"
