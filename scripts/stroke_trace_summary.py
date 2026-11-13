"""Summarize a stroke trace written by a Dudel session.

Start Dudel with the environment variable DUDEL_TRACE_STROKE set to a file
path, paint, close Dudel, then:

    python scripts/stroke_trace_summary.py TRACE.jsonl

The file has one JSON line per frame of every stroke (src/stage_timer.rs).
This prints, per stroke: how long frames took, how far painting fell behind
the pen, and where the time went.
"""

import json
import sys


def percentile(values, p):
    if not values:
        return 0.0
    ordered = sorted(values)
    return ordered[max(1, -(-len(ordered) * p // 100)) - 1]


def strokes(frames):
    """Frames grouped into strokes: a gap of more than 250 ms starts a new one."""
    group = []
    for frame in frames:
        if group and frame["t_ms"] - group[-1]["t_ms"] > 250:
            yield group
            group = []
        group.append(frame)
    if group:
        yield group


def report(index, frames):
    dabs = sum(f["dabs"] for f in frames)
    if dabs == 0:
        return
    seconds = (frames[-1]["t_ms"] - frames[0]["t_ms"]) / 1000
    intervals = [f["interval_ms"] for f in frames[1:]]
    cpu = [f["frame_ms"] for f in frames]
    queued = [f["queued"] for f in frames]
    lift = next((i for i, f in enumerate(frames) if not f["stroking"]), len(frames) - 1)
    print(f"\n== stroke {index}: {seconds:.1f} s, {len(frames)} frames, {dabs} dabs")
    print(
        "   frame to frame: p50 {:.1f} / p95 {:.1f} / max {:.1f} ms ({:.0f} frames a second)".format(
            percentile(intervals, 50),
            percentile(intervals, 95),
            max(intervals, default=0.0),
            1000 / max(percentile(intervals, 50), 1e-9),
        )
    )
    print(
        "   CPU per frame:  p50 {:.1f} / p95 {:.1f} / max {:.1f} ms".format(
            percentile(cpu, 50), percentile(cpu, 95), max(cpu)
        )
    )
    print(
        "   behind the pen: up to {} dabs; {} at pen lift, painted {:.0f} ms later".format(
            max(queued), frames[lift]["queued"], frames[-1]["t_ms"] - frames[lift]["t_ms"]
        )
    )
    total = sum(cpu)
    stages = {}
    for frame in frames:
        for name, ms in frame["stages"].items():
            stages[name] = stages.get(name, 0.0) + ms
    parts = [
        f"{name} {ms:.0f} ms ({ms / total * 100:.0f}%, {ms * 1000 / dabs:.0f} us/dab)"
        for name, ms in sorted(stages.items(), key=lambda item: -item[1])
    ]
    print(f"   CPU time {total:.0f} ms: " + "; ".join(parts))
    accounted = sum(stages.get(name, 0.0) for name in ("pump", "paper_view", "expand", "display"))
    print(f"   rest of the interface: {total - accounted:.0f} ms ({(total - accounted) / len(frames):.1f} ms a frame)")
    third = -(-len(frames) // 3)
    for name, start in zip(("first", "middle", "last"), range(0, len(frames), third)):
        part = frames[start : start + third]
        part_dabs = max(1, sum(f["dabs"] for f in part))
        pump = sum(f["stages"].get("pump", 0.0) for f in part)
        display = sum(f["stages"].get("display", 0.0) for f in part)
        print(
            "   {} third: frame {:.1f} ms, pump {:.2f} ms/dab, display {:.2f} ms/frame, up to {} dabs behind".format(
                name,
                sum(f["frame_ms"] for f in part) / len(part),
                pump / part_dabs,
                display / len(part),
                max(f["queued"] for f in part),
            )
        )


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    with open(sys.argv[1], encoding="utf-8") as file:
        frames = [json.loads(line) for line in file if line.strip()]
    if not frames:
        sys.exit("The trace has no stroke frames.")
    for index, group in enumerate(strokes(frames), 1):
        report(index, group)


if __name__ == "__main__":
    main()
