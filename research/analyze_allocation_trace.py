"""Replay bounded large-allocation events; reject overflow or incomplete transactions."""
import argparse
from collections import defaultdict
import json
from pathlib import Path


class Replay:
    def __init__(self, minimum):
        self.minimum = minimum
        self.live = {}
        self.pending = {}
        self.unknown_frees = 0

    def add(self, pointer, size, frames):
        if not pointer or size < self.minimum or pointer in self.live:
            raise ValueError("invalid/duplicate live allocation")
        self.live[pointer] = (size, tuple(frames))

    def apply(self, event):
        kind, pointer = event["kind"], event["pointer"]
        if kind == 1:
            self.add(pointer, event["size"], event["frames"])
        elif kind == 2:
            previous = self.live.pop(pointer, None)
            if previous is not None and previous[0] != event["size"]:
                raise ValueError("deallocation layout disagrees with recorded allocation size")
            if previous is None:
                self.unknown_frees += 1  # Existing before the trace began.
        elif kind == 3:
            previous = self.live.pop(pointer, None)
            if previous is not None and previous[0] != event["size"]:
                raise ValueError("reallocation layout disagrees with recorded allocation size")
            self.pending[event["sequence"]] = (event, previous)
        elif kind == 4:
            transaction = event["transaction"]
            if transaction not in self.pending:
                raise ValueError("reallocation end has no matching begin")
            begin, previous = self.pending.pop(transaction)
            if pointer == 0:
                if previous is not None:
                    self.add(begin["pointer"], previous[0], previous[1])
            elif begin["new_size"] >= self.minimum:
                self.add(pointer, begin["new_size"], begin["frames"])
        else:
            raise ValueError(f"unknown event kind {kind}")

    def groups(self):
        groups = defaultdict(lambda: {"bytes": 0, "count": 0})
        for size, frames in self.live.values():
            groups[frames]["bytes"] += size
            groups[frames]["count"] += 1
        return dict(groups)


def analyze(trace_path, native_path=None):
    trace = json.loads(trace_path.read_text(encoding="utf-8"))
    if trace["dropped_events"]:
        raise ValueError("trace overflow: retained ownership is incomplete")
    events = trace["events"]
    if len(events) != trace["recorded_events"] or any(
            event["sequence"] != i + 1 for i, event in enumerate(events)):
        raise ValueError("missing or unordered trace events")
    symbols = trace.get("symbols", {})
    def describe(frames, values):
        stack = [symbols.get(str(frame), {}).get("name", f"0x{frame:x}") for frame in frames]
        skip = ("dudel::allocation_trace", "dudel::allocation_profile", "alloc::alloc",
                "alloc::raw_vec", "alloc::vec", "core::alloc", "std::alloc", "__rust")
        owner = next((name for name in stack if not name.startswith(skip)), "unresolved")
        return {**values, "owner": owner, "stack": stack, "addresses": list(frames)}

    native = json.loads(native_path.read_text(encoding="utf-8")) if native_path else None
    cuts = []
    if native:
        if not native.get("allocation_stack_profile_enabled"):
            raise ValueError("native report does not enable the allocation stack trace")
        labels = {"initial-idle", "history-released-before-save", "final-idle-history-released"}
        for point in native["checkpoints"]:
            if point["label"] in labels:
                cut = point["allocation_trace"]["event_position"]
                if not 0 <= cut <= len(events):
                    raise ValueError("checkpoint event cut outside trace coverage")
                histogram = point["allocation_size_live_bytes"]
                if len(histogram) != 32 or any(value < 0 for value in histogram):
                    raise ValueError("invalid allocation size histogram")
                cuts.append((cut, point))
    cuts.sort(key=lambda item: item[0])
    replay = Replay(trace["minimum_bytes"])
    minimum_bin = trace["minimum_bytes"].bit_length() - 1
    if trace["minimum_bytes"] != 1 << minimum_bin:
        raise ValueError("trace threshold must match a power-of-two histogram bin")
    index, samples, snapshots = 0, [], {}
    for cut, point in cuts:
        while index < cut:
            replay.apply(events[index])
            index += 1
        groups = replay.groups()
        large = sum(values["bytes"] for values in groups.values())
        sizes = point["allocation_size_live_bytes"]
        sample = {"label": point["label"], "round": point["round"], "event_position": cut,
                  "writers_in_flight": point["allocation_trace"]["writers_in_flight"],
                  "pending_reallocations": len(replay.pending), "tracked_large_bytes": large,
                  "histogram_large_bytes": sum(sizes[minimum_bin:]),
                  "histogram_small_bytes": sum(sizes[:minimum_bin]),
                  "global_rust_live_bytes": point["rust_allocations"]["live_requested_bytes"],
                  "untracked_large_difference_bytes": sum(sizes[20:]) - large}
        samples.append(sample)
        snapshots[(point["label"], point["round"])] = (sample, groups)
    while index < len(events):
        replay.apply(events[index])
        index += 1
    if replay.pending:
        raise ValueError("trace ended with incomplete reallocation transactions")
    comparisons = []
    if native:
        end_round = native.get("rounds", 1) - 1
        start_round = min(3, end_round)
        for label in sorted({sample["label"] for sample in samples}):
            before, before_groups = snapshots[(label, start_round)]
            after, after_groups = snapshots[(label, end_round)]
            group_changes = []
            for frames in before_groups.keys() | after_groups.keys():
                first = before_groups.get(frames, {"bytes": 0, "count": 0})
                last = after_groups.get(frames, {"bytes": 0, "count": 0})
                delta = {"bytes": last["bytes"] - first["bytes"], "count": last["count"] - first["count"]}
                if delta["bytes"]:
                    group_changes.append(describe(frames, delta))
            comparisons.append({"label": label, "from_round": start_round, "to_round": end_round,
                "quiet_cuts": all(sample["writers_in_flight"] == sample["pending_reallocations"] == 0
                                  for sample in [before, after]),
                "global_rust_drift_bytes": after["global_rust_live_bytes"] - before["global_rust_live_bytes"],
                "tracked_large_drift_bytes": after["tracked_large_bytes"] - before["tracked_large_bytes"],
                "histogram_large_drift_bytes": after["histogram_large_bytes"] - before["histogram_large_bytes"],
                "histogram_small_drift_bytes": after["histogram_small_bytes"] - before["histogram_small_bytes"],
                "group_changes": sorted(group_changes, key=lambda group: group["bytes"], reverse=True)})
    return {"source": str(trace_path), "native_source": str(native_path) if native_path else None,
            "minimum_bytes": trace["minimum_bytes"], "recorded_events": len(events),
            "dropped_events": 0, "trace_storage_bytes": trace["trace_storage_bytes"],
            "resolved_addresses": len(symbols), "image_base": trace["image_base"],
            "unknown_frees": replay.unknown_frees, "idle_samples": samples,
            "comparisons": comparisons,
            "live_groups": sorted([describe(frames, group) for frames, group in replay.groups().items()],
                                  key=lambda group: group["bytes"], reverse=True),
            "scope": "Events cover requests >= the recorded threshold after trace start. Earlier/smaller allocations are excluded. Cuts/histograms are concurrent samples; compare quiet cuts and check histogram agreement. No overflow/incomplete transactions accepted. Stack attribution describes retention, not proof of a leak or GPU timing."}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("trace", type=Path)
    parser.add_argument("--native-report", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = analyze(args.trace, args.native_report)
    with args.output.open("x", encoding="utf-8") as stream:
        json.dump(result, stream, indent=2)
        stream.write("\n")
    print("events", result["recorded_events"], "resolved addresses", result["resolved_addresses"])
    for group in result["live_groups"][:12]:
        print(group["bytes"], group["count"], group["owner"])
    for comparison in result["comparisons"]:
        print(comparison["label"], "large drift", comparison["tracked_large_drift_bytes"],
              "Rust drift", comparison["global_rust_drift_bytes"], "quiet", comparison["quiet_cuts"])
        for group in comparison["group_changes"][:4]:
            print(" ", group["bytes"], group["count"], group["owner"])


if __name__ == "__main__":
    main()
