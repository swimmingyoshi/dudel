"""Evaluate the completion plan's per-run memory targets, never full acceptance."""
import argparse
import json
import math
from pathlib import Path
import statistics

from analyze_native_session import summarize

STAGES = ("initial-idle", "history-released-before-save", "final-idle-history-released")


def assess(report, summary, warmup=5):
    if report.get("complete") is False:
        raise ValueError("Incomplete native session is not eligible for acceptance")
    rounds = summary["rounds"]
    if warmup < 0 or rounds - warmup < 2:
        raise ValueError("Need at least two post-warmup rounds")
    if report.get("fixture_options", {}).get("rounds", rounds) != rounds:
        raise ValueError("Fixture/report round counts disagree")
    if report.get("fixture_options", {}).get("layers", report["layers"]) != report["layers"]:
        raise ValueError("Fixture/report layer counts disagree")
    stages = {}
    for label in STAGES:
        series = summary["idle_series"][label]["samples"]
        if [p["round"] for p in series] != list(range(rounds)):
            raise ValueError(f"{label}: incomplete or out-of-order matching idle samples")
        tail = series[warmup:]
        xs = [p["round"] for p in tail]
        ys = [p["private_committed_mib"] for p in tail]
        if any(not math.isfinite(y) or y < 0 for y in ys):
            raise ValueError("Invalid process commitment measurement")
        mx, my = statistics.mean(xs), statistics.mean(ys)
        slope = sum((x - mx) * (y - my) for x, y in zip(xs, ys)) / sum((x - mx) ** 2 for x in xs)
        ownership = {}
        for key in ("registered_snapshot_total_mib", "display_rgba_mib", "display_prefix_mib", "history_mib"):
            values = [p[key] for p in tail]
            if any(not math.isfinite(v) or v < 0 for v in values):
                raise ValueError(f"Invalid ownership measurement: {key}")
            ownership[key] = {"net_mib": values[-1] - values[0], "maximum_mib": max(values)}
        stages[label] = {
            "tail_start_round_zero_based": tail[0]["round"], "samples": len(tail),
            "private_net_mib": ys[-1] - ys[0], "private_slope_mib_per_round": slope,
            "private_min_mib": min(ys), "private_max_mib": max(ys),
            "private_target_pass": ys[-1] - ys[0] <= 32 and slope <= 1,
            "ownership": ownership,
            "tracked_ownership_target_pass": all(v["net_mib"] <= 0.01 for v in ownership.values())
                and ownership["history_mib"]["maximum_mib"] == 0,
        }
    configuration_pass = (
        rounds >= 30 and 20 <= report["layers"] <= 32
        and report.get("profile") == "release" and "Dx12" in summary["renderer"]
        and not any(report.get(flag, False) for flag in (
            "allocation_profile_enabled", "allocation_stack_profile_enabled", "gpu_allocation_profile_enabled"))
    )
    return {
        "rounds": rounds, "warmup_rounds": warmup, "layers": report["layers"],
        "fixture_options": report.get("fixture_options"), "renderer": summary["renderer"],
        "elapsed_seconds": summary["elapsed_seconds"],
        "configuration_target_pass": configuration_pass,
        "per_run_memory_targets_pass": configuration_pass and all(
            s["private_target_pass"] and s["tracked_ownership_target_pass"] for s in stages.values()),
        "stages": stages,
        "release_gate_accepted": False,
        "scope": "Per-run CPU/process memory subchecks only. Requires three runs, two-hour mixed session, dense fixtures, GPU ownership, uninstrumented timing, source identity and physical artist acceptance before G2 can close. Uses five warmup rounds, unlike historical three-round descriptive summaries.",
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    report = json.loads(args.report.read_text(encoding="utf-8"))
    result = assess(report, summarize(args.report))
    result["source"] = str(args.report)
    with args.output.open("x", encoding="utf-8") as stream:
        json.dump(result, stream, indent=2)
        stream.write("\n")
    print(json.dumps({"per_run_memory_targets_pass": result["per_run_memory_targets_pass"],
                      "release_gate_accepted": False, "stages": result["stages"]}, indent=2))


if __name__ == "__main__":
    main()
