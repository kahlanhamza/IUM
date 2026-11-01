#!/usr/bin/env bash
# Snapshot ALL project work and push to github.com/kahlanhamza/IUM
# Token is read from .secrets/gh_token (never stored in repo, never printed).
set -euo pipefail
cd /home/z/my-project
git add -A
if git diff --cached --quiet; then
  echo "[preserve] nothing new to commit"
  exit 0
fi
git commit -m "preserve: session snapshot $(date -u '+%Y-%m-%d %H:%M UTC')"
bash /home/z/my-project/scripts/gh-push.sh
