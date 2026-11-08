#!/usr/bin/env python3
"""
IUM "Ink" design-language mockups (approval artifacts — NOT app code).

Generates 3 high-fidelity static mockups of the redesigned desktop app:
  1. light.html — main window, light theme (library + dense transfer queue)
  2. dark.html  — main window, dark theme, docked inspector pane open
  3. settings.html — settings as a centered modal over a smoke scrim

Design system embodied here (researched: Fluent 2, Apple HIG, Raycast,
Linear case studies — see the proposal message):
  - unified compact toolbar (no web masthead), sidebar + content + status bar
  - edge-to-edge 36px rows, hairline dividers, zero floating cards
  - two primaries only: logo ink #0F172A + white; red reserved for failures
  - Inter (UI) + JetBrains Mono (machine data, tabular numerals)
  - radii 4–10px, no drop shadows inside the window (hairlines carry edges)
"""
import os
from string import Template

OUT = "/home/z/my-project/design/mockup"
FONTDIR = os.path.join(OUT, "node_modules", "@fontsource")

# ---------------------------------------------------------------- icons
# Minimal 24px stroke icons, stroke-width 1.5, round caps — the round caps
# deliberately echo the ink-dart logomark geometry.
IC = {
    "upload": '<path d="M12 15.5V4.5M7.5 9L12 4.5L16.5 9M5 15.5v3h14v-3"/>',
    "download": '<path d="M12 4.5v11M7.5 11L12 15.5L16.5 11M5 17v2.5h14V17"/>',
    "wmin": '<path d="M5.5 12h13"/>',
    "wmax": '<rect x="6" y="6" width="12" height="12" rx="1.8"/>',
    "wclose": '<path d="M6.5 6.5l11 11M17.5 6.5l-11 11"/>',
    "play": '<path d="M8.5 5.8v12.4L18.5 12z" fill="currentColor" stroke="none"/>',
    "pause": '<path d="M8.5 5.5v13M15.5 5.5v13"/>',
    "trash": '<path d="M5 7h14M10 4.5h4M6.8 7l.8 12.5h8.8L17.2 7M10 10.5v6M14 10.5v6"/>',
    "sliders": '<path d="M4 7h16M4 12h16M4 17h16"/>'
               '<circle cx="14.5" cy="7" r="2.1" fill="var(--chrome)"/>'
               '<circle cx="8" cy="12" r="2.1" fill="var(--chrome)"/>'
               '<circle cx="16.5" cy="17" r="2.1" fill="var(--chrome)"/>',
    "search": '<circle cx="11" cy="11" r="6"/><path d="M15.6 15.6L20 20"/>',
    "doc": '<path d="M7.5 3.5h6.2l4.3 4.3V19a1.5 1.5 0 0 1-1.5 1.5h-9A1.5 1.5 0 0 1 6 19V5a1.5 1.5 0 0 1 1.5-1.5z"/>'
           '<path d="M13.5 3.8V8.2h4.2"/>',
    "media": '<rect x="3.5" y="5.5" width="17" height="13" rx="2.4"/>'
             '<path d="M10.6 9.6v4.8l4.3-2.4z" fill="currentColor" stroke="none"/>',
    "check": '<path d="M5 12.8l4.4 4.4L19 7.4"/>',
    "x": '<path d="M6.5 6.5l11 11M17.5 6.5l-11 11"/>',
    "cloud": '<path d="M7.3 17.5h9.2a3.9 3.9 0 0 0 .7-7.74 5.4 5.4 0 0 0-10.4-1.2A4 4 0 0 0 7.3 17.5z"/>',
    "video": '<rect x="3.5" y="6" width="17" height="12" rx="2.8"/>'
             '<path d="M10.5 9.6v4.8l4.2-2.4z" fill="currentColor" stroke="none"/>',
    "server": '<rect x="4" y="4.5" width="16" height="6" rx="1.6"/>'
              '<rect x="4" y="13.5" width="16" height="6" rx="1.6"/>'
              '<circle cx="7.3" cy="7.5" r="0.4" fill="currentColor"/>'
              '<circle cx="7.3" cy="16.5" r="0.4" fill="currentColor"/>',
    "db": '<ellipse cx="12" cy="5.8" rx="7" ry="2.7"/>'
          '<path d="M5 5.8v12.4c0 1.5 3.1 2.7 7 2.7s7-1.2 7-2.7V5.8"/>'
          '<path d="M5 12c0 1.5 3.1 2.7 7 2.7s7-1.2 7-2.7"/>',
    "plus": '<path d="M12 5.5v13M5.5 12h13"/>',
    "sort": '<path d="M8 10.2l4-4.2 4 4.2z" fill="currentColor" stroke="none"/>',
    "tri": '<path d="M12 5l7.5 13h-15z" fill="currentColor" stroke="none"/>',
    "file": '<path d="M7 3.5h7.2L18.5 7.8V20a.9.9 0 0 1-.9.9H7a.9.9 0 0 1-.9-.9V4.4a.9.9 0 0 1 .9-.9z"/>',
}

def svg(name, size=15, cls=""):
    return (f'<svg class="{cls}" width="{size}" height="{size}" viewBox="0 0 24 24" '
            f'fill="none" stroke="currentColor" stroke-width="1.5" '
            f'stroke-linecap="round" stroke-linejoin="round">{IC[name]}</svg>')

def dart(size, color="currentColor"):
    """The frozen ink-dart logomark (design/logo geometry, #79)."""
    return (f'<svg width="{size}" height="{size}" viewBox="0 0 512 512" fill="none" '
            f'stroke="{color}" stroke-linecap="round">'
            f'<g transform="translate(11 -10.8) scale(1.15)">'
            f'<path stroke-width="46" d="M 214 314.4 A 104 104 0 0 1 261.9 227.4"/>'
            f'<path stroke-width="54" d="M 148 314 A 170 170 0 0 1 278 150"/></g></svg>')

# ---------------------------------------------------------------- CSS
CSS = Template("""
@font-face{font-family:'Inter';font-weight:400;font-style:normal;
  src:url('node_modules/@fontsource/inter/files/inter-latin-400-normal.woff2') format('woff2')}
@font-face{font-family:'Inter';font-weight:500;font-style:normal;
  src:url('node_modules/@fontsource/inter/files/inter-latin-500-normal.woff2') format('woff2')}
@font-face{font-family:'Inter';font-weight:600;font-style:normal;
  src:url('node_modules/@fontsource/inter/files/inter-latin-600-normal.woff2') format('woff2')}
@font-face{font-family:'JetBrains Mono';font-weight:400;font-style:normal;
  src:url('node_modules/@fontsource/jetbrains-mono/files/jetbrains-mono-latin-400-normal.woff2') format('woff2')}
@font-face{font-family:'JetBrains Mono';font-weight:500;font-style:normal;
  src:url('node_modules/@fontsource/jetbrains-mono/files/jetbrains-mono-latin-500-normal.woff2') format('woff2')}

*{box-sizing:border-box;margin:0;padding:0}
html,body{width:100%;height:100%}
body{
  font-family:'Inter',system-ui,sans-serif;
  font-feature-settings:"calt","kern","liga","ss03";
  -webkit-font-smoothing:antialiased;text-rendering:optimizeLegibility;
  background:linear-gradient(140deg,var(--desk1),var(--desk2));
  display:grid;place-items:center;overflow:hidden;user-select:none;
}
:root, [data-theme="light"]{
  --desk1:#EAEDF2; --desk2:#D8DDE5;
  --chrome:#F5F6F8; --content:#FFFFFF; --panel:#FAFBFC;
  --text:#0F172A; --text2:#4A5670; --text3:#8C96A8;
  --line:#E7EAF0; --line2:#D3D9E2;
  --wash:rgba(15,23,42,.055); --track:rgba(15,23,42,.085);
  --fill:#0F172A; --fill-text:#FFFFFF;
  --red:#C43A2B; --red-soft:rgba(196,58,43,.09);
  --scrim:rgba(15,23,42,.35);
  --keycap:rgba(15,23,42,.045);
  --search-bg:#FFFFFF;
  --win-sh:0 42px 90px rgba(15,23,42,.26),0 10px 28px rgba(15,23,42,.10);
  --modal-sh:0 30px 80px rgba(15,23,42,.24),0 6px 20px rgba(15,23,42,.10);
  --gclose:rgba(15,23,42,.06);
}
[data-theme="dark"]{
  --desk1:#1A1E25; --desk2:#0E1116;
  --chrome:#0B101A; --content:#0E1521; --panel:#121A29;
  --text:#EDF1F8; --text2:rgba(237,241,248,.64); --text3:rgba(237,241,248,.38);
  --line:rgba(255,255,255,.075); --line2:rgba(255,255,255,.16);
  --wash:rgba(237,241,248,.07); --track:rgba(237,241,248,.11);
  --fill:#E9EEF6; --fill-text:#0B101A;
  --red:#FF8F86; --red-soft:rgba(255,143,134,.12);
  --scrim:rgba(3,7,13,.55);
  --keycap:rgba(237,241,248,.06);
  --search-bg:rgba(237,241,248,.045);
  --win-sh:0 42px 90px rgba(0,0,0,.55),0 10px 28px rgba(0,0,0,.35);
  --modal-sh:0 30px 80px rgba(0,0,0,.55),0 6px 20px rgba(0,0,0,.4);
  --gclose:rgba(237,241,248,.08);
}

/* ------------------------------------------------------------ window */
.win{width:1280px;height:800px;border-radius:10px;overflow:hidden;
  border:1px solid var(--line2);box-shadow:var(--win-sh);
  display:flex;flex-direction:column;background:var(--content);position:relative}

/* ------------------------------------------------- titlebar / toolbar */
.titlebar{height:48px;flex:none;display:flex;align-items:center;
  padding:0 14px;background:var(--chrome);border-bottom:1px solid var(--line)}
.tl{display:flex;gap:8px;margin-right:20px}
.tl i{width:12px;height:12px;border-radius:50%;display:block;box-shadow:inset 0 0 0 1px rgba(0,0,0,.14)}
.tl i:nth-child(1){background:#FF5F57}.tl i:nth-child(2){background:#FEBC2E}.tl i:nth-child(3){background:#28C840}
.tb{display:flex;align-items:center}
.btn{height:28px;border-radius:6px;display:inline-flex;align-items:center;gap:7px;
  font-size:12.5px;font-weight:500;padding:0 10px;border:1px solid transparent;
  color:var(--text2);background:transparent;white-space:nowrap}
.btn.primary{background:var(--fill);color:var(--fill-text);padding:0 13px;font-weight:500}
.btn.sm{height:24px;font-size:12px;padding:0 10px}
.btn.sm.primary{padding:0 12px}
.btn.ghost{border-color:var(--line2)}
.sep{width:1px;height:18px;background:var(--line);margin:0 9px}
.spacer{flex:1}
.search{width:228px;height:28px;border:1px solid var(--line2);border-radius:6px;
  background:var(--search-bg);display:flex;align-items:center;gap:7px;
  padding:0 6px 0 9px;color:var(--text3);font-size:12.5px}
.key{font-family:'JetBrains Mono';font-size:10px;padding:2.5px 5px;border:1px solid var(--line2);
  border-radius:4px;background:var(--keycap);line-height:1;color:var(--text3)}

/* ------------------------------------------------------------ sidebar */
.main{flex:1;display:flex;min-height:0}
.sidebar{width:210px;flex:none;background:var(--chrome);border-right:1px solid var(--line);
  padding:10px 8px;display:flex;flex-direction:column;gap:1px;overflow:hidden}
.slabel{font-size:10px;font-weight:600;letter-spacing:.9px;color:var(--text3);
  padding:12px 10px 5px;text-transform:uppercase}
.sitem{height:28px;border-radius:6px;display:flex;align-items:center;gap:9px;
  padding:0 8px;color:var(--text2);font-size:13px;flex:none}
.sitem svg{flex:none}
.sitem.active{background:var(--wash);color:var(--text);font-weight:500}
.scount{margin-left:auto;font-family:'JetBrains Mono';font-size:10.5px;color:var(--text3)}
.sdot-on{margin-left:auto;width:6px;height:6px;border-radius:50%;background:var(--fill)}
.sdot-off{margin-left:auto;width:6px;height:6px;border-radius:50%;box-shadow:inset 0 0 0 1.5px var(--text3)}
.sadd{color:var(--text3)}

/* ------------------------------------------------------------ content */
.content{flex:1;display:flex;flex-direction:column;min-width:0;background:var(--content)}
.chead,.row{display:grid;grid-template-columns:minmax(0,1fr) 86px 106px 148px 90px 88px;
  gap:12px;align-items:center;padding:0 16px}
.chead{flex:none;height:30px;border-bottom:1px solid var(--line);
  font-size:10px;font-weight:600;letter-spacing:.8px;text-transform:uppercase;color:var(--text3)}
.chead .h{display:flex;align-items:center;gap:4px}
.chead .r{text-align:right}
.rows{flex:1;overflow:hidden}
.row{height:36px;border-bottom:1px solid var(--line)}
.row:last-child{border-bottom:none}
.row.sel{background:var(--wash)}
.name{display:flex;align-items:center;gap:9px;min-width:0}
.name svg{color:var(--text3);flex:none}
.nm{font-weight:500;color:var(--text);white-space:nowrap;overflow:hidden;text-overflow:ellipsis;font-size:13px}
.num{font-family:'JetBrains Mono';font-size:11px;color:var(--text2);text-align:right;
  font-variant-numeric:tabular-nums}
.dest{font-size:12px;color:var(--text2);white-space:nowrap}
.prog{display:flex;align-items:center;gap:8px}
.track{width:86px;height:4px;border-radius:2px;background:var(--track);overflow:hidden;flex:none}
.fill{height:100%;border-radius:2px;background:var(--fill)}
.pct{font-family:'JetBrains Mono';font-size:10.5px;color:var(--text3);width:32px;text-align:right}
.state{display:flex;align-items:center;gap:7px;font-size:12px;color:var(--text2);white-space:nowrap}
.dot7{width:7px;height:7px;border-radius:50%;flex:none}
.state.up .dot7{background:var(--fill)}
.state.up .st{color:var(--text);font-weight:500}
.state.queue .dot7{box-shadow:inset 0 0 0 1.5px var(--text3)}
.state.fail{color:var(--red)}.state.fail .st{font-weight:500}

/* ---------------------------------------------------------- statusbar */
.status{flex:none;height:28px;display:flex;align-items:center;gap:12px;padding:0 14px;
  border-top:1px solid var(--line);background:var(--chrome);font-size:11.5px;color:var(--text2)}
.status .mono{font-family:'JetBrains Mono';font-size:10.5px;color:var(--text3);font-variant-numeric:tabular-nums}
.stsep{width:1px;height:12px;background:var(--line)}
.stgroup{display:flex;align-items:center;gap:8px}
.stright{margin-left:auto;display:flex;align-items:center;gap:10px}
.edot{width:6px;height:6px;border-radius:50%;background:var(--fill)}
.red{color:var(--red)}

/* ---------------------------------------------------------- inspector */
.inspector{flex:none;height:150px;border-top:1px solid var(--line);background:var(--panel);
  padding:11px 16px 12px;display:flex;flex-direction:column;gap:9px}
.ihead{display:flex;align-items:center;gap:10px}
.iname{font-size:13px;font-weight:600;color:var(--text)}
.ipath{font-family:'JetBrains Mono';font-size:10.5px;color:var(--text3)}
.iclose{margin-left:auto;color:var(--text3);display:flex}
.icols{display:grid;grid-template-columns:290px 1fr 300px;gap:26px;flex:1;min-height:0}
.cap{font-size:10px;font-weight:600;letter-spacing:.8px;color:var(--text3);
  text-transform:uppercase;margin-bottom:7px}
.mline{font-family:'JetBrains Mono';font-size:11px;color:var(--text2);
  font-variant-numeric:tabular-nums;line-height:1.75;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.jseq{color:var(--text3)}
.chunks{display:grid;grid-template-columns:repeat(16,8px);grid-auto-rows:8px;gap:2.5px}
.c{width:8px;height:8px;border-radius:2px;background:var(--track)}
.c.done{background:var(--fill)}
.c.act{background:var(--fill);opacity:.45}

/* -------------------------------------------------------------- modal */
.scrim{position:absolute;inset:0;background:var(--scrim)}
.modal{position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);
  width:640px;background:var(--content);border:1px solid var(--line2);
  border-radius:10px;box-shadow:var(--modal-sh);overflow:hidden}
.mhead{padding:16px 18px 10px;display:flex;align-items:flex-start;justify-content:space-between}
.mtitle{font-size:15px;font-weight:600;color:var(--text)}
.msub{font-size:12px;color:var(--text3);margin-top:3px;line-height:1.45}
.iclosebtn{color:var(--text3);display:flex;padding:3px;border-radius:5px}
.mlist{padding:2px 10px 8px}
.mrow{height:46px;display:flex;align-items:center;gap:11px;padding:0 8px;
  border-bottom:1px solid var(--line)}
.mrow:last-child{border-bottom:none}
.micon{color:var(--text3);display:flex;flex:none}
.mnames{min-width:0}
.mname{font-size:13px;font-weight:500;color:var(--text)}
.mhost{font-family:'JetBrains Mono';font-size:10px;color:var(--text3);margin-top:1.5px}
.mright{margin-left:auto;display:flex;align-items:center;gap:12px}
.mstate{font-size:12px;color:var(--text2)}
.mstate.dim{color:var(--text3)}
.msection{border-top:1px solid var(--line)}
.mfoot{display:flex;align-items:center;padding:11px 18px;border-top:1px solid var(--line);gap:8px}
.mhint{font-size:11px;color:var(--text3);display:flex;align-items:center;gap:6px}
.sw{width:34px;height:20px;border-radius:10px;background:var(--track);position:relative;flex:none}
.sw i{position:absolute;left:2px;top:2px;width:16px;height:16px;border-radius:50%;
  background:var(--content);box-shadow:inset 0 0 0 1px var(--line2)}

/* ------------------------------------------------- per-OS window chrome */
.w-mac{border-radius:10px}
.w-win{border-radius:8px}
.w-gnome{border-radius:12px;border:none}
.caps{display:flex;align-items:center;margin-right:-14px;margin-left:6px}
.capbtn{width:44px;height:30px;display:grid;place-items:center;color:var(--text2)}
.gtitle{position:absolute;left:0;right:0;text-align:center;font-size:13.5px;
  font-weight:600;color:var(--text);pointer-events:none}
.gclose{width:26px;height:26px;border-radius:50%;display:grid;place-items:center;
  color:var(--text2);background:var(--gclose);margin-left:auto}

/* ---------------------------------------------------------- launch page */
body.launch{display:block;background:var(--chrome);overflow:auto}
.lp{max-width:1120px;margin:0 auto;padding:0 44px}
.lpnav{height:64px;display:flex;align-items:center;gap:30px}
.lpbrand{display:flex;align-items:center;gap:9px;font-weight:600;font-size:15px;color:var(--text)}
.lplinks{display:flex;gap:24px;font-size:13px;color:var(--text2)}
.btn-lg{height:40px;padding:0 20px;border-radius:8px;font-size:14px}
.lphero{text-align:center;padding:56px 0 0}
.chip{display:inline-flex;align-items:center;gap:6px;font-family:'JetBrains Mono';font-size:10.5px;
  color:var(--text2);border:1px solid var(--line2);border-radius:999px;padding:5px 12px}
.chip i{width:5px;height:5px;border-radius:50%;background:var(--fill);display:block}
.lph1{font-size:52px;font-weight:600;letter-spacing:-1.4px;color:var(--text);margin:22px 0 12px}
.lpsub{font-size:16px;color:var(--text2);line-height:1.55}
.ctas{display:flex;gap:10px;justify-content:center;margin-top:28px}
.lpmeta{margin-top:16px;font-family:'JetBrains Mono';font-size:11px;color:var(--text3)}
.lptile{width:76px;height:76px;border-radius:20px;background:var(--panel);border:1px solid var(--line2);
  display:grid;place-items:center;margin:46px auto 56px;
  box-shadow:0 0 110px 36px rgba(233,238,246,0.055)}
.lpmx{border-top:1px solid var(--line);padding:38px 0 44px;display:grid;
  grid-template-columns:300px 1fr;gap:64px}
.mxt{font-size:20px;font-weight:600;color:var(--text)}
.mxs{font-size:13px;color:var(--text2);margin-top:6px;line-height:1.5}
.mxn{font-family:'JetBrains Mono';font-size:10.5px;color:var(--text3);margin-top:14px;line-height:1.6}
.mxcols{display:grid;grid-template-columns:1fr 1fr 1fr;gap:30px}
.mxh{font-size:10.5px;font-weight:600;letter-spacing:.9px;text-transform:uppercase;
  color:var(--text3);padding-bottom:9px;border-bottom:1px solid var(--line2)}
.mxrow{height:37px;display:flex;align-items:center;justify-content:space-between;
  border-bottom:1px solid var(--line);font-size:13px;color:var(--text);white-space:nowrap}
.lptrust{border-top:1px solid var(--line);padding:22px 0;display:flex;gap:40px;
  font-size:12.5px;color:var(--text2)}
.lptrust span{display:flex;align-items:center;gap:8px}
.lptrust svg{color:var(--fill)}
.lpfoot{border-top:1px solid var(--line);padding:20px 0 30px;display:flex;align-items:center;
  font-size:12px;color:var(--text3)}
.lpfoot .r{margin-left:auto;display:flex;gap:20px}
""")

# ---------------------------------------------------------------- data
LIBRARY = [("All uploads", "doc", 18, True), ("Active", "play", 2, False),
           ("Queued", "tri", 1, False), ("Completed", "check", 14, False),
           ("Failed", "x", 1, False)]
DESTS = [("Google Drive", "cloud", True), ("YouTube", "video", True),
         ("WebDAV", "server", False), ("Amazon S3", "db", False)]

ROWS = [  # (name, icon, size, dest, pct, speed, state)
    ("quarterly-report.pdf", "doc", "1.8 GB", "Google Drive", 62, "11.3 MB/s", "up"),
    ("backup-2026-11-07.img", "doc", "24.1 GB", "Amazon S3", 8, "41.2 MB/s", "up"),
    ("keynote-master-v3.mov", "media", "4.4 GB", "YouTube", None, "—", "queue"),
    ("assets-bundle.zip", "doc", "860 MB", "WebDAV", None, "—", "fail"),
    ("dataset-customers.csv.gz", "doc", "312 MB", "Google Drive", None, "—", "ok"),
    ("thesis-final-v2.docx", "doc", "2.1 MB", "Google Drive", None, "—", "ok"),
    ("ui-kit-library.fig", "doc", "148 MB", "Amazon S3", None, "—", "ok"),
    ("invoice-1024.pdf", "doc", "640 KB", "WebDAV", None, "—", "ok"),
    ("logo-set-monochrome.svg", "doc", "88 KB", "Google Drive", None, "—", "ok"),
    ("standup-recording-1108.mp3", "media", "96 MB", "YouTube", None, "—", "ok"),
    ("photos-archive.tar.zst", "doc", "6.7 GB", "Amazon S3", None, "—", "ok"),
    ("nginx-access-11-07.log", "doc", "44 MB", "WebDAV", None, "—", "ok"),
    ("brand-guidelines-v4.pdf", "doc", "18 MB", "Google Drive", None, "—", "ok"),
    ("promo-cut-15s.mp4", "media", "820 MB", "YouTube", None, "—", "ok"),
    ("monorepo-bundle.git", "doc", "1.2 GB", "WebDAV", None, "—", "ok"),
    ("cert-chain-2026.pem", "doc", "12 KB", "Amazon S3", None, "—", "ok"),
    ("wallpaper-pack-5k.zip", "doc", "340 MB", "Google Drive", None, "—", "ok"),
    ("crash-report-1107.dmp", "doc", "2.4 MB", "Amazon S3", None, "—", "ok"),
]

STATE = {
    "up":    ('<span class="dot7"></span><span class="st">Uploading</span>'),
    "queue": ('<span class="dot7"></span><span class="st">Queued</span>'),
    "ok":    (f'{svg("check",13)}<span class="st">Verified</span>'),
    "fail":  (f'{svg("x",13)}<span class="st">Failed</span>'),
}

def row_html(i, r):
    name, icon, size, dest, pct, speed, st = r
    cls = "row sel" if i == 0 else "row"
    if pct is not None:
        prog = (f'<div class="prog"><div class="track"><div class="fill" style="width:{pct}%">'
                f'</div></div><span class="pct">{pct}%</span></div>')
    else:
        prog = '<span class="num" style="text-align:left;color:var(--text3)">—</span>'
    return f'''<div class="{cls}">
<div class="name">{svg(icon,15)}<span class="nm">{name}</span></div>
<span class="num">{size}</span><span class="dest">{dest}</span>
{prog}<span class="num">{speed}</span>
<div class="state {st}">{STATE[st]}</div></div>'''

def sidebar_html():
    out = ['<div class="sidebar"><div class="slabel">Library</div>']
    for label, icon, n, active in LIBRARY:
        a = " active" if active else ""
        fail = ' style="color:var(--red)"' if label == "Failed" else ""
        out.append(f'<div class="sitem{a}">{svg(icon,15)}<span{fail}>{label}</span>'
                   f'<span class="scount">{n}</span></div>')
    out.append('<div class="slabel">Destinations</div>')
    for label, icon, on in DESTS:
        dot = "sdot-on" if on else "sdot-off"
        out.append(f'<div class="sitem">{svg(icon,15)}<span>{label}</span><span class="{dot}"></span></div>')
    out.append(f'<div class="sitem sadd">{svg("plus",15)}<span>Connect destination…</span></div>')
    out.append('</div>')
    return "".join(out)

def toolbar_html(chrome="mac"):
    key = "⌘K" if chrome == "mac" else "Ctrl K"
    tools = f'''<div class="tb">
<button class="btn primary">{svg("upload",14)}New Upload</button>
<div class="sep"></div>
<button class="btn">{svg("play",14)}Start</button>
<button class="btn">{svg("pause",14)}Pause</button>
<button class="btn">{svg("trash",14)}Remove</button>
</div>'''
    right = f'''<div class="spacer"></div>
<div class="search">{svg("search",13)}Search uploads<span class="spacer"></span><span class="key">{key}</span></div>
<div class="sep"></div>
<button class="btn" style="padding:0 8px">{svg("sliders",15)}</button>'''
    if chrome == "mac":
        return f'''<div class="titlebar">
<div class="tl"><i></i><i></i><i></i></div>
{tools}{right}
</div>'''
    if chrome == "win":
        caps = f'''<div class="caps">
<span class="capbtn">{svg("wmin",14)}</span>
<span class="capbtn">{svg("wmax",13)}</span>
<span class="capbtn">{svg("wclose",15)}</span>
</div>'''
        return f'''<div class="titlebar">{tools}{right}{caps}
</div>'''
    # gnome: headerbar with centered title + round close, then a slim tools row
    return f'''<div class="titlebar" style="position:relative">
<span class="gtitle">IUM</span>
<span class="gclose">{svg("x",13)}</span>
</div>
<div class="titlebar" style="height:44px;border-top:none">{tools}{right}
</div>'''

STATUS_HTML = f'''<div class="status">
<div class="stgroup">{svg("tri",11)}<span>2 uploading</span><span class="mono">11.3 MB/s</span></div>
<div class="stsep"></div><div class="stgroup"><span class="mono">1 queued</span></div>
<div class="stsep"></div><div class="stgroup"><span class="red">1 failed</span></div>
<div class="stright"><span class="edot"></span><span class="mono">Engine 127.0.0.1:8727</span>
<div class="stsep"></div><span class="mono">IUM v0.1.1</span></div></div>'''

def inspector_html():
    cells = []
    for i in range(128):
        c = "done" if i < 41 else ("act" if i == 41 else "")
        cells.append(f'<div class="c {c}"></div>')
    return f'''<div class="inspector">
<div class="ihead"><span class="iname">quarterly-report.pdf</span>
<span class="ipath">~/Documents/finance/quarterly-report.pdf</span>
<span class="iclose">{svg("x",13)}</span></div>
<div class="icols">
<div><div class="cap">Transfer</div>
<div class="mline">1.12 GB of 1.8 GB — 62%</div>
<div class="mline">11.3 MB/s · ETA 00:01:07</div>
<div class="mline">resumable · 8 MiB chunks · session kept</div></div>
<div><div class="cap">Chunks — 41/128</div><div class="chunks">{"".join(cells)}</div></div>
<div><div class="cap">Journal — latest</div>
<div class="mline"><span class="jseq">#412</span> verify · sha256 ok</div>
<div class="mline"><span class="jseq">#411</span> chunk.put · 41/128</div>
<div class="mline"><span class="jseq">#410</span> chunk.assign · 41 → w3</div>
<div class="mline"><span class="jseq">#409</span> chunk.evict · w1 slow</div></div>
</div></div>'''

def main_window(theme, inspector, chrome="mac"):
    rows = "".join(row_html(i, r) for i, r in enumerate(ROWS))
    insp = inspector_html() if inspector else ""
    return f'''<div class="win w-{chrome}" data-theme="{theme}">
{toolbar_html(chrome)}
<div class="main">
{sidebar_html()}
<div class="content">
<div class="chead">
<div class="h">Name {svg("sort",9)}</div><div class="h r">Size</div><div class="h">Destination</div>
<div class="h">Progress</div><div class="h r">Speed</div><div class="h">State</div></div>
<div class="rows">{rows}</div>
{insp}
{STATUS_HTML}
</div></div></div>'''

# ------------------------------------------------------------- settings
CONN = [
    ("cloud", "Google Drive", "drive.usercontent.google.com", "Connected", "ghost", "Disconnect", False),
    ("video", "YouTube", "www.googleapis.com · resumable", "Connected", "ghost", "Disconnect", False),
    ("server", "WebDAV", "your own server · any port", "Not connected", "primary", "Connect", True),
    ("db", "Amazon S3", "s3.amazonaws.com · any S3-compatible", "Credentials saved", "ghost", "Edit", False),
]

def modal_html():
    rows = []
    for icon, name, host, state, kind, label, dim in CONN:
        st_cls = "mstate dim" if dim else "mstate"
        rows.append(f'''<div class="mrow"><span class="micon">{svg(icon,17)}</span>
<div class="mnames"><div class="mname">{name}</div><div class="mhost">{host}</div></div>
<div class="mright"><span class="{st_cls}">{state}</span>
<button class="btn sm {kind}">{label}</button></div></div>''')
    return f'''<div class="scrim"></div>
<div class="modal" data-theme="light">
<div class="mhead"><div><div class="mtitle">Settings</div>
<div class="msub">Destinations connect with your own account. Credentials never leave this machine's keychain.</div></div>
<span class="iclosebtn">{svg("x",15)}</span></div>
<div class="mlist">{"".join(rows)}
<div class="mrow"><span class="micon">{svg("sliders",17)}</span>
<div class="mnames"><div class="mname">Engine endpoint</div>
<div class="mhost">ium-core local daemon</div></div>
<div class="mright"><span class="edot"></span><span class="mstate mono" style="font-family:'JetBrains Mono';font-size:11px">127.0.0.1:8727 · Running</span></div></div>
<div class="mrow"><span class="micon">{svg("play",17)}</span>
<div class="mnames"><div class="mname">Launch at sign-in</div></div>
<div class="mright"><span class="sw"><i></i></span></div></div>
<div class="mrow"><span class="micon">{svg("upload",17)}</span>
<div class="mnames"><div class="mname">Automatic updates</div></div>
<div class="mright"><span class="mstate mono" style="font-family:'JetBrains Mono';font-size:11px">v0.1.1</span>
<button class="btn sm ghost">Check now</button></div></div>
</div>
<div class="mfoot"><span class="mhint"><span class="key">Esc</span> to close</span>
<span class="spacer"></span><button class="btn primary sm">Done</button></div>
</div>'''

# ---------------------------------------------------------- launch page
MATRIX = [
    ("macOS", [("Apple Silicon", "dmg"), ("Intel", "dmg")]),
    ("Windows", [("x64", "msi"), ("ARM64", "msi")]),
    ("Linux", [(".deb", "deb"), (".rpm", "rpm"), ("Flathub", "flatpak")]),
]

def launch_page():
    cols = []
    for os_name, rows in MATRIX:
        rws = "".join(f'''<div class="mxrow"><span>{label}</span>
<button class="btn sm ghost">{svg("download",13)}Download</button></div>''' for label, ext in rows)
        cols.append(f'<div><div class="mxh">{os_name}</div>{rws}</div>')
    return f'''<div class="lp">
<div class="lpnav">
<span class="lpbrand">{dart(20, "var(--text)")}IUM</span>
<span class="lplinks"><span>Features</span><span>Docs</span><span>Releases</span><span>GitHub</span></span>
<span class="spacer"></span>
<button class="btn ghost">★ Star</button>
<button class="btn primary">Download</button>
</div>
<div class="lphero">
<span class="chip"><i></i>Free · open source · MIT</span>
<h1 class="lph1">Download IUM.</h1>
<div class="lpsub">Reliability-engineered uploads for macOS, Windows and Linux —<br>journaled, resumable, verified before it declares Done.</div>
<div class="ctas">
<button class="btn primary btn-lg">{svg("download",15)}Download for macOS</button>
<button class="btn ghost btn-lg">All platforms</button>
</div>
<div class="lpmeta">v0.1.1 · 14.8 MB · sha256 published with every asset</div>
<div class="lptile">{dart(46, "#EDF1F8")}</div>
</div>
<div class="lpmx">
<div><div class="mxt">IUM for desktop.</div>
<div class="mxs">macOS, Windows and Linux.</div>
<div class="mxn">Every asset ships with its sha256 checksum.<br>Auto-updates are signed (Tauri updater).</div></div>
<div class="mxcols">{"".join(cols)}</div>
</div>
<div class="lptrust">
<span>{svg("check",13)}Every byte journaled — SQLite WAL</span>
<span>{svg("check",13)}Verify-before-declare — sha256</span>
<span>{svg("check",13)}Resume for weeks — chunk sessions</span>
</div>
<div class="lpfoot">{dart(14, "var(--text3)")}<span style="margin-left:8px">IUM — Internet Upload Manager</span>
<span class="r"><span>MIT license</span><span>GitHub</span><span>Releases</span><span>© 2026 the IUM community</span></span>
</div>
</div>'''

# ---------------------------------------------------------------- pages
def page(theme, body, modal=False):
    return f'''<!DOCTYPE html><html><head><meta charset="utf-8">
<style>{CSS.substitute()}</style></head>
<body data-theme="{theme}">{body}{modal_html() if modal else ""}</body></html>'''

os.makedirs(OUT, exist_ok=True)
pages = {
    "light.html":    page("light", main_window("light", False)),
    "dark.html":     page("dark", main_window("dark", True)),
    "settings.html": page("light", main_window("light", False), modal=True),
    "win.html":      page("dark", main_window("dark", False, chrome="win")),
    "linux.html":    page("light", main_window("light", False, chrome="gnome")),
}
for fn, html in pages.items():
    with open(os.path.join(OUT, fn), "w") as f:
        f.write(html)
    print("wrote", fn)
# the launch page replaces the gradient desk with the page canvas itself
launch = page("dark", launch_page())
launch = launch.replace('<body data-theme="dark">', '<body class="launch" data-theme="dark">')
with open(os.path.join(OUT, "launch.html"), "w") as f:
    f.write(launch)
print("wrote launch.html")
