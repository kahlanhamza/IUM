#!/usr/bin/env python3
"""Refresh the STATUS.md 'Last updated' header line (was stuck at M14)."""
import re

P = "STATUS.md"
new_line = (
    "> Last updated: **(THE PRE-RELEASE ACCURACY SWEEP, #105 — the owner's release bar "
    "enforced: every claim wire-fact-checked, provider research from the providers' own "
    "official docs. THE DESTINATION LANES ON THE RECORD: #103 closed (provider presets + "
    "guided keys + the S3 Test probe + expiring share links; 535-test battery re-driven "
    "from zero and CI run 43008204325 = 8/8 — the first live CI verdict since the billing "
    "block, so the ADR-007 gate for a v0.2.3 cut is OPEN); #104 closed (the PUBLIC distro "
    "front door answers the provider search, v0.2.2-true). #105 finds and fixes three "
    "classes of drift: (A) the guided region slot never reached the SigV4 scope — "
    "submit() sent no region while the job signer reads env→vault, so a guided Wasabi "
    "eu-central-2 pick was signed us-east-1 and refused; fix = vault write-through "
    "(partial merge, keys untouched) + the #81 hot-apply BEFORE createJob, custom mode "
    "writes nothing (RED comment 6797741553); (B) the provider registry vs the providers' "
    "own 2026 docs: Filebase's general endpoint is s3.filebase.io (.com is IPFS), Wasabi "
    "8→16 regions, DO Spaces 6→13, B2 +us-east-005 / -us-west-003 (uncorroborated); the "
    "engine's parse_target drift guard stays green over every new sample; (C) the private "
    "docs promised public SBOMs/attestations that #102 removed from the public asset set "
    "(v0.2.2 ships 15 assets, none of them provenance) — README Security now says "
    "checksums + minisign signatures, USERGUIDE §1/§2 say what actually ships; badge "
    "329+163 → 346+191 (real counts, re-run); transports rows gained TeraBox·Feishu·MEGA. "
    "KNOWN NEXT (release lane): all six product shots pre-date the provider picker "
    "(re-shot 11-10 11:53 vs picker landing e26a7cb 11-11) — re-shoot at the cut with "
    "shot 02 driving the guided flow; the distro README/USERGUIDE get their v0.2.3-true "
    "wording at the cut per the #104 scope-honesty rule. | #105 in flight | this commit)**"
)

with open(P, encoding="utf-8") as f:
    lines = f.readlines()

for i, line in enumerate(lines):
    if line.startswith("> Last updated:"):
        lines[i] = new_line + "\n"
        break
else:
    raise SystemExit("header line not found")

with open(P, "w", encoding="utf-8") as f:
    f.writelines(lines)
print("header refreshed;", len(new_line), "chars")
