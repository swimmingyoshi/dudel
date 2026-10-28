"""Verify native diagnostic error preservation using an isolated output conflict."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

REPOSITORY = Path(__file__).resolve().parents[1]


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest().upper()


def check(executable, build_path, output):
    output = output.resolve()
    if not output.is_relative_to(REPOSITORY / "target" / "qa"):
        raise ValueError("Evidence must use a new isolated target/qa folder")
    build = json.loads(build_path.read_text(encoding="utf-8-sig"))
    if sha(executable) != build["executable_sha256"]:
        raise ValueError("Executable differs from build identity")
    output.mkdir(parents=True, exist_ok=False)
    frozen = output / "dudel.exe"
    shutil.copy2(executable, frozen)
    (output / "BUILD.json").write_text(json.dumps(build, indent=2) + "\n")
    state = {"status": "starting", "executable_sha256": sha(frozen),
             "started_utc": datetime.now(timezone.utc).isoformat(), "release_gate_accepted": False}

    def record(**fields):
        state.update(fields)
        pending = output / "STATE.pending.json"
        pending.write_text(json.dumps(state, indent=2) + "\n")
        pending.replace(output / "STATE.json")

    report = output / "session.json"
    stderr = output / "native.stderr.log"
    marker = b'{"fixture":"deliberate diagnostic publication conflict"}\n'
    process = None
    try:
        environment = dict(os.environ, DUDEL_RENDERER="dx12")
        with (output / "native.stdout.log").open("wb") as out, stderr.open("wb") as err:
            process = subprocess.Popen(
                [str(frozen), "--soak-native-mask-session", str(report), "1280", "800", "1"],
                cwd=REPOSITORY, env=environment, stdout=out, stderr=err,
                creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
            record(status="running", pid=process.pid)
            deadline = time.monotonic() + 90
            while "Native session: initial-idle" not in stderr.read_text(errors="replace"):
                if process.poll() is not None:
                    raise RuntimeError("Native process exited before the conflict could be injected")
                if time.monotonic() > deadline:
                    raise TimeoutError("Native startup marker did not arrive")
                time.sleep(0.05)
            # Startup has already checked that the report path is absent. Create
            # a competing destination now; exercise real no-clobber publication.
            with report.open("xb") as conflict:
                conflict.write(marker)
            record(status="conflict-injected")
            code = process.wait(timeout=180)
        if code != 0:
            raise RuntimeError(f"Unexpected native exit code {code}")
        partial = report.with_suffix(".incomplete.json")
        evidence = json.loads(partial.read_text(encoding="utf-8"))
        if report.read_bytes() != marker or report.with_suffix(".png").exists():
            raise ValueError("Conflict destination changed or success capture was published")
        if (evidence.get("complete") is not False or evidence.get("release_gate_accepted") is not False
                or evidence.get("planned_rounds") != 1 or not evidence.get("error")
                or evidence.get("frame_samples_recorded", 0) <= 0 or "frames" in evidence):
            raise ValueError("Incomplete evidence scope/identity is incorrect")
        jobs = evidence["completed_jobs"]
        if (len(jobs) != 2 or {job["operation"] for job in jobs} != {"Save", "Open"}
                or any(job["round"] != 0 or job["command_to_editor_ready_ms"] <= 0 for job in jobs)
                or not evidence["checkpoints"]):
            raise ValueError("Completed job/checkpoint evidence was lost")
        if "Native session failed:" not in stderr.read_text(errors="replace"):
            raise ValueError("Native capture wrapper did not report the error")
        if "UI capture timed out" in stderr.read_text(errors="replace"):
            raise ValueError("Failed diagnostic resumed the ordinary screenshot stage")
        sys.path.insert(0, str(REPOSITORY / "research"))
        from analyze_native_session import summarize
        try:
            summarize(partial)
        except ValueError as error:
            if "Incomplete" not in str(error):
                raise
        else:
            raise ValueError("Incomplete report was accepted by the summary reader")
        record(status="passed", pid=None, exit_code=code, error=evidence["error"],
               preserved_jobs=len(jobs), preserved_checkpoints=len(evidence["checkpoints"]),
               incomplete_sha256=sha(partial), conflict_preserved=True,
               incomplete_rejected_by_summary=True, no_spurious_screenshot_timeout=True)
        print("Native failure evidence preserved; conflict unchanged; incomplete summary rejected.", flush=True)
    except BaseException as error:
        if process is not None and process.poll() is None:
            process.kill()
            process.wait()
        record(status="failed", pid=None, error=str(error))
        raise


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", type=Path, required=True)
    parser.add_argument("--build", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    check(args.executable, args.build, args.output)
