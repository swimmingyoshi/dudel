"""Summarize isolated native-session reports; never treat memory drift as a leak verdict."""
import argparse
import json
import math
from pathlib import Path
import statistics

MIB = 1024 * 1024


def distribution(values):
    ordered = sorted(values)
    if not ordered:
        return None
    return {
        "count": len(ordered),
        "median": statistics.median(ordered),
        "p95_nearest_rank": ordered[math.ceil(len(ordered) * 0.95) - 1],
        "maximum": ordered[-1],
    }


def summarize(path):
    report = json.loads(path.read_text(encoding="utf-8"))
    if report.get("complete") is False:
        raise ValueError("Incomplete native session cannot be summarized as completed evidence")
    rounds = report.get("rounds", 1)
    cycles = report["cycles"]
    frames = report["frames"]
    if "frame_samples_recorded" in report and report["frame_samples_recorded"] != len(frames):
        raise ValueError("Frame report is missing recorded samples")
    checkpoints = report["checkpoints"]
    if report.get("allocation_profile_enabled", False) and any(
            p.get("rust_allocations") is None for p in checkpoints):
        raise ValueError(f"{path}: allocation profile has missing checkpoint counters")
    checkpoint_storage = None
    capacity_key = "diagnostic_checkpoint_capacity_bytes"
    used_key = "diagnostic_checkpoint_used_bytes"
    if any(capacity_key in p or used_key in p for p in checkpoints):
        if len(checkpoints) < 2 or any(capacity_key not in p or used_key not in p for p in checkpoints):
            raise ValueError(f"{path}: incomplete fixed checkpoint storage counters")
        capacity = checkpoints[0][capacity_key]
        record_bytes = checkpoints[1][used_key]
        if (record_bytes <= 0 or capacity < len(checkpoints) * record_bytes
                or any(p[capacity_key] != capacity or p[used_key] != i * record_bytes
                       for i, p in enumerate(checkpoints))):
            raise ValueError(f"{path}: checkpoint storage grew or used records are inconsistent")
        checkpoint_storage = {"fixed_capacity_bytes": capacity, "record_bytes": record_bytes,
                              "records": len(checkpoints), "used_excludes_current_record": True}
    idle_series = {}
    round_stats = []
    active = {"BrushInput", "BrushApplied", "BrushUndone", "BrushRedone", "Start",
              "Preview", "Applied", "Undone", "Redone", "UndoAll", "RedoAll"}
    for number in range(rounds):
        samples = [f for f in frames if f.get("round", 0) == number]
        points = [p for p in checkpoints if p.get("round", 0) == number]
        edits = [p["cycle"] for p in points if p["label"] == "edit-undo-redo-settled"]
        if edits != list(range(1, cycles + 1)):
            raise ValueError(f"{path}: incomplete edit checks in round {number}")
        required = ["initial-idle", "cancel-restored", "all-undone", "all-redone",
                    "history-released-before-save", "save-reopen-verified",
                    "final-idle-history-released"]
        if rounds > 1:
            required.append("brush-undo-redo-settled")
        for label in required:
            if sum(p["label"] == label for p in points) != 1:
                raise ValueError(f"{path}: missing/duplicate {label} in round {number}")
        round_stats.append({
            "round": number,
            "ui_prefix_ms": distribution([f["ui_update_before_diagnostic_ms"] for f in samples]),
            "active_ui_prefix_ms": distribution([f["ui_update_before_diagnostic_ms"] for f in samples if f["phase"] in active]),
            "instrumented_app_command_ms": distribution([f["diagnostic_app_command_ms"] for f in samples if f.get("diagnostic_app_command_ms", 0) > 0]),
            "diagnostic_driver_ms": distribution([f["diagnostic_driver_ms"] for f in samples]),
        })
    for label in ["initial-idle", "history-released-before-save", "final-idle-history-released"]:
        series = []
        for p in checkpoints:
            if p["label"] == label:
                memory = p["process_memory"]
                if memory is None:
                    raise ValueError(f"{path}: process memory unavailable")
                series.append({
                    "round": p.get("round", 0),
                    "elapsed_seconds": p["elapsed_seconds"],
                    "resident_mib": memory["resident_bytes"] / MIB,
                    "private_committed_mib": memory["private_committed_bytes"] / MIB,
                    "tracked_live_mib": p["tracked_live_bytes"] / MIB,
                    "tracked_additional_mib": p["tracked_additional_bytes"] / MIB,
                    "registered_snapshot_total_mib": (p["tracked_live_bytes"] + p["tracked_additional_bytes"]) / MIB,
                    "display_rgba_mib": p["display_rgba_bytes"] / MIB,
                    "display_prefix_mib": p["display_prefix_bytes"] / MIB,
                    "history_mib": p["history_retained_bytes"] / MIB,
                    "diagnostic_frame_capacity_mib": p["diagnostic_frame_capacity_bytes"] / MIB,
                    "diagnostic_frame_used_mib": p.get("diagnostic_frame_used_bytes", 0) / MIB,
                    **({"rust_live_requested_mib": p["rust_allocations"]["live_requested_bytes"] / MIB,
                        "rust_lifetime_peak_requested_mib": p["rust_allocations"]["lifetime_peak_requested_bytes"] / MIB}
                       if p.get("rust_allocations") is not None else {}),
                    **({"gpu_suballocated_mib": p["gpu_allocations"]["allocated_bytes"] / MIB,
                        "gpu_reserved_mib": p["gpu_allocations"]["reserved_bytes"] / MIB,
                        "gpu_allocation_count": p["gpu_allocations"]["allocation_count"],
                        "gpu_block_count": p["gpu_allocations"]["block_count"]}
                       if p.get("gpu_allocations") is not None else {}),
                })
        tail = series[min(3, len(series) - 1):]
        drift = {key: tail[-1][key] - tail[0][key] for key in
                 ["resident_mib", "private_committed_mib", "tracked_live_mib", "tracked_additional_mib", "registered_snapshot_total_mib", "display_rgba_mib", "display_prefix_mib", "history_mib", "diagnostic_frame_used_mib"]}
        if all("rust_live_requested_mib" in p for p in tail):
            drift["rust_live_requested_mib"] = tail[-1]["rust_live_requested_mib"] - tail[0]["rust_live_requested_mib"]
        for key in ["gpu_suballocated_mib", "gpu_reserved_mib", "gpu_allocation_count", "gpu_block_count"]:
            if all(key in p for p in tail):
                drift[key] = tail[-1][key] - tail[0][key]
        idle_series[label] = {"samples": series, "tail_start_round": tail[0]["round"], "tail_drift": drift}
    commands = {}
    for phase in sorted({f["phase"] for f in frames}):
        values = [f["diagnostic_app_command_ms"] for f in frames
                  if f["phase"] == phase and f.get("diagnostic_app_command_ms", 0) > 0]
        if values:
            commands[phase] = distribution(values)
    return {
        "source": str(path), "renderer": report["renderer"],
        "allocation_profile_enabled": report.get("allocation_profile_enabled", False),
        "checkpoint_storage": checkpoint_storage,
        "gpu_allocation_profile_enabled": report.get("gpu_allocation_profile_enabled", False),
        "gpu_allocation_checkpoints": [
            {"label": p["label"], "round": p.get("round", 0), "cycle": p["cycle"],
             "allocated_mib": p["gpu_allocations"]["allocated_bytes"] / MIB,
             "reserved_mib": p["gpu_allocations"]["reserved_bytes"] / MIB,
             "allocation_count": p["gpu_allocations"]["allocation_count"],
             "block_count": p["gpu_allocations"]["block_count"]}
            for p in checkpoints if p.get("gpu_allocations") is not None],
        "allocation_checkpoints": [
            {"label": p["label"], "round": p.get("round", 0), "cycle": p["cycle"],
             "private_committed_mib": p["process_memory"]["private_committed_bytes"] / MIB,
             **{key: value / MIB if key.endswith("bytes") else value
                for key, value in p["rust_allocations"].items()}}
            for p in checkpoints if p.get("rust_allocations") is not None],
        "zoom": report["application_zoom"], "viewport_points": report["viewport_points"],
        "rounds": rounds, "mask_edits_verified": rounds * cycles,
        "brush_strokes_verified": rounds if rounds > 1 else 0,
        "elapsed_seconds": report["elapsed_seconds"], "callbacks": len(frames),
        "frame_storage": report.get("frame_storage", "Legacy in-memory frame array"),
        "frame_log_ms": (distribution([f["diagnostic_previous_frame_log_ms"] for f in frames[1:]]
                                      + [report["diagnostic_final_frame_log_ms"]])
                         if report.get("version", 0) >= 3 else None),
        "ui_prefix_ms": distribution([f["ui_update_before_diagnostic_ms"] for f in frames]),
        "active_ui_prefix_ms": distribution([f["ui_update_before_diagnostic_ms"] for f in frames if f["phase"] in active]),
        "generated_brush_ui_prefix_ms": distribution([f["ui_update_before_diagnostic_ms"] for f in frames if f["phase"] in {"BrushInput", "BrushApplied"}]),
        "ui_prefix_ms_by_phase": {phase: distribution([f["ui_update_before_diagnostic_ms"] for f in frames if f["phase"] == phase]) for phase in sorted({f["phase"] for f in frames})},
        "round_stats": round_stats, "command_ms_by_phase": commands,
        "idle_series": idle_series,
        "scope": "Descriptive measurements, not a leak verdict, GPU time, FPS, physical-input latency or release certification. Nearest-rank p95. Tail drift omits the first three rounds when present. Diagnostic frame use is shown separately; allocator/driver ownership is not inferred.",
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("reports", nargs="+", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    summaries = [summarize(path) for path in args.reports]
    with args.output.open("x", encoding="utf-8") as stream:
        json.dump(summaries, stream, indent=2)
        stream.write("\n")
    for summary in summaries:
        print(f"zoom={summary['zoom']}: rounds={summary['rounds']}, seconds={summary['elapsed_seconds']:.2f}, callbacks={summary['callbacks']}")
        for label, data in summary["idle_series"].items():
            print(label, json.dumps(data["tail_drift"], sort_keys=True))


if __name__ == "__main__":
    main()
