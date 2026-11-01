#!/usr/bin/env bash
# Push helper: feeds the fine-grained PAT to git via a credential helper on the fly.
# The token never appears in the remote URL, in .git/config, or in output.
set -euo pipefail
cd /home/z/my-project
HELPER='!f(){ printf "username=%s\n" kahlanhamza; printf "password=%s\n" "$(cat /home/z/my-project/.secrets/gh_token)"; }; f'
if ! git -c "credential.helper=${HELPER}" push -u origin main 2>&1 | sed -E 's#(https://)[^@/]*@#\1#g'; then
  echo "[gh-push] push rejected — trying rebase onto remote state"
  git -c "credential.helper=${HELPER}" pull --rebase origin main
  git -c "credential.helper=${HELPER}" push -u origin main
fi
echo "[gh-push] done"
