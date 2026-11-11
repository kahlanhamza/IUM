#!/usr/bin/env python3
"""Verify the v0.2.3 distro release on kahlanhamza/IUM-dist — all unauthenticated (the user's view).

The #107 battery: feed, signatures, checksums, latest, and the provenance
gate (no SBOM/attestation-shaped file in the public asset set — #102).
"""
import hashlib, json, urllib.request

VERSION = '0.2.3'
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
check(f'manifest version == {VERSION}', man.get('version') == VERSION, man.get('version', ''))

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

# 4. SHA256 exact match on every installer the sums file covers
sums_raw = get(f'{BASE}/SHA256SUMS.txt')[1].decode()
sums = {}
for line in sums_raw.splitlines():
    parts = line.split()
    if len(parts) == 2:
        sums[parts[1].strip('*').strip()] = parts[0]
targets = [a for a in sums if a.endswith(('.deb', '.exe', '.dmg', '.AppImage', '.rpm', '.tar.gz'))]
check('SHA256SUMS covers >=6 installers', len(targets) >= 6, f'({len(targets)})')
for asset in targets:
    s3, data = get(f'{BASE}/{asset}')
    got = hashlib.sha256(data).hexdigest()
    check(f'sha256 {asset[:44]}', got == sums.get(asset), f'({len(data)} bytes)')

# 5. the release really is "latest" (metadata via token; the user-view checks above were unauth)
tok = open('.secrets/gh_token').read().strip()
api = urllib.request.Request('https://api.github.com/repos/kahlanhamza/IUM-dist/releases/latest',
                             headers={'User-Agent': 'ium-wire', 'Authorization': f'Bearer {tok}'})
with urllib.request.urlopen(api) as resp:
    rel = json.load(resp)
check(f'latest release is v{VERSION} (not draft/prerelease)',
      rel['tag_name'] == f'v{VERSION}' and not rel['draft'] and not rel['prerelease'])

# 6. the provenance gate (#102): no internal sidecar in the PUBLIC set
names = sorted(a['name'] for a in rel['assets'])
print('  assets:', len(names))
bad = [n for n in names if any(t in n.lower() for t in
       ('sbom', 'attestation', 'spdx', 'cyclonedx', 'cosign', 'intoto', '.jsonl'))]
check('no provenance-shaped asset public', not bad, str(bad))
allow_suffix = ('.deb', '.rpm', '.AppImage', '.dmg', '.msi', '.exe', '.tar.gz',
                '.sig', '.txt', '.json')
bad2 = [n for n in names if not n.endswith(allow_suffix)]
check('every asset is installer/sig/sums/feed shaped', not bad2, str(bad2))
check('SHA256SUMS.txt + latest.json present',
      'SHA256SUMS.txt' in names and 'latest.json' in names)

print('RESULT:', 'ALL GREEN' if not fails else f'{len(fails)} FAILURES: {fails}')
raise SystemExit(1 if fails else 0)
