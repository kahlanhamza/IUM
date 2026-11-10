#!/usr/bin/env python3
"""Set the DISTRO_TOKEN Actions secret on kahlanhamza/IUM (#101).

The release publish job ships the release cross-repo onto the PUBLIC distro
repo (kahlanhamza/IUM-dist); GITHUB_TOKEN is source-repo-scoped, so the job
authenticates with this PAT instead. Value: read from .secrets/gh_token
(the owner's fine-grained PAT, verified to hold push on BOTH repos) —
never printed, sealed with the repo's public key (libsodium sealed box).
"""
import json
import urllib.request
from pathlib import Path

REPO = "kahlanhamza/IUM"
TOKEN = Path("/home/z/my-project/.secrets/gh_token").read_text().strip()


def api(method: str, path: str, payload: dict | None = None):
    req = urllib.request.Request(
        f"https://api.github.com/repos/{REPO}/{path}",
        data=json.dumps(payload).encode() if payload is not None else None,
        method=method,
        headers={
            "Authorization": f"Bearer {TOKEN}",
            "Accept": "application/vnd.github+json",
        },
    )
    with urllib.request.urlopen(req) as resp:
        return json.load(resp)


def main() -> int:
    from nacl.encoding import Base64Encoder
    from nacl.public import PublicKey, SealedBox

    pk = api("GET", "actions/secrets/public-key")
    sealed = SealedBox(PublicKey(pk["key"].encode(), Base64Encoder())).encrypt(
        TOKEN.encode()
    )
    import base64
    api("PUT", "actions/secrets/DISTRO_TOKEN", {
        "encrypted_value": base64.b64encode(sealed).decode(),
        "key_id": pk["key_id"],
    })
    names = [s["name"] for s in api("GET", "actions/secrets?per_page=30")["secrets"]]
    print(f"DISTRO_TOKEN set; repo secrets now: {sorted(names)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
