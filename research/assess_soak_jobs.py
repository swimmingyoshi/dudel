"""Evaluate reported lifetime peaks and full editor-ready save/open durations."""
import math
from analyze_native_session import distribution


def assess(report):
    if report.get("complete") is False:
        raise ValueError("Incomplete native session is not eligible for acceptance")
    jobs = report.get("completed_jobs")
    checkpoints = report.get("checkpoints", [])
    if jobs is None or not checkpoints or any(
            "peak_private_committed_bytes" not in (p.get("process_memory") or {}) for p in checkpoints):
        return {"available": False, "per_run_job_peak_targets_pass": False,
                "reason": "Missing completed-job or Windows lifetime-peak counters; historical checkpoints cannot reconstruct peaks",
                "release_gate_accepted": False}
    rounds = report["rounds"]
    if len(jobs) != rounds * 2:
        raise ValueError("Incomplete completed save/open samples")
    by_operation = {"Save": [], "Open": []}
    seen = set()
    for job in jobs:
        pair = (job["round"], job["operation"])
        value = job["command_to_editor_ready_ms"]
        if (pair in seen or pair[0] not in range(rounds) or pair[1] not in by_operation
                or not math.isfinite(value) or value <= 0):
            raise ValueError("Invalid or duplicate completed-job sample")
        seen.add(pair)
        by_operation[pair[1]].append(value)
    peaks = []
    for point in checkpoints:
        memory = point["process_memory"]
        peak = memory["peak_private_committed_bytes"]
        if (not isinstance(peak, int) or peak <= 0 or peak < memory["private_committed_bytes"]
                or (peaks and peak < peaks[-1])):
            raise ValueError("Invalid/nonmonotonic Windows lifetime-peak counter")
        peaks.append(peak)
    options = report.get("fixture_options", {})
    standard = report.get("layers") == 20 and options.get("density") == "Sparse"
    maximum = report.get("layers") == 32 and options.get("density") in ("Pattern", "Noise")
    peak_budget = (2 if standard else 4) * 1024**3
    job_budget = 5000 if standard else 30000
    payload = report.get("initial_raster_and_layer_mask_payload_bytes")
    configuration = bool((standard or maximum) and report.get("canvas") == [2048, 2048]
                         and report.get("profile") == "release" and rounds >= 5
                         and not any(report.get(key, False) for key in (
                             "allocation_profile_enabled", "allocation_stack_profile_enabled", "gpu_allocation_profile_enabled"))
                         and isinstance(payload, int) and 0 < payload <= 512 * 1024**2)
    durations = {key: distribution(values) for key, values in by_operation.items()}
    timing_pass = all(value["count"] >= 5 and value["p95_nearest_rank"] <= job_budget
                      for value in durations.values())
    peak_pass = max(peaks) <= peak_budget
    return {"available": True, "configuration_target_pass": configuration,
            "fixture_class": "standard" if standard else "maximum" if maximum else "diagnostic",
            "private_commit_lifetime_peak_bytes": max(peaks), "peak_budget_bytes": peak_budget,
            "peak_target_pass": peak_pass, "completed_jobs_ms": durations,
            "completed_job_p95_budget_ms": job_budget, "completed_job_target_pass": timing_pass,
            "per_run_job_peak_targets_pass": configuration and timing_pass and peak_pass,
            "release_gate_accepted": False,
            "scope": "Warm-cache command-to-editor-ready wall times include worker completion/polling and Open display settlement, before pixel oracles. Windows lifetime peak through the last checkpoint includes fixed diagnostic storage. Not isolated worker CPU, cold-cache performance, VRAM, frame timing, physical input or release certification."}
