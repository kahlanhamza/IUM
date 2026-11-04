"""
IUM self-hosted GitHub Actions runner kernel (runs on Kaggle compute;
pushed by scripts/kaggle_push_runner.sh).

The kernel:
  1. downloads the pinned actions-runner tarball from GitHub releases
     (public, no auth) — binaries are NOT shipped in the dataset;
  2. overlays the pre-registered runner config (.runner/.credentials*)
     from the private `hamzakhl/ium-runner` dataset;
  3. runs `./run.sh` under a hard timeout — it long-polls GitHub, picks
     up every queued job matching label `ium-kaggle`, and the session
     self-ends politely when the timeout hits (no 12 h idle sessions).

Secrets: only the runner-scoped credential travels (private dataset).
The GitHub PAT never enters the kernel; revoke anytime via
Settings -> Actions -> Runners -> Remove.
"""
import os
import shutil
import subprocess
import tarfile
import urllib.request
from pathlib import Path

RUNNER_VER = "RUNNER_VER_PLACEHOLDER"
TIMEOUT_SECS = "TIMEOUT_SECS_PLACEHOLDER"
TARBALL_URL = (
    f"https://github.com/actions/runner/releases/download/"
    f"v{RUNNER_VER}/actions-runner-linux-x64-{RUNNER_VER}.tar.gz"
)
WORK = Path("/kaggle/working")
RUN_DIR = Path("/tmp/actions-runner")


def log(msg: str) -> None:
    print(f"[ium-runner] {msg}", flush=True)


def diagnostics() -> None:
    for probe in (
        "id",
        "git --version || true",
        "ffmpeg -version 2>/dev/null | head -1 || echo 'no ffmpeg'",
        "sudo -n true 2>/dev/null && echo 'sudo: available' || echo 'sudo: NOT available'",
        "python3 -c 'import urllib.request; print(urllib.request.getproxies())'",
        "python3 -c \"from pathlib import Path; print({str(p): sorted(x.name for x in p.iterdir()) for p in Path('/kaggle/input').iterdir() if p.is_dir()})\"",
    ):
        p = subprocess.run(probe, shell=True, capture_output=True, text=True)
        log(f"$ {probe}\n{(p.stdout + p.stderr).strip()}")


def download_runner() -> None:
    RUN_DIR.parent.mkdir(parents=True, exist_ok=True)
    if RUN_DIR.exists():
        shutil.rmtree(RUN_DIR)
    RUN_DIR.mkdir(parents=True)
    gz = Path("/tmp/runner.tgz")
    log(f"downloading {TARBALL_URL}")
    urllib.request.urlretrieve(TARBALL_URL, gz)
    with tarfile.open(gz) as t:
        t.extractall(RUN_DIR, filter="data")
    log(f"extracted to {RUN_DIR}")


def overlay_config() -> None:
    src = Path("/kaggle/input/ium-runner")
    tars = list(src.glob("runner-config.tar.gz"))
    if tars:
        with tarfile.open(tars[0]) as t:
            t.extractall(RUN_DIR, filter="data")
    else:
        # Kaggle auto-extracts archives on dataset upload — the dotfiles
        # land directly in the input dir.
        copied = []
        for name in (".runner", ".credentials", ".credentials_rsaparams", ".env", ".path"):
            p = src / name
            if p.exists():
                shutil.copy2(p, RUN_DIR / name)
                copied.append(name)
        log(f"auto-extracted shape: copied {copied}")
    RUN_DIR.chmod(0o755)
    for f in ("config.sh", "run.sh"):
        (RUN_DIR / f).chmod(0o755)
    have = sorted(p.name for p in RUN_DIR.iterdir() if p.name.startswith("."))
    log(f"config overlay done, dotfiles present: {have}")
    assert (RUN_DIR / ".runner").exists(), ".runner missing after overlay"
    assert (RUN_DIR / ".credentials").exists(), ".credentials missing after overlay"


def setup_runner_user() -> None:
    # The kernel runs as root; the runner refuses root/sudo sessions
    # ("Must not run interactively with sudo"). Run the listener as a
    # dedicated non-root user instead; jobs keep passwordless sudo for
    # the apt-based system-deps steps.
    subprocess.run(
        "id runner >/dev/null 2>&1 || useradd -m -s /bin/bash runner",
        shell=True, check=True,
    )
    subprocess.run(
        "echo 'runner ALL=(ALL) NOPASSWD:ALL' > /etc/sudoers.d/runner "
        "&& chmod 440 /etc/sudoers.d/runner",
        shell=True, check=True,
    )
    for d in (str(RUN_DIR), "/tmp/ium-work"):
        subprocess.run(f"mkdir -p {d} && chown -R runner:runner {d}", shell=True, check=True)
    log("runner user ready (runner, passwordless sudo)")


def run_runner() -> int:
    # su to the runner user: clears SUDO_* and the root identity the
    # Listener refuses; timeout still bounds the session.
    cmd = (
        f"su -s /bin/bash runner -c 'cd {RUN_DIR} && timeout {TIMEOUT_SECS} ./run.sh'"
    )
    log(f"starting runner (timeout {TIMEOUT_SECS}s): {cmd}")
    p = subprocess.run(cmd, shell=True)
    log(f"runner exited rc={p.returncode} (124 = timeout, expected for idle)")
    return p.returncode


def main() -> None:
    WORK.mkdir(parents=True, exist_ok=True)
    diagnostics()
    download_runner()
    overlay_config()
    setup_runner_user()
    rc = run_runner()
    (WORK / "RESULT.txt").write_text(f"runner ver={RUNNER_VER} rc={rc}\n")
    log(f"FINAL rc={rc}")


main()
