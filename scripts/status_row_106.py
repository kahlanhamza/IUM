#!/usr/bin/env python3
"""Insert the #106 incident row into the session log and finalize the header."""
P = "STATUS.md"

ROW = (
    "| 2026-11-11 (the CI mirror incident, Closes #106) "
    "| **The #105 verdict chase hit a genuine infra outage and the runbook worked.** Run 43036333957 (the #105 HEAD): 6/8 jobs green on both attempts while BOTH ubuntu jobs wedged 35+ min on the WebKitGTK apt step — the live log showed apt `Ign:`-ing every package from azure.archive.ubuntu.com at 20 s intervals (the runner image's default mirror). "
    "Re-run-failed-jobs wedged identically; attempt 2 (43044589016) proved the first hardening no-opped — `Get:1 file:/etc/apt/apt-mirrors.txt Mirrorlist` revealed GitHub's noble image routes apt through a LOCAL MIRRORLIST file where the hostname actually lives. Attempt 3 (43049461021) proved the mirrorlist rewrite DID take (Ign lines said archive.ubuntu.com) and the canonical mirror stalled IDENTICALLY — 18+ min vs the **44 s baseline** measured on yesterday's green run. HTTPS endpoints fetched in seconds on the same runner and all archives answered 200 from outside: the runner's **HTTP route to the Ubuntu archives** was dead. "
    "Final fix: apt over **HTTPS to mirrors.edge.kernel.org** (Fastly-CDN), scheme+host rewritten across mirrorlist + deb822 + classic sources, `Acquire::Retries=5` + 15 s fetch timeout. **Run 43052820406 = 8/8 GREEN first try.** LESSON: when CI wedges mid-step, cancel and READ the log before the next guess — two of the three fixes were guessable, only the log showed the mirrorlist; and the baseline (44 s) is what turns 'slow' into 'stalled'. "
    "| run 43052820406 8/8 on `2058327`; #105 + #106 both keyword-closed; GREEN receipts on both issues | 2058327 + af753c5 + ebcbec6 |"
)

with open(P, encoding="utf-8") as f:
    content = f.read()

marker = "|---|---|---|---|\n"
hdr_idx = content.index("## 9. Session log")
sep_idx = content.index(marker, hdr_idx) + len(marker)
content = content[:sep_idx] + ROW + "\n" + content[sep_idx:]

content = content.replace("| #105 in flight | this commit)**", "| #105 + #106 closed on run 43052820406 8/8; the release lane is next (screenshot re-shoot, distro v0.2.3 wording, the version sweep) | this commit)**")

with open(P, "w", encoding="utf-8") as f:
    f.write(content)
print("row + header finalized")
