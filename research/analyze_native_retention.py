"""Validate native-session coverage and summarize matching-idle retention.

184-byte heap blocks are a proxy for the previously identified thread records,
not independent proof of each block's owner. Rust, heap and GPU views overlap.
"""
import argparse
import json
from pathlib import Path
import statistics

from analyze_native_session import summarize

MIB = 1024 * 1024
SIZE_KEY = (1 << 63) + 184


def analyze(path, warmup=3):
    validated = summarize(path)  # Reject incomplete correctness/checkpoint series.
    report = json.loads(path.read_text(encoding="utf-8"))
    heap_path = path.with_suffix(".heapwalk.json")
    walks = json.loads(heap_path.read_text(encoding="utf-8"))["walks"]
    if len(walks) != report["rounds"] or [w["round"] for w in walks] != list(range(report["rounds"])):
        raise ValueError("Heap walks must cover every round exactly once in order")
    points = [p for p in report["checkpoints"] if p["label"] == "final-idle-history-released"]
    series = []
    for point, walk in zip(points, walks):
        if not walk["walked"] or walk["overflow_blocks"]:
            raise ValueError("Incomplete or overflowing heap walk")
        if point["history_retained_bytes"] != 0:
            raise ValueError("Final idle retains history")
        regions = point["memory_regions"]
        if regions["heaps_omitted"]:
            raise ValueError("Heap summary omitted heaps")
        # GetProcessHeaps does not promise the default heap is first. The walk
        # measures that heap directly, so use its busy-byte total for allocation.
        slots = [s for s in walk["slots"] if s["key"] == SIZE_KEY]
        if len(slots) > 1:
            raise ValueError("Duplicate 184-byte size bucket")
        series.append({
            "round": point["round"],
            "private_mib": point["process_memory"]["private_committed_bytes"] / MIB,
            "rust_live_mib": point["rust_allocations"]["live_requested_bytes"] / MIB,
            "default_heap_busy_mib": walk["busy_bytes"] / MIB,
            "size_184_blocks": slots[0]["count"] if slots else 0,
            "gpu": point["gpu_allocations"],
        })
    tail = series[warmup:]
    if len(tail) < 2:
        raise ValueError("Need at least two post-warmup rounds")
    metrics = {}
    xs = [p["round"] for p in tail]
    mean_x = statistics.mean(xs)
    for key in ("private_mib", "rust_live_mib", "default_heap_busy_mib", "size_184_blocks"):
        ys = [p[key] for p in tail]
        mean_y = statistics.mean(ys)
        metrics[key] = {
            "net": ys[-1] - ys[0],
            "slope_per_round": sum((x - mean_x) * (y - mean_y) for x, y in zip(xs, ys))
            / sum((x - mean_x) ** 2 for x in xs),
        }
    return {
        "report": str(path), "renderer": report["renderer"],
        "rounds": report["rounds"], "warmup_rounds": warmup,
        "mask_edits_verified": validated["mask_edits_verified"],
        "brush_strokes_verified": validated["brush_strokes_verified"],
        "elapsed_seconds": report["elapsed_seconds"],
        "tail": metrics, "final_idle_series": series,
        "scope": "Single diagnostic run; not G2 release acceptance, physical latency or independent block-owner proof.",
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("reports", type=Path, nargs="+")
    parser.add_argument("--warmup", type=int, default=3)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.warmup < 0:
        parser.error("warmup must be nonnegative")
    results = [analyze(path, args.warmup) for path in args.reports]
    with args.output.open("x", encoding="utf-8") as stream:
        json.dump(results, stream, indent=2)
        stream.write("\n")
    for result in results:
        print(result["report"], json.dumps(result["tail"], sort_keys=True))


if __name__ == "__main__":
    main()
