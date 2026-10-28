"""Kill our native producer after normal recovery saves, restart and restore a copy."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

REPOSITORY = Path(__file__).resolve().parents[1]


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest().upper()


def check(executable, output, workload="small", save_sharing_failure=False):
    if save_sharing_failure and os.name != "nt":
        raise ValueError("The save sharing-lock probe requires Windows")
    output = output.resolve()
    if not output.is_relative_to(REPOSITORY / "target" / "qa"):
        raise ValueError("Recovery evidence must be inside this repository's target/qa")
    output.mkdir(parents=True, exist_ok=False)
    frozen = output / "dudel.exe"
    shutil.copy2(executable, frozen)
    state = {"status": "starting", "executable_sha256": sha(frozen), "workload": workload,
             "save_sharing_failure": save_sharing_failure, "completed": [],
             "started_utc": datetime.now(timezone.utc).isoformat()}

    def record(**details):
        state.update(details)
        temporary = output / "STATE.tmp"
        temporary.write_text(json.dumps(state, indent=2) + "\n", encoding="utf-8")
        temporary.replace(output / "STATE.json")

    environment = dict(os.environ, DUDEL_RENDERER="dx12")
    flags = subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0
    try:
        for version in (4, 5):
            fixture = output / f"v{version}"
            record(status="running", phase=f"v{version}-producer", pid=None)
            with (output / f"v{version}-producer.log").open("wb") as log:
                suffix = "" if workload == "small" else "-" + workload
                child = subprocess.Popen([str(frozen), "--probe-artwork-recovery", str(fixture), f"write-v{version}{suffix}"],
                    cwd=output, env=environment, stdout=log, stderr=log, creationflags=flags)
                try:
                    record(pid=child.pid)
                    deadline = time.monotonic() + (90 if workload == "small" else 270)
                    ready = fixture / "write-ready.json"
                    while not ready.is_file():
                        if child.poll() is not None:
                            raise RuntimeError(f"Producer exited before recovery evidence: {fixture}")
                        if time.monotonic() > deadline:
                            raise TimeoutError("Normal recovery producer timed out")
                        time.sleep(0.1)
                    # Publication of this marker follows a closed complete JSON file;
                    # tolerate the short create/write observation window.
                    while True:
                        try:
                            producer = json.loads(ready.read_text(encoding="utf-8"))
                            break
                        except json.JSONDecodeError:
                            if child.poll() is not None:
                                raise RuntimeError("Producer exited while publishing readiness evidence")
                            if time.monotonic() > deadline:
                                raise
                            time.sleep(0.05)
                    if (producer["pid"] != child.pid or producer["generations_verified"] != 2
                            or producer["uncheckpointed_generation"] != 3
                            or producer["expected_restore_generation"] != 2):
                        raise RuntimeError("Producer identity or generation evidence mismatch")
                    if child.poll() is not None:
                        raise RuntimeError("Producer was not live at the interruption point")
                    if workload != "small" and producer.get("canvas") != [2048, 2048]:
                        raise RuntimeError("Large fixture geometry was not exercised")
                    if workload == "large-adjustment" and (not producer.get("busy_preview_seen")
                            or producer.get("eligible_frames_deferred", 0) < 1
                            or producer.get("busy_released_seconds", 0) < 20):
                        raise RuntimeError("Busy adjustment deferral evidence is missing")
                    snapshot = Path(producer["snapshot"]).resolve()
                    # Rust canonical paths use Windows' extended prefix; samefile
                    # compares directory identity without textual-prefix ambiguity.
                    if not snapshot.parent.samefile(fixture / "recovery"):
                        raise RuntimeError("Snapshot escaped its disposable fixture")
                    original = fixture / "original.dudel"
                    header = b"DUDL" + bytes([version]) + b"\r\n"
                    if not original.read_bytes().startswith(header):
                        raise RuntimeError("Fixture is not the requested native container version")
                    before = {original: sha(original), snapshot: sha(snapshot)}
                    child.kill()
                    child.wait(timeout=15)
                    if any(sha(path) != digest for path, digest in before.items()):
                        raise RuntimeError("Process termination changed committed artwork files")
                finally:
                    if child.poll() is None:
                        child.kill()
                        child.wait(timeout=15)
            record(status="running", phase=f"v{version}-restore", pid=None,
                   terminated_producer_pid=child.pid)
            with (output / f"v{version}-restore.log").open("wb") as log:
                restore_phase = "restore-save-sharing-failure" if save_sharing_failure else "restore"
                subprocess.run([str(frozen), "--probe-artwork-recovery", str(fixture), restore_phase],
                    cwd=output, env=environment, stdout=log, stderr=log,
                    creationflags=flags, timeout=90 if workload == "small" else 270, check=True)
            restored = json.loads((fixture / "restore.json").read_text(encoding="utf-8"))
            if not restored["passed"]:
                raise RuntimeError("Native recovery restore did not pass")
            if save_sharing_failure and not restored.get("save_failure_verified"):
                raise RuntimeError("Native save failure/retry evidence is missing")
            if (not (fixture / "restored-copy.dudel").read_bytes().startswith(header)
                    or any(sha(path) != digest for path, digest in before.items())):
                raise RuntimeError("Restore changed the container version or committed source files")
            state["completed"].append({"version": version, "producer": producer, "restore": restored,
                                       "original_sha256": before[original], "snapshot_sha256": before[snapshot]})
            record()
            print(f"Native v{version} interruption/restart/restore/save passed", flush=True)
        record(status="passed", phase="complete", pid=None,
               completed_utc=datetime.now(timezone.utc).isoformat())
    except BaseException as error:
        record(status="failed", pid=None, error=str(error))
        raise


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--workload", choices=("small", "large", "large-adjustment"), default="small")
    parser.add_argument("--save-sharing-failure", action="store_true", help="Windows: lock an existing save destination, verify failure preserves it, then unlock and retry")
    args = parser.parse_args()
    check(args.executable, args.output, args.workload, args.save_sharing_failure)
