#!/usr/bin/env python3
"""Poll a workflow run until completion; print job verdicts. Usage: poll_run.py <run_id> [max_seconds]"""
import json, sys, time, urllib.request

tok = open('.secrets/gh_token').read().strip()
run_id = sys.argv[1]
max_s = int(sys.argv[2]) if len(sys.argv) > 2 else 540
base = f'https://api.github.com/repos/kahlanhamza/IUM/actions'

def api(url):
    r = urllib.request.Request(url, headers={'Authorization': f'Bearer {tok}', 'User-Agent': 'ium-wire'})
    with urllib.request.urlopen(r) as resp:
        return json.load(resp)

deadline = time.time() + max_s
last = ''
while time.time() < deadline:
    run = api(f'{base}/runs/{run_id}')
    state = f"{run['status']}/{run['conclusion']}"
    if state != last:
        print(f"[{time.strftime('%H:%M:%S')}] run {run_id}: {state}", flush=True)
        last = state
    if run['status'] == 'completed':
        jobs = api(f'{base}/runs/{run_id}/jobs?per_page=40')
        bad = 0
        for j in jobs['jobs']:
            ok = j['conclusion'] in ('success', 'skipped')
            bad += 0 if ok else 1
            if not ok:
                print(f"  FAIL {j['name']}")
        print('VERDICT:', 'GREEN' if bad == 0 else f'RED ({bad} jobs failed)')
        sys.exit(0 if bad == 0 else 1)
    time.sleep(45)
print('TIMEOUT: still running')
sys.exit(2)
