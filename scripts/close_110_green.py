import json
import urllib.request

BASE = "https://api.github.com/repos/kahlanhamza/IUM/issues/110"


def api(url: str, method: str = "GET", payload: dict | None = None) -> dict:
    tok = open("/home/z/my-project/.secrets/gh_token").read().strip()
    req = urllib.request.Request(
        url,
        data=json.dumps(payload).encode() if payload is not None else None,
        headers={"Authorization": f"Bearer {tok}", "Accept": "application/vnd.github+json"},
        method=method,
    )
    with urllib.request.urlopen(req) as r:
        return json.load(r)


def main() -> None:
    body = open("/home/z/my-project/scripts/close110.md").read()
    out = api(f"{BASE}/comments", "POST", {"body": body})
    print(f"comment id={out['id']} url={out['html_url']}")
    r = api(BASE, "PATCH", {"state": "closed", "state_reason": "completed"})
    print(f"issue state={r['state']} state_reason={r['state_reason']} closed_at={r['closed_at']}")


if __name__ == "__main__":
    main()
