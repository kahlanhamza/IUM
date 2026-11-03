#!/usr/bin/env bash
# realworld-stress.sh — the real-world stress drill (issue #22).
#
# Run UNDER scripts/s3-test.sh --with-realworld FILE [FILE...]: the pinned
# SeaweedFS 4.51 is already booted on a random port, the bucket exists and
# IUM_S3_* are exported. This driver executes the five real-world scenarios
# against the REAL server with the REAL production adapter and collects one
# `EVIDENCE {json}` line per check into rea-lab/realworld/evidence-<ts>.md.
#
# Scenario map:
#   S1  the plain real upload (film → Done → read-back byte-exact)
#   S2  crash & resume: SIGKILL mid-flight, fresh process reattaches,
#       WIRE == size − confirmed_at_kill (zero re-upload)
#   S3  hostile network through scripts/throttle_proxy.py (victim link
#       96 KiB/s for 45 s; production eviction defaults)
#   S4  user actions: pause → frozen wire → resume → Done; cancel →
#       Canceled journaled, server truth: object never completed
#   S5  concurrent fleet: three real films racing through one manager
#
# Between scenarios the bucket is wiped (objects deleted + multiparts
# aborted) so the server never carries cross-scenario state.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$SCRIPT_DIR/.." && pwd)"
OUT="$REPO/rea-lab/realworld"
STAMP="$(date +%Y%m%d-%H%M%S)"
RUN="$OUT/run-$STAMP"
mkdir -p "$RUN"/{s1,s3,s4pause,s4cancel,s5}
EVID="$OUT/evidence-$STAMP.md"

say() { echo "[realworld] $*"; }
die() { echo "[realworld] FAIL: $*" >&2; exit 1; }

FILES=("$@")
[ "${#FILES[@]}" -ge 1 ] || die "no asset files given"
F1="${FILES[0]}"
F2="${FILES[1]:-}"
F3="${FILES[2]:-}"
# S1 carries the SIZE proof (largest asset). The failure-mode scenarios
# (S2 crash, S3 hostile network, S4 pause/cancel) prefer the smaller real
# film: the scenarios are about the failure modes, and SeaweedFS volume
# files never shrink, so multi-scenario disk cost scales with the asset.
FF="${F2:-$F1}"
# The failure-mode scenarios need a MULTI-PART plan: the hostile-network
# victim is partNumber=2 (it must EXIST and be surrounded by healthy peers),
# and S2's kill window needs a frontier that moves in several part-sized
# steps. The planner (chunk.rs) splits with 4 workers and CHUNK_MIN = 5 MiB:
# chunks = min(4, floor(size / 5 MiB)). So the failure asset must be >= 20
# MiB — smaller files plan 1–3 chunks and the drill would silently run
# unimpeded (observed: a 10 MiB asset plans ONE chunk → no partNumber=2 → no
# victim → no eviction possible). Fail loud instead of lying.
FF_SIZE="$(stat -c%s "$FF")"
if [ "$FF_SIZE" -lt $((20 * 1024 * 1024)) ]; then
  die "failure asset $(basename "$FF") is ${FF_SIZE} B — below the 20 MiB floor \
(4 workers x 5 MiB CHUNK_MIN): the plan would not even contain a victim part 2"
fi
BASE="${IUM_S3_ENDPOINT:?IUM_S3_ENDPOINT not set}"
AK="${IUM_S3_ACCESS_KEY:?}"
SK="${IUM_S3_SECRET_KEY:?}"
REGION="${IUM_S3_REGION:-us-east-1}"
# S3 victim-link profile (owner reality 2026-11-03: uplinks are KILOBYTES per
# second — a 29 MB/s localhost number is engine overhead, not a network).
# Defaults: 98304 B/s = 96 KiB/s for 45 s. Override to model your real link,
# e.g. IUM_S3_VICTIM_RATE=32768 (32 KiB/s) IUM_S3_VICTIM_SECONDS=60.
VICTIM_RATE="${IUM_S3_VICTIM_RATE:-98304}"
VICTIM_SECONDS="${IUM_S3_VICTIM_SECONDS:-45}"

: > "$EVID"
collect() { # append every EVIDENCE line of a command's output to the record
  local out="$1"
  grep '^EVIDENCE ' <<<"$out" | while read -r line; do
    echo "$line" >> "$EVID"
    echo "$line"
  done || true
}

PY3=python3

sha_of() { sha256sum "$1" | cut -d' ' -f1; }

# --- signed helpers over scripts/s3curl.py (stdlib SigV4, no CLI deps) -----
s3_status() { # METHOD URL → HTTP status (use HEAD where the body isn't needed)
  "$PY3" - "$1" "$2" <<'EOF'
import sys
sys.path.insert(0, "scripts")
import os, s3curl
st, _, _ = s3curl.request(sys.argv[1], sys.argv[2],
                          os.environ["IUM_S3_ACCESS_KEY"],
                          os.environ["IUM_S3_SECRET_KEY"],
                          os.environ.get("IUM_S3_REGION", "us-east-1"))
print(st)
EOF
}
s3_get_to() { # URL OUTFILE → status; body written to OUTFILE
  "$PY3" - "$1" "$2" <<'EOF'
import sys
sys.path.insert(0, "scripts")
import os, s3curl
st, body, _ = s3curl.request("GET", sys.argv[1],
                             os.environ["IUM_S3_ACCESS_KEY"],
                             os.environ["IUM_S3_SECRET_KEY"],
                             os.environ.get("IUM_S3_REGION", "us-east-1"))
if 200 <= st < 300:
    with open(sys.argv[2], "wb") as f:
        f.write(body)
print(st)
EOF
}
wipe_bucket() { # delete every object, abort every multipart
  # SeaweedFS buckets are filer directories: a filer-native recursive delete
  # is exact and needs no S3 listing (the S3 v1 listing proved flaky here).
  # The filer binds its default port 8888 (the harness never overrides it).
  if [ -n "${IUM_FILER_URL:-}" ]; then
    curl -fsS -X DELETE "${IUM_FILER_URL}/${BASE##*/}?recursive=true" >/dev/null 2>&1 || true
    "$PY3" "$SCRIPT_DIR/s3curl.py" PUT "$BASE" --ak "$AK" --sk "$SK" --region "$REGION" >/dev/null
  fi
  "$PY3" "$SCRIPT_DIR/s3curl.py" --abort-multiparts "$BASE" --ak "$AK" --sk "$SK" --region "$REGION" >/dev/null
}
free_port() { $PY3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()'; }
frontier() { # JOURNAL_DB → Σ bytes_confirmed (read-only peek into the WAL)
  $PY3 - "$1" <<'EOF'
import sys, sqlite3
try:
    conn = sqlite3.connect(f"file:{sys.argv[1]}?mode=ro", uri=True)
    row = conn.execute("SELECT COALESCE(SUM(bytes_confirmed),0) FROM chunks").fetchone()
    print(row[0])
except sqlite3.Error:
    print(0)
EOF
}

run_bin() { # scenario args... → captured stdout+stderr (builds once, runs quietly;
  # stderr is captured so a bin failure prints its REASON, issue #29)
  cargo run -q -p ium-core --bin realworld_stress -- "$@" 2>&1
}
# S2 kills the upload process itself — run the built binary directly so the
# SIGKILL lands on the uploader, not on a cargo wrapper.
CHILD_BIN="$REPO/target/debug/s3_drill_child"
cargo build -q -p ium-core --bin s3_drill_child

# --- asset facts into the record -------------------------------------------
{
  echo "# real-world stress evidence — $STAMP (issue #22)"
  echo
  i=1
  for f in "${FILES[@]}"; do
    echo "- asset $i: $(basename "$f") — $(stat -c%s "$f") bytes — sha256 $(sha_of "$f")"
    i=$((i + 1))
  done
  echo "- server: pinned SeaweedFS 4.51 (real), adapter: production S3MultipartAdapter (real SigV4)"
  echo
} >> "$EVID"

# ===========================================================================
say "S1: plain real upload"
# ===========================================================================
S1_KEY="$BASE/rw-s1/$(basename "$F1")"
S1_SHA="$(sha_of "$F1")"
OUT_S1="$(run_bin s1 --journal "$RUN/s1" --file "$F1" --endpoint "$S1_KEY")" || die "S1 bin failed: $OUT_S1"
collect "$OUT_S1"
st="$(s3_status HEAD "$S1_KEY")"
[ "$st" = "200" ] || die "S1: object HEAD -> $st"
s3_get_to "$S1_KEY" "$RUN/s1-received.mp4" >/dev/null
S1_RSHA="$(sha_of "$RUN/s1-received.mp4")"
[ "$S1_RSHA" = "$S1_SHA" ] || die "S1: read-back sha256 mismatch ($S1_RSHA)"
echo "EVIDENCE {\"scenario\":\"s1-readback\",\"sha256_ok\":true}" | tee -a "$EVID"
rm -f "$RUN/s1-received.mp4"   # read-back copy is evidence-checkable, not keeper-sized
say "S1 GREEN — read-back byte-exact"
wipe_bucket

# ===========================================================================
say "S2: crash & resume (SIGKILL mid-flight)"
# ===========================================================================
S2_SHA="$(sha_of "$FF")"
S2_SIZE="$(stat -c%s "$FF")"
S2_DONE=0
for ATTEMPT in 1 2 3; do
  JDB="$RUN/s2-$ATTEMPT.journal.db"
  say "S2 attempt $ATTEMPT: child uploads plainly, parent kills mid-flight"
  # --allow-calls 1: DETERMINISTIC mid-flight crash shape. Exactly one part
  # completes; every later upload call wedges at ENTRY (bytes sent: zero) so
  # the journal sits stable at 25% confirmed with three spans unconfirmed —
  # the kill lands whenever the parent polls, on ANY link speed. (Plain
  # racing kills cannot work on fast links: with parallel workers all parts
  # confirm in ONE step, and the first cloud run burned all 3 attempts that
  # way, the post-completion kill hitting an already-Done journal.) A frozen
  # then killed process IS the "laptop died" shape; the resume still exercises
  # the real path: reattach from the journaled probe, zero re-upload of the
  # confirmed part, verify-repair if the server holds a duplicate.
  "$CHILD_BIN" upload --journal "$JDB" --file "$FF" --allow-calls 1 \
    > "$RUN/s2-$ATTEMPT-child.log" 2>&1 &
  CHILD=$!
  KILLED=0
  FRONTIER=0
  for _ in $(seq 1 1800); do
    if ! kill -0 "$CHILD" 2>/dev/null; then break; fi
    FRONTIER="$(frontier "$JDB")"
    if [ "$FRONTIER" -ge $((S2_SIZE / 4)) ]; then
      kill -9 "$CHILD" 2>/dev/null || true
      KILLED=1
      break
    fi
    sleep 0.05
  done
  wait "$CHILD" 2>/dev/null || true
  if [ "$KILLED" != 1 ]; then
    say "S2 attempt $ATTEMPT: child finished before the kill landed — honest retry"
    rm -f "$JDB"
    wipe_bucket
    continue
  fi
  # The child is dead: the journal is final. Re-read the frontier NOW (the
  # poll reading can be stale by one window — more parts may have confirmed
  # between the last poll and the SIGKILL).
  FRONTIER="$(frontier "$JDB")"
  say "S2: killed at frontier $FRONTIER / $S2_SIZE bytes"
  OUT_R="$($CHILD_BIN resume --journal "$JDB")" || die "S2 resume failed: $OUT_R"
  WIRE="$(grep -oE '^WIRE [0-9]+' <<<"$OUT_R" | cut -d' ' -f2 || echo 0)"
  DONESTATE="$(grep -oE '^DONE [a-z]+' <<<"$OUT_R" | cut -d' ' -f2 || echo unknown)"
  # Did the #24 verify-repair sweep fire? (journal truth, not string sniffing)
  REPAIRED="$($PY3 - "$JDB" <<'EOF'
import sys, sqlite3
try:
    conn = sqlite3.connect(f"file:{sys.argv[1]}?mode=ro", uri=True)
    print(conn.execute("SELECT COUNT(*) FROM events WHERE kind='verify_repair'").fetchone()[0])
except sqlite3.Error:
    print(0)
EOF
)"
  if [ "$REPAIRED" -ge 1 ]; then
    # the repair re-sends every span onto a fresh multipart: one extra full copy
    EXPECT_WIRE=$((S2_SIZE - FRONTIER + S2_SIZE))
  else
    EXPECT_WIRE=$((S2_SIZE - FRONTIER))
  fi
  if [ "$WIRE" != "$EXPECT_WIRE" ] || [ "$DONESTATE" != "done" ]; then
    die "S2: WIRE=$WIRE (expected $EXPECT_WIRE) DONE=$DONESTATE"
  fi
  # s3_drill_child derives its endpoint from IUM_S3_ENDPOINT verbatim, so
  # parse_target defaults the key to the source BASENAME — the object lives
  # directly under the bucket, not under an rw-s2/ prefix.
  S2_KEY="$BASE/$(basename "$FF")"
  st="$(s3_status HEAD "$S2_KEY")"
  [ "$st" = "200" ] || die "S2: object HEAD -> $st"
  s3_get_to "$S2_KEY" "$RUN/s2-received.mp4" >/dev/null
  [ "$(sha_of "$RUN/s2-received.mp4")" = "$S2_SHA" ] || die "S2: read-back sha256 mismatch"
  echo "EVIDENCE {\"scenario\":\"s2-crash-resume\",\"attempt\":$ATTEMPT,\"size\":$S2_SIZE,\"confirmed_at_kill\":$FRONTIER,\"wire_resent\":$WIRE,\"expected_wire\":$EXPECT_WIRE,\"verify_repair_swept\":$([ "$REPAIRED" -ge 1 ] && echo true || echo false),\"zero_reupload\":$([ "$REPAIRED" -ge 1 ] && echo false || echo true),\"state\":\"done\",\"readback_sha256_ok\":true}" | tee -a "$EVID"
  rm -f "$RUN/s2-received.mp4"
  S2_DONE=1
  break
done
[ "$S2_DONE" = 1 ] || die "S2: no attempt landed a mid-flight kill (all 3 finished too fast)"
say "S2 GREEN — zero re-upload of confirmed parts"
wipe_bucket

# ===========================================================================
say "S3: hostile network (throttle proxy, production eviction)"
# ===========================================================================
S3_SHA="$(sha_of "$FF")"
S3_PORT="$(free_port)"
SRV_HOSTport="${BASE#http://}"          # 127.0.0.1:PORT/bucket
SRV_HOST="${SRV_HOSTport%%:*}"
SRV_PORT="$(echo "${SRV_HOSTport%%/*}" | cut -d: -f2)"
$PY3 "$SCRIPT_DIR/throttle_proxy.py" "$S3_PORT" "$SRV_HOST" "$SRV_PORT" \
  --victim-part 2 --victim-rate "$VICTIM_RATE" --victim-seconds "$VICTIM_SECONDS" \
  > "$RUN/throttle-proxy.log" 2>&1 &
PROXY_PID=$!
for _ in $(seq 1 50); do
  if $PY3 -c "import socket;s=socket.socket();s.settimeout(0.3);s.connect(('127.0.0.1',$S3_PORT));s.close()" 2>/dev/null; then break; fi
  sleep 0.1
done
say "S3: proxy up on $S3_PORT → $SRV_HOST:$SRV_PORT"
S3_EXIT=0
OUT_S3="$(run_bin s3hostile --journal "$RUN/s3" --file "$FF" --endpoint "http://127.0.0.1:$S3_PORT/$(basename "$BASE")/rw-s3/$(basename "$FF")")" || S3_EXIT=1
kill "$PROXY_PID" 2>/dev/null || true
wait "$PROXY_PID" 2>/dev/null || true
if [ "$S3_EXIT" != 0 ]; then die "S3 bin failed: $OUT_S3"; fi
collect "$OUT_S3"
# Extract elapsed_s WITHOUT string interpolation — the evidence line carries
# escaped inner quotes (the eviction payloads), and embedding captured output
# inside a python -c triple-quote mangles them (JSONDecodeError). stdin is
# quote-proof.
ELAPSED="$($PY3 -c 'import json, sys
line = [l for l in sys.stdin.read().splitlines() if l.startswith("EVIDENCE ")][0]
print(json.loads(line[9:])["elapsed_s"])' <<<"$OUT_S3")"
$PY3 - "$ELAPSED" <<'EOF'
import sys
e = float(sys.argv[1])
assert e < 300, f"S3 took {e:.1f}s — the impaired link was waited out, not evicted"
EOF
S3_KEY="$BASE/rw-s3/$(basename "$FF")"
s3_get_to "$S3_KEY" "$RUN/s3-received.mp4" >/dev/null
[ "$(sha_of "$RUN/s3-received.mp4")" = "$S3_SHA" ] || die "S3: read-back sha256 mismatch"
WAIT_OUT=$(( $(stat -c%s "$FF") / 4 / VICTIM_RATE ))
echo "EVIDENCE {\"scenario\":\"s3-readback\",\"sha256_ok\":true,\"elapsed_s\":$ELAPSED,\"victim_rate_Bps\":$VICTIM_RATE,\"wait_out_would_be_s\":$WAIT_OUT}" | tee -a "$EVID"
rm -f "$RUN/s3-received.mp4"
say "S3 GREEN — eviction fired on live wire numbers, video intact"
wipe_bucket

# ===========================================================================
say "S4: user actions (pause/resume then cancel)"
# ===========================================================================
S4_SHA="$(sha_of "$FF")"
# The pause leg runs through a mild uniform throttle (2 MiB/s — issue #29,
# second face): on a runner the loopback finishes the whole film between two
# 25 ms polls and the 10% pause point never exists. At 2 MiB/s the point
# lasts ~4 s — a REAL mid-flight pause on a realistic link, race-free. The
# cancel leg stays on the deterministic in-process wedge (no throttle needed).
S4_PAUSE_RATE=2097152
S4_PORT="$(free_port)"
$PY3 "$SCRIPT_DIR/throttle_proxy.py" "$S4_PORT" "$SRV_HOST" "$SRV_PORT" \
  --victim-part all --victim-rate "$S4_PAUSE_RATE" --victim-seconds 100000 \
  > "$RUN/s4-pause-proxy.log" 2>&1 &
S4_PID=$!
for _ in $(seq 1 50); do
  if $PY3 -c "import socket;s=socket.socket();s.settimeout(0.3);s.connect(('127.0.0.1',$S4_PORT));s.close()" 2>/dev/null; then break; fi
  sleep 0.1
done
OUT_P="$(run_bin s4pause --journal "$RUN/s4pause" --file "$FF" \
  --endpoint "http://127.0.0.1:$S4_PORT/$(basename "$BASE")/rw-s4/pause.mp4")" || { kill "$S4_PID" 2>/dev/null || true; die "S4 pause bin failed: $OUT_P"; }
kill "$S4_PID" 2>/dev/null || true
wait "$S4_PID" 2>/dev/null || true
collect "$OUT_P"
s3_get_to "$BASE/rw-s4/pause.mp4" "$RUN/s4-received.mp4" >/dev/null
[ "$(sha_of "$RUN/s4-received.mp4")" = "$S4_SHA" ] || die "S4: paused job's read-back sha256 mismatch"
echo "EVIDENCE {\"scenario\":\"s4pause-readback\",\"sha256_ok\":true,\"pause_leg_rate_Bps\":$S4_PAUSE_RATE}" | tee -a "$EVID"
rm -f "$RUN/s4-received.mp4"
wipe_bucket

OUT_C="$(run_bin s4cancel --journal "$RUN/s4cancel" --file "$FF" --endpoint "$BASE/rw-s4/cancel.mp4")" || die "S4 cancel bin failed: $OUT_C"
collect "$OUT_C"
st="$(s3_status HEAD "$BASE/rw-s4/cancel.mp4")"
[ "$st" = "404" ] || die "S4: canceled job's object exists on the server (status $st) — a canceled upload must never complete"
echo "EVIDENCE {\"scenario\":\"s4cancel-server-truth\",\"head_status\":$st,\"object_never_completed\":true}" | tee -a "$EVID"
say "S4 GREEN — Paused froze the wire, Canceled never lied"
wipe_bucket

# ===========================================================================
say "S5: concurrent fleet (three real films)"
# ===========================================================================
if [ -n "$F3" ]; then
  S5_FILES=("$F1" "$F2" "$F3")
elif [ -n "$F2" ]; then
  S5_FILES=("$F1" "$F2")
else
  say "S5 skipped loud: fewer than 2 assets provided"
  S5_FILES=()
fi
if [ "${#S5_FILES[@]}" -ge 2 ]; then
  OUT_5="$(run_bin s5 --journal "$RUN/s5" --bucket-base "$BASE" --file "${S5_FILES[0]}" --file "${S5_FILES[1]}" $([ -n "${S5_FILES[2]:-}" ] && echo --file "${S5_FILES[2]}") )" || die "S5 bin failed: $OUT_5"
  collect "$OUT_5"
  i=1
  for f in "${S5_FILES[@]}"; do
    st="$(s3_status HEAD "$BASE/$(basename "$f")")"
    [ "$st" = "200" ] || die "S5: object $i HEAD -> $st"
    s3_get_to "$BASE/$(basename "$f")" "$RUN/s5-received-$i.mp4" >/dev/null
    [ "$(sha_of "$RUN/s5-received-$i.mp4")" = "$(sha_of "$f")" ] || die "S5: read-back sha256 mismatch for $(basename "$f")"
    rm -f "$RUN/s5-received-$i.mp4"
    i=$((i + 1))
  done
  echo "EVIDENCE {\"scenario\":\"s5-readback\",\"files\":${#S5_FILES[@]},\"all_sha256_ok\":true}" | tee -a "$EVID"
  say "S5 GREEN — fleet raced, every film byte-exact"
  wipe_bucket
fi

# ===========================================================================
say "S6: worse-case ladder — uniform B/s-class uplink (issue #28)"
# ===========================================================================
# The owner's measured reality (2026-11-03): the uplink is plain BYTES per
# second, worse than any kbps-class model. Each leg chokes EVERY UploadPart
# PUT (the whole link, not one peer) and runs the fleet-1 plan TIMEBOXED —
# at 512 B/s a 30 MB film takes ~17 h, so the leg proves the ENGINE (resplit
# fires once, floor respected, cancel honest, wire actually paced) not the
# wire's patience. Set IUM_S6_RATES="32768,2048,512" to run the ladder;
# empty/unset skips the leg (victim-mode bench only).
IUM_S6_RATES="${IUM_S6_RATES:-}"
if [ -n "$IUM_S6_RATES" ]; then
  S6_SECONDS="${IUM_S6_SECONDS:-120}"
  IFS=',' read -r -a S6_RATES <<< "$IUM_S6_RATES"
  for R in "${S6_RATES[@]}"; do
    R="${R//[[:space:]]/}"
    [ -n "$R" ] || continue
    say "S6[$R]: uniform uplink ${R} B/s, fleet 1, timebox ${S6_SECONDS}s, then cancel"
    mkdir -p "$RUN/s6-$R"   # the bin opens <journal-dir>/journal.db
    S6_PORT="$(free_port)"
    $PY3 "$SCRIPT_DIR/throttle_proxy.py" "$S6_PORT" "$SRV_HOST" "$SRV_PORT" \
      --victim-part all --victim-rate "$R" --victim-seconds 100000 \
      > "$RUN/s6-proxy-$R.log" 2>&1 &
    S6_PID=$!
    for _ in $(seq 1 50); do
      if $PY3 -c "import socket;s=socket.socket();s.settimeout(0.3);s.connect(('127.0.0.1',$S6_PORT));s.close()" 2>/dev/null; then break; fi
      sleep 0.1
    done
    S6_EXIT=0
    OUT_S6="$(run_bin s6worse --journal "$RUN/s6-$R" --file "$FF" \
      --endpoint "http://127.0.0.1:$S6_PORT/$(basename "$BASE")/rw-s6-$R/$(basename "$FF")" \
      --rate "$R" --seconds "$S6_SECONDS" 2>&1)" || S6_EXIT=1
    # Grace: the canceled job's FINs must reach the proxy and its handlers
    # must log their VICTIM-WIRE teardown lines before the process dies.
    sleep 2
    kill "$S6_PID" 2>/dev/null || true
    wait "$S6_PID" 2>/dev/null || true
    if [ "$S6_EXIT" != 0 ]; then die "S6[$R] bin failed: $OUT_S6"; fi
    collect "$OUT_S6"
    # Wire honesty (the only meter that cannot lie): the proxy's relayed-byte
    # counter must show ~rate x timebox actually pumped — lower bound proves
    # the link really was paced; upper bound proves the throttle never leaked.
    # The counter comes from VICTIM-FINAL (SIGTERM dump) / VICTIM-TICK lines:
    # a canceled engine's FIN queues behind ~1 MB of unread paced data, so
    # teardown-based accounting would wait an hour at 512 B/s (issue #28).
    $PY3 - "$R" "$S6_SECONDS" "$RUN/s6-proxy-$R.log" <<'EOF'
import re, sys
rate, tb, path = int(sys.argv[1]), float(sys.argv[2]), sys.argv[3]
final = tick = wire = 0
for ln in open(path):
    m = re.search(r"VICTIM-FINAL moved=(\d+)B", ln)
    if m: final = int(m.group(1))
    m = re.search(r"VICTIM-TICK moved=(\d+)B", ln)
    if m: tick = max(tick, int(m.group(1)))
    m = re.search(r"VICTIM-WIRE moved=(\d+)B", ln)
    if m: wire += int(m.group(1))
moved = max(final, tick, wire)
lo = rate * tb / 4.0
# upper bound budgets the honest kernel-buffer burst (~1 MB: the client may
# push that into socket buffers before pacing backpressure clamps it) — an
# UNTHROTTLED leg moves 22+ MB in a 60 s box, so this stays decisive.
hi = rate * tb * 1.5 + rate * 60 + 1024 * 1024
assert moved >= lo, f"S6[{rate}] wire: moved {moved} B < {lo:.0f} B lower bound "\
                    f"(rate {rate} x timebox {tb}s) — the link was NOT paced"
assert moved <= hi, f"S6[{rate}] wire: moved {moved} B > {hi:.0f} B upper bound — the throttle leaked"
print(f"[realworld] S6[{rate}] wire honesty OK: {moved} B relayed at the paced rate")
EOF
    S6_KEY="$BASE/rw-s6-$R/$(basename "$FF")"
    st="$(s3_status HEAD "$S6_KEY")"
    [ "$st" = "404" ] || die "S6[$R]: canceled job's object exists (status $st) — a canceled upload must never complete"
    echo "EVIDENCE {\"scenario\":\"s6worse-server-truth\",\"rate_Bps\":$R,\"head_status\":$st,\"object_never_completed\":true}" | tee -a "$EVID"
    say "S6[$R] GREEN — resplit fired on a B/s-class link, floor respected, cancel honest"
    wipe_bucket
  done
else
  say "S6 skipped: IUM_S6_RATES not set (victim-mode bench only)"
fi

# ===========================================================================
say "S7: real-network models — loss, RTT, reset (issue #32)"
# ===========================================================================
# Impairment beyond bandwidth, the application-observable of each (a userspace
# relay cannot drop in-stream bytes): loss = RTO-scale stalls on a paced lane;
# rtt = 250 ms ± jitter per connection, NO throttle; reset = a real one-shot
# mid-body RST. Set IUM_S7_MODELS="loss,rtt,reset" to run the legs; empty/
# unset skips (the default until this issue closes on a cloud run).
IUM_S7_MODELS="${IUM_S7_MODELS:-}"
if [ -n "$IUM_S7_MODELS" ]; then
  S7_LOSS_PCT="${IUM_S7_LOSS_PCT:-2.0}"
  S7_LOSS_RATE="${IUM_S7_LOSS_RATE:-2097152}"     # the loss lane rides a 2 MiB/s pace
  S7_RTT_MS="${IUM_S7_RTT_MS:-250}"
  S7_JITTER_MS="${IUM_S7_JITTER_MS:-50}"
  S7_RESET_BYTES="${IUM_S7_RESET_BYTES:-1048576}" # 1 MiB into the first part PUT
  IFS=',' read -r -a S7_MODELS <<< "$IUM_S7_MODELS"
  for MODEL in "${S7_MODELS[@]}"; do
    MODEL="${MODEL//[[:space:]]/}"
    [ -n "$MODEL" ] || continue
    say "S7[$MODEL]: real-network model leg"
    mkdir -p "$RUN/s7-$MODEL"
    S7_PORT="$(free_port)"
    case "$MODEL" in
      loss)  S7_FLAGS=(--victim-part all --victim-rate "$S7_LOSS_RATE" --victim-seconds 100000 --loss "$S7_LOSS_PCT") ;;
      rtt)   S7_FLAGS=(--victim-part none --latency-ms "$S7_RTT_MS" --jitter-ms "$S7_JITTER_MS") ;;
      reset) S7_FLAGS=(--victim-part none --reset-after-bytes "$S7_RESET_BYTES") ;;
      *) die "S7: unknown model '$MODEL' (loss|rtt|reset)" ;;
    esac
    $PY3 "$SCRIPT_DIR/throttle_proxy.py" "$S7_PORT" "$SRV_HOST" "$SRV_PORT" \
      "${S7_FLAGS[@]}" > "$RUN/s7-proxy-$MODEL.log" 2>&1 &
    S7_PID=$!
    for _ in $(seq 1 50); do
      if $PY3 -c "import socket;s=socket.socket();s.settimeout(0.3);s.connect(('127.0.0.1',$S7_PORT));s.close()" 2>/dev/null; then break; fi
      sleep 0.1
    done
    S7_EXIT=0
    OUT_S7="$(run_bin s7net --journal "$RUN/s7-$MODEL" --file "$FF" \
      --endpoint "http://127.0.0.1:$S7_PORT/$(basename "$BASE")/rw-s7-$MODEL/$(basename "$FF")" \
      --model "$MODEL" 2>&1)" || S7_EXIT=1
    sleep 2   # grace: the proxy's teardown/wire lines must land before SIGTERM
    kill "$S7_PID" 2>/dev/null || true
    wait "$S7_PID" 2>/dev/null || true
    if [ "$S7_EXIT" != 0 ]; then die "S7[$MODEL] bin failed: $OUT_S7"; fi
    collect "$OUT_S7"
    $PY3 - "$MODEL" "$FF" "$RUN/s7-proxy-$MODEL.log" <<'EOF'
import re, sys
model, film, logpath = sys.argv[1], sys.argv[2], sys.argv[3]
size = __import__("os").path.getsize(film)
log = open(logpath).read()
if model == "loss":
    # The stalls must have REALLY happened on the wire, and the pace must
    # have held: every byte of the film went through the 2 MiB/s lane.
    stalls = sum(int(m) for m in re.findall(r"LOSS stalls=(\d+)", log))
    assert stalls >= 1, "S7[loss]: no LOSS stalls in the proxy log — the model never fired"
    moved = 0
    for ln in log.splitlines():
        m = re.search(r"VICTIM-FINAL moved=(\d+)B", ln)
        if m: moved = max(moved, int(m.group(1)))
        m = re.search(r"VICTIM-TICK moved=(\d+)B", ln)
        if m: moved = max(moved, int(m.group(1)))
    assert moved >= size, f"S7[loss]: wire moved {moved} < film {size} — read-back would lie"
    print(f"[realworld] S7[loss] wire OK: {moved} B through the paced lane, {stalls} RTO stalls absorbed")
elif model == "rtt":
    n = len(re.findall(r"LATENCY ", log))
    assert n >= 2, f"S7[rtt]: only {n} LATENCY pauses in the proxy log — the RTT model never fired"
    print(f"[realworld] S7[rtt] wire OK: {n} RTT pauses applied, bandwidth untouched")
elif model == "reset":
    m = re.search(r"RESET-WIRE moved=(\d+)B", log)
    assert m, "S7[reset]: no RESET-WIRE in the proxy log — the RST never landed"
    moved = int(m.group(1))
    assert log.count("RESET-WIRE") == 1, "S7[reset]: the one-shot latch fired more than once"
    assert moved >= 65536, f"S7[reset]: RST landed at {moved} B — too early to be mid-body"
    print(f"[realworld] S7[reset] wire OK: real mid-body RST at {moved} B, exactly one")
EOF
    S7_KEY="$BASE/rw-s7-$MODEL/$(basename "$FF")"
    if [ "$MODEL" = "reset" ]; then
      # the retry completed the object — server truth must carry it
      st="$(s3_status HEAD "$S7_KEY")"
      [ "$st" = "200" ] || die "S7[reset]: object HEAD -> $st (the retry must complete the upload)"
    fi
    s3_get_to "$S7_KEY" "$RUN/s7-received.mp4" >/dev/null
    [ "$(sha_of "$RUN/s7-received.mp4")" = "$(sha_of "$FF")" ] || die "S7[$MODEL]: read-back sha256 mismatch"
    rm -f "$RUN/s7-received.mp4"
    echo "EVIDENCE {\"scenario\":\"s7net-$MODEL\",\"readback_sha256_ok\":true}" | tee -a "$EVID"
    say "S7[$MODEL] GREEN — byte-exact through the $MODEL model"
    wipe_bucket
  done
else
  say "S7 skipped: IUM_S7_MODELS not set"
fi

# ===========================================================================
{
  echo
  echo "## raw child/resume logs"
  echo
  echo "- s2 attempts: $RUN/s2-*.journal.db (parent log per attempt)"
  echo "- throttle proxy: see $RUN/throttle-proxy.log"
  if [ -n "${IUM_S6_RATES:-}" ]; then echo "- s6 ladder: $RUN/s6-proxy-*.log (VICTIM-WIRE lines are the wire meter)"; fi
  if [ -n "${IUM_S7_MODELS:-}" ]; then echo "- s7 models: $RUN/s7-proxy-*.log (LOSS/RESET-WIRE lines are the wire meter)"; fi
} >> "$EVID"
say "ALL SCENARIOS GREEN — evidence: $EVID"
