#!/usr/bin/env python3
"""Verify the v0.2.2 distro release on kahlanhamza/IUM-dist — all unauthenticated (the user's view)."""
import hashlib, json, urllib.request

BASE = 'https://github.com/kahlanhamza/IUM-dist/releases/latest/download'

def get(url):
    r = urllib.request.Request(url, headers={'User-Agent': 'ium-wire-unauth'})
    with urllib.request.urlopen(r) as resp:
        return resp.status, resp.read()

fails = []
def check(name, cond, detail=''):
    print(('  OK  ' if cond else '  FAIL'), name, detail)
    if not cond:
        fails.append(name)

# 1. latest.json unauthenticated
s, raw = get(f'{BASE}/latest.json')
check('latest.json unauthenticated 200', s == 200, f'(status {s})')
man = json.loads(raw)
check('manifest version == 0.2.2', man.get('version') == '0.2.2', man.get('version', ''))

# 2. manifest platform keys = shipped installers
plats = man.get('platforms', {})
print('  platforms:', sorted(plats.keys()))
check('>=3 platforms', len(plats) >= 3)
for k, v in sorted(plats.items()):
    check(f'  {k}: has signature+url', bool(v.get('signature')) and 'IUM-dist' in v.get('url', ''))

# 3. signature in manifest == the .sig asset bytes (per platform)
for k, v in sorted(plats.items()):
    asset = v['url'].rsplit('/', 1)[1]
    s2, sig = get(f'{BASE}/{asset}.sig' if not asset.endswith('.sig') else f'{BASE}/{asset}')
    check(f'  {k}: manifest sig == {asset}.sig bytes', sig and sig.decode().strip() == v['signature'].strip())

# 4. SHA256 exact match on one real installer per OS family
sums_raw = get(f'{BASE}/SHA256SUMS.txt')[1].decode()
sums = {}
for line in sums_raw.splitlines():
    parts = line.split()
    if len(parts) == 2:
        sums[parts[1].strip('*').strip()] = parts[0]
targets = [a for a in sums if a.endswith(('.deb', '.exe', '.dmg', '.AppImage'))]
for asset in targets[:4]:
    s3, data = get(f'{BASE}/{asset}')
    got = hashlib.sha256(data).hexdigest()
    check(f'sha256 {asset[:44]}', got == sums.get(asset), f'({len(data)} bytes)')

# 5. the release really is "latest" (metadata via token; the user-view checks above were unauth)
tok = open('.secrets/gh_token').read().strip()
api = urllib.request.Request('https://api.github.com/repos/kahlanhamza/IUM-dist/releases/latest',
                             headers={'User-Agent': 'ium-wire', 'Authorization': f'Bearer {tok}'})
with urllib.request.urlopen(api) as resp:
    rel = json.load(resp)
check('latest release is v0.2.2 (not draft/prerelease)',
      rel['tag_name'] == 'v0.2.2' and not rel['draft'] and not rel['prerelease'])
check('asset count', len(rel['assets']) > 0, f'({len(rel["assets"])} assets)')

print('RESULT:', 'ALL GREEN' if not fails else f'{len(fails)} FAILURES: {fails}')
