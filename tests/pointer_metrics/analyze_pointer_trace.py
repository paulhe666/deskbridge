#!/usr/bin/env python3
"""Analyze pointer movement traces for stutter and jump metrics.

Input CSV schemas:
  t_ms,x,y
  t_ms,dx,dy

The script is intentionally dependency-free so it can run on Windows, macOS,
and Linux without installing Python packages.
"""

from __future__ import annotations

import argparse
import csv
import json
import math
import statistics
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable, Sequence


@dataclass(frozen=True)
class Point:
    t_ms: float
    x: float
    y: float


@dataclass(frozen=True)
class Metrics:
    sample_count: int
    duration_ms: float
    mean_interval_ms: float
    p50_interval_ms: float
    p95_interval_ms: float
    p99_interval_ms: float
    max_interval_ms: float
    interval_jitter_ms: float
    stutter_threshold_ms: float
    stutter_count: int
    stutter_rate: float
    total_distance_px: float
    mean_step_px: float
    p95_step_px: float
    max_step_px: float
    mean_velocity_px_s: float
    p95_velocity_px_s: float
    max_velocity_px_s: float
    velocity_jitter_px_s: float
    acceleration_spike_threshold_px_s2: float
    acceleration_spike_count: int
    max_acceleration_px_s2: float
    stutter_score: float

    def as_dict(self) -> dict[str, float | int]:
        return self.__dict__.copy()


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Analyze pointer movement traces for stutter, jitter, and jumps."
    )
    parser.add_argument("csv_path", type=Path, help="CSV trace file path")
    parser.add_argument(
        "--stutter-threshold-ms",
        type=float,
        default=20.0,
        help="Intervals larger than this are counted as stutters. Default: 20.0",
    )
    parser.add_argument(
        "--acceleration-spike-threshold",
        type=float,
        default=80000.0,
        help="Acceleration spike threshold in px/s^2. Default: 80000",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        help="Print metrics as JSON instead of a readable report.",
    )
    return parser.parse_args()


def read_trace(path: Path) -> list[Point]:
    if not path.exists():
        raise FileNotFoundError(f"trace file not found: {path}")

    with path.open("r", encoding="utf-8-sig", newline="") as handle:
        reader = csv.DictReader(handle)
        if reader.fieldnames is None:
            raise ValueError("CSV has no header")

        fields = {name.strip() for name in reader.fieldnames if name is not None}
        has_absolute = {"t_ms", "x", "y"}.issubset(fields)
        has_delta = {"t_ms", "dx", "dy"}.issubset(fields)
        if not has_absolute and not has_delta:
            raise ValueError("CSV must contain t_ms,x,y or t_ms,dx,dy columns")

        points: list[Point] = []
        current_x = 0.0
        current_y = 0.0
        for line_number, row in enumerate(reader, start=2):
            try:
                t_ms = float(required(row, "t_ms"))
                if has_absolute:
                    current_x = float(required(row, "x"))
                    current_y = float(required(row, "y"))
                else:
                    current_x += float(required(row, "dx"))
                    current_y += float(required(row, "dy"))
            except ValueError as exc:
                raise ValueError(f"invalid numeric value at line {line_number}: {exc}") from exc
            points.append(Point(t_ms=t_ms, x=current_x, y=current_y))

    validate_trace(points)
    return points


def required(row: dict[str, str | None], key: str) -> str:
    value = row.get(key)
    if value is None or value == "":
        raise ValueError(f"missing {key}")
    return value


def validate_trace(points: Sequence[Point]) -> None:
    if len(points) < 2:
        raise ValueError("trace must contain at least two points")
    previous = points[0].t_ms
    for index, point in enumerate(points[1:], start=1):
        if point.t_ms <= previous:
            raise ValueError(
                f"t_ms must be strictly increasing: row {index + 1} has {point.t_ms} <= {previous}"
            )
        previous = point.t_ms


def percentile(values: Sequence[float], percent: float) -> float:
    if not values:
        return 0.0
    ordered = sorted(values)
    if len(ordered) == 1:
        return ordered[0]
    rank = (len(ordered) - 1) * (percent / 100.0)
    lower = math.floor(rank)
    upper = math.ceil(rank)
    if lower == upper:
        return ordered[int(rank)]
    weight = rank - lower
    return ordered[lower] * (1.0 - weight) + ordered[upper] * weight


def stddev(values: Sequence[float]) -> float:
    if len(values) < 2:
        return 0.0
    return statistics.pstdev(values)


def distances(points: Sequence[Point]) -> list[float]:
    return [
        math.hypot(points[i].x - points[i - 1].x, points[i].y - points[i - 1].y)
        for i in range(1, len(points))
    ]


def intervals(points: Sequence[Point]) -> list[float]:
    return [points[i].t_ms - points[i - 1].t_ms for i in range(1, len(points))]


def velocities(step_px: Sequence[float], interval_ms: Sequence[float]) -> list[float]:
    return [
        step / (interval / 1000.0) if interval > 0.0 else 0.0
        for step, interval in zip(step_px, interval_ms)
    ]


def accelerations(velocity_px_s: Sequence[float], interval_ms: Sequence[float]) -> list[float]:
    output: list[float] = []
    for i in range(1, len(velocity_px_s)):
        dt_s = interval_ms[i] / 1000.0
        if dt_s <= 0.0:
            output.append(0.0)
        else:
            output.append(abs(velocity_px_s[i] - velocity_px_s[i - 1]) / dt_s)
    return output


def analyze(
    points: Sequence[Point],
    stutter_threshold_ms: float,
    acceleration_spike_threshold_px_s2: float,
) -> Metrics:
    interval_ms = intervals(points)
    step_px = distances(points)
    velocity_px_s = velocities(step_px, interval_ms)
    acceleration_px_s2 = accelerations(velocity_px_s, interval_ms)

    stutter_count = sum(1 for item in interval_ms if item > stutter_threshold_ms)
    stutter_rate = stutter_count / len(interval_ms) if interval_ms else 0.0
    acceleration_spike_count = sum(
        1 for item in acceleration_px_s2 if item > acceleration_spike_threshold_px_s2
    )

    p95_interval = percentile(interval_ms, 95.0)
    max_interval = max(interval_ms) if interval_ms else 0.0
    p95_step = percentile(step_px, 95.0)
    max_step = max(step_px) if step_px else 0.0
    velocity_jitter = stddev(velocity_px_s)

    # Composite score: lower is better. The scale is intentionally simple so it can
    # be compared across versions without hidden model parameters.
    stutter_score = (
        stutter_rate * 100.0
        + max(0.0, p95_interval - stutter_threshold_ms) * 1.5
        + max(0.0, max_interval - stutter_threshold_ms) * 0.5
        + min(50.0, velocity_jitter / 200.0)
        + min(50.0, max(0.0, max_step - p95_step) / 4.0)
    )

    return Metrics(
        sample_count=len(points),
        duration_ms=points[-1].t_ms - points[0].t_ms,
        mean_interval_ms=sum(interval_ms) / len(interval_ms),
        p50_interval_ms=percentile(interval_ms, 50.0),
        p95_interval_ms=p95_interval,
        p99_interval_ms=percentile(interval_ms, 99.0),
        max_interval_ms=max_interval,
        interval_jitter_ms=stddev(interval_ms),
        stutter_threshold_ms=stutter_threshold_ms,
        stutter_count=stutter_count,
        stutter_rate=stutter_rate,
        total_distance_px=sum(step_px),
        mean_step_px=sum(step_px) / len(step_px),
        p95_step_px=p95_step,
        max_step_px=max_step,
        mean_velocity_px_s=sum(velocity_px_s) / len(velocity_px_s),
        p95_velocity_px_s=percentile(velocity_px_s, 95.0),
        max_velocity_px_s=max(velocity_px_s) if velocity_px_s else 0.0,
        velocity_jitter_px_s=velocity_jitter,
        acceleration_spike_threshold_px_s2=acceleration_spike_threshold_px_s2,
        acceleration_spike_count=acceleration_spike_count,
        max_acceleration_px_s2=max(acceleration_px_s2) if acceleration_px_s2 else 0.0,
        stutter_score=stutter_score,
    )


def format_float(value: float) -> str:
    if abs(value) >= 1000.0:
        return f"{value:,.2f}"
    return f"{value:.3f}"


def print_report(metrics: Metrics) -> None:
    rows: Iterable[tuple[str, float | int, str]] = [
        ("sample_count", metrics.sample_count, "points"),
        ("duration_ms", metrics.duration_ms, "ms"),
        ("mean_interval_ms", metrics.mean_interval_ms, "ms"),
        ("p50_interval_ms", metrics.p50_interval_ms, "ms"),
        ("p95_interval_ms", metrics.p95_interval_ms, "ms"),
        ("p99_interval_ms", metrics.p99_interval_ms, "ms"),
        ("max_interval_ms", metrics.max_interval_ms, "ms"),
        ("interval_jitter_ms", metrics.interval_jitter_ms, "ms"),
        ("stutter_threshold_ms", metrics.stutter_threshold_ms, "ms"),
        ("stutter_count", metrics.stutter_count, "intervals"),
        ("stutter_rate", metrics.stutter_rate, "ratio"),
        ("total_distance_px", metrics.total_distance_px, "px"),
        ("mean_step_px", metrics.mean_step_px, "px"),
        ("p95_step_px", metrics.p95_step_px, "px"),
        ("max_step_px", metrics.max_step_px, "px"),
        ("mean_velocity_px_s", metrics.mean_velocity_px_s, "px/s"),
        ("p95_velocity_px_s", metrics.p95_velocity_px_s, "px/s"),
        ("max_velocity_px_s", metrics.max_velocity_px_s, "px/s"),
        ("velocity_jitter_px_s", metrics.velocity_jitter_px_s, "px/s"),
        (
            "acceleration_spike_threshold_px_s2",
            metrics.acceleration_spike_threshold_px_s2,
            "px/s^2",
        ),
        ("acceleration_spike_count", metrics.acceleration_spike_count, "events"),
        ("max_acceleration_px_s2", metrics.max_acceleration_px_s2, "px/s^2"),
        ("stutter_score", metrics.stutter_score, "lower is better"),
    ]

    print("Pointer trace metrics")
    print("=====================")
    for name, value, unit in rows:
        if isinstance(value, int):
            text = str(value)
        else:
            text = format_float(value)
        print(f"{name:36} {text:>14}  {unit}")


def main() -> int:
    args = parse_args()
    try:
        points = read_trace(args.csv_path)
        metrics = analyze(
            points,
            stutter_threshold_ms=args.stutter_threshold_ms,
            acceleration_spike_threshold_px_s2=args.acceleration_spike_threshold,
        )
    except Exception as exc:  # noqa: BLE001 - command line tool should print concise errors.
        print(f"error: {exc}", file=sys.stderr)
        return 1

    if args.json:
        print(json.dumps(metrics.as_dict(), ensure_ascii=False, indent=2, sort_keys=True))
    else:
        print_report(metrics)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
