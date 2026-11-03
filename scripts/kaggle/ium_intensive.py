"""
IUM intensive-program kernel (runs on Kaggle compute; pushed by scripts/kaggle_push.py).

The kernel:
  1. unpacks the repo source from the private `hamzakhl/ium-source` dataset;
  2. installs the pinned-ish Rust toolchain (stable, cached in the session);
  3. runs the SCENARIO baked at push time and streams progress;
  4. writes every artifact to /kaggle/working (the kernel's output).

SCENARIOS
  smoke     — offline gate: fmt + clippy -D warnings + cargo test -p ium-core
  stress    — the real-network stress campaign: S1-S5 + all three S7 net legs
              (SeaweedFS 4.51 + the throttle proxy; ~real film, byte-exact asserts)
  longhaul  — the long-horizon leg: s3-test.sh --with-longhaul FILE RATE SECS
              (honest timebox-pause + fresh-journal replay; SECS bounded by the
              kernel session, not the 17 h overnight config)

Secrets: NONE travel in this file. The source is a private dataset; no GitHub
PAT is needed inside the kernel at all.
"""
import os
import subprocess
import shutil
import tarfile
from pathlib import Path

SCENARIO = "SCENARIO_PLACEHOLDER"
WORK = Path("/kaggle/working")
SRC = Path("/kaggle/ium-src")


def log(msg: str) -> None:
    print(f"[ium-kernel] {msg}", flush=True)


def unpack_source() -> Path:
    for cand in [Path("/kaggle/input/ium-source")]:
        assert cand.exists(), f"dataset input missing: {cand}"
        tars = list(cand.glob("ium-source.tar.gz"))
        assert tars, "no ium-source.tar.gz in the dataset"
        if SRC.exists():
            shutil.rmtree(SRC)
        SRC.mkdir(parents=True)
        with tarfile.open(tars[0]) as t:
            t.extractall(SRC)
        log(f"source unpacked to {SRC}")
        return SRC
    raise RuntimeError("unreachable")


def ensure_rust() -> None:
    if shutil.which("cargo"):
        log("cargo already present")
        return
    env = dict(os.environ)
    subprocess.run(
        "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y "
        "--profile minimal --default-toolchain stable",
        shell=True, check=True, env=env,
    )
    os.environ["PATH"] = f"{Path.home()/' .cargo'/ 'bin'}:{os.environ['PATH']}".replace(" .cargo", ".cargo")
    log(f"toolchain: {subprocess.run(['cargo','--version'],capture_output=True,text=True).stdout.strip()}")


def run(cmd: str, name: str) -> int:
    log(f"=== {name} ===")
    p = subprocess.run(cmd, shell=True, cwd=SRC, env=dict(os.environ))
    log(f"=== {name} -> rc={p.returncode} ===")
    return p.returncode


def scenario_smoke() -> int:
    rc = run("cargo fmt --all -- --check", "fmt")
    rc |= run("cargo clippy -p ium-core --all-targets -- -D warnings", "clippy")
    rc |= run("cargo test -p ium-core", "offline suite")
    return rc


def scenario_stress() -> int:
    # the stress driver builds servers, the proxy and the engine, then runs
    # S1-S5 + the three S7 net models with byte-exact asserts
    return run("bash scripts/realworld-stress.sh", "real-world stress campaign")


def scenario_longhaul() -> int:
    film = WORK / "longhaul-film.mp4"
    subprocess.run(
        f"ffmpeg -y -f lavfi -i testsrc2=size=1280x720:rate=30:duration=60 "
        f"-f lavfi -i sine=frequency=440:duration=60 -c:v libx264 -preset fast "
        f"-pix_fmt yuv420p -c:a aac -shortest {film}",
        shell=True, check=True,
    )
    log(f"film generated: {film.stat().st_size/1e6:.1f} MB")
    # 30 MiB-class leg at 512 B/s-class honesty is a 17 h overnight config —
    # inside a kernel session the honest shape is the timebox: upload at a
    # uniform paced rate, timebox-pause at a lease boundary, fresh-journal
    # replay to Done. 900 s at 96 KiB/s ≈ 86 MiB-class leg.
    return run(
        f"bash scripts/s3-test.sh --with-longhaul {film} 98304 900",
        "longhaul leg (uniform 96 KiB/s, 900 s timebox)",
    )


def main() -> None:
    WORK.mkdir(parents=True, exist_ok=True)
    unpack_source()
    ensure_rust()
    if SCENARIO == "smoke":
        rc = scenario_smoke()
    elif SCENARIO == "stress":
        rc = scenario_stress()
    elif SCENARIO == "longhaul":
        rc = scenario_longhaul()
    else:
        raise RuntimeError(f"unknown scenario {SCENARIO}")
    (WORK / "RESULT.txt").write_text(f"scenario={SCENARIO} rc={rc}\n")
    log(f"FINAL rc={rc} — artifacts in /kaggle/working")


main()
