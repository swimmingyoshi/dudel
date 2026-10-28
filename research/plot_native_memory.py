"""Plot comparable native-process idle checkpoints from analyze_native_session.py."""
import argparse
import json
from pathlib import Path

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("summary", type=Path)
    parser.add_argument("output_stem", type=Path)
    parser.add_argument("--labels", nargs="+", help="Optional panel titles, one per run")
    args = parser.parse_args()
    outputs = [args.output_stem.with_suffix(suffix) for suffix in [".png", ".svg"]]
    if any(path.exists() for path in outputs):
        raise FileExistsError("Choose a new figure output stem")
    data = json.loads(args.summary.read_text(encoding="utf-8"))
    if args.labels is not None and len(args.labels) != len(data):
        parser.error("--labels must supply one title per run")
    metrics = [("private_committed_mib", "Private committed, MiB"), ("resident_mib", "Resident, MiB")]
    if all("rust_live_requested_mib" in p
           for run in data for series in run["idle_series"].values() for p in series["samples"]):
        metrics.append(("rust_live_requested_mib", "Live Rust requests, MiB"))
    figure, axes = plt.subplots(len(metrics), len(data), figsize=(6 * len(data), 3 * len(metrics) + 1), squeeze=False)
    labels = [("initial-idle", "Reset idle", "#0072B2"),
              ("history-released-before-save", "History released", "#D55E00"),
              ("final-idle-history-released", "Reopened idle", "#009E73")]
    for column, run in enumerate(data):
        for row, (metric, title) in enumerate(metrics):
            axis = axes[row][column]
            for key, label, color in labels:
                samples = run["idle_series"][key]["samples"]
                axis.plot([p["round"] + 1 for p in samples], [p[metric] for p in samples],
                          marker="o", markersize=3, linewidth=1.5, label=label, color=color)
            axis.set_ylabel(title)
            axis.set_xlabel("Round")
            axis.set_xticks(range(1, run["rounds"] + 1))
            axis.grid(axis="y", color="#dddddd", linewidth=0.6)
            axis.spines[["top", "right"]].set_visible(False)
            if row == 0:
                axis.set_title(args.labels[column] if args.labels else f"{run['zoom'] * 100:g}% application zoom")
    handles, names = axes[0][0].get_legend_handles_labels()
    figure.legend(handles, names, loc="upper center", ncol=3, bbox_to_anchor=(0.5, 0.94), frameon=False)
    figure.suptitle("Native memory at comparable idle checkpoints", fontsize=15, y=0.995)
    footer = "Generated input; fixed diagnostic baseline and frame reserve. These counters exclude VRAM and do not establish a leak verdict."
    if len(data) == 1:
        footer = "Generated input; fixed diagnostic baseline and frame reserve.\nCounters exclude VRAM and do not establish a leak verdict."
    figure.text(0.5, 0.015, footer,
                ha="center", fontsize=9)
    figure.tight_layout(rect=(0, 0.04, 1, 0.9))
    for output in outputs:
        figure.savefig(output, dpi=160)
    plt.close(figure)


if __name__ == "__main__":
    main()
