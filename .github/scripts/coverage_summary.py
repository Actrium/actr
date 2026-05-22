#!/usr/bin/env python3
"""Build a weighted line-coverage summary from per-language reports."""

from __future__ import annotations

import argparse
import json
import sys
import xml.etree.ElementTree as ET
from pathlib import Path
from typing import Iterable


class CoverageReport:
    def __init__(self, language: str, report_format: str, path: Path, covered: int, total: int):
        self.language = language
        self.report_format = report_format
        self.path = path
        self.covered = covered
        self.total = total

    @property
    def percent(self) -> float:
        if self.total == 0:
            return 0.0
        return self.covered * 100.0 / self.total

    def to_json(self) -> dict[str, object]:
        return {
            "language": self.language,
            "format": self.report_format,
            "path": str(self.path),
            "covered_lines": self.covered,
            "total_lines": self.total,
            "line_coverage": round(self.percent, 2),
        }


def parse_lcov(language: str, path: Path) -> CoverageReport:
    covered = 0
    total = 0

    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if line.startswith("LH:"):
            covered += int(line.removeprefix("LH:"))
        elif line.startswith("LF:"):
            total += int(line.removeprefix("LF:"))

    if total == 0:
        raise ValueError(f"{language} LCOV report has no line totals: {path}")

    return CoverageReport(language, "lcov", path, covered, total)


def local_name(tag: str) -> str:
    return tag.rsplit("}", 1)[-1]


def line_counter_totals(counters: Iterable[ET.Element]) -> tuple[int, int] | None:
    covered = 0
    missed = 0
    found = False

    for counter in counters:
        if counter.attrib.get("type") != "LINE":
            continue
        covered += int(counter.attrib["covered"])
        missed += int(counter.attrib["missed"])
        found = True

    if not found:
        return None

    return covered, covered + missed


def direct_line_counters(element: ET.Element) -> list[ET.Element]:
    return [child for child in list(element) if local_name(child.tag) == "counter"]


def child_elements(element: ET.Element, name: str) -> list[ET.Element]:
    return [child for child in list(element) if local_name(child.tag) == name]


def parse_kover_xml(language: str, path: Path) -> CoverageReport:
    root = ET.parse(path).getroot()

    # Kover writes JaCoCo-style counters at multiple levels. Prefer the report-level
    # LINE counter to avoid counting the same lines more than once.
    totals = line_counter_totals(direct_line_counters(root))
    if totals is None:
        totals = line_counter_totals(
            counter
            for package in child_elements(root, "package")
            for counter in direct_line_counters(package)
        )
    if totals is None:
        totals = line_counter_totals(
            counter
            for package in child_elements(root, "package")
            for source_file in child_elements(package, "sourcefile")
            for counter in direct_line_counters(source_file)
        )
    if totals is None:
        raise ValueError(f"{language} XML report has no LINE counter: {path}")

    covered, total = totals
    if total == 0:
        raise ValueError(f"{language} XML report has no line totals: {path}")

    return CoverageReport(language, "kover-xml", path, covered, total)


def aggregate_reports(reports: list[CoverageReport]) -> dict[str, object]:
    covered = sum(report.covered for report in reports)
    total = sum(report.total for report in reports)
    line_coverage = round(covered * 100.0 / total, 2) if total else 0.0

    return {
        "covered_lines": covered,
        "total_lines": total,
        "line_coverage": line_coverage,
    }


def badge_color(percent: float) -> str:
    if percent >= 90.0:
        return "brightgreen"
    if percent >= 80.0:
        return "green"
    if percent >= 70.0:
        return "yellowgreen"
    if percent >= 60.0:
        return "yellow"
    if percent >= 50.0:
        return "orange"
    return "red"


def render_markdown(reports: list[CoverageReport], aggregate: dict[str, object]) -> str:
    lines = [
        "# Code Coverage Summary",
        "",
        "Total coverage is weighted by covered executable lines over total executable lines across all parsed reports.",
        "",
        "| Language | Report | Covered lines | Total lines | Line coverage |",
        "|----------|--------|---------------|-------------|---------------|",
    ]

    for report in reports:
        lines.append(
            f"| {report.language} | {report.report_format} | {report.covered} | "
            f"{report.total} | {report.percent:.2f}% |"
        )

    lines.extend(
        [
            f"| **Total** | weighted line coverage | {aggregate['covered_lines']} | "
            f"{aggregate['total_lines']} | **{aggregate['line_coverage']:.2f}%** |",
            "",
        ]
    )
    return "\n".join(lines)


def write_outputs(reports: list[CoverageReport], output_dir: Path) -> None:
    output_dir.mkdir(parents=True, exist_ok=True)
    aggregate = aggregate_reports(reports)

    summary = {
        "aggregate": aggregate,
        "languages": [report.to_json() for report in reports],
    }
    (output_dir / "coverage-summary.json").write_text(
        json.dumps(summary, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    (output_dir / "coverage-summary.md").write_text(
        render_markdown(reports, aggregate),
        encoding="utf-8",
    )
    (output_dir / "coverage-badge.json").write_text(
        json.dumps(
            {
                "schemaVersion": 1,
                "label": "coverage",
                "message": f"{aggregate['line_coverage']:.2f}%",
                "color": badge_color(float(aggregate["line_coverage"])),
            },
            indent=2,
            sort_keys=True,
        )
        + "\n",
        encoding="utf-8",
    )


def existing_path(value: str) -> Path:
    path = Path(value)
    if not path.is_file():
        raise argparse.ArgumentTypeError(f"not a file: {value}")
    return path


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rust-lcov", type=existing_path)
    parser.add_argument("--web-rust-lcov", type=existing_path)
    parser.add_argument("--typescript-lcov", type=existing_path)
    parser.add_argument("--python-lcov", type=existing_path)
    parser.add_argument("--kotlin-xml", type=existing_path)
    parser.add_argument("--output-dir", type=Path, required=True)
    return parser


def reports_from_args(args: argparse.Namespace) -> list[CoverageReport]:
    reports: list[CoverageReport] = []

    if args.rust_lcov:
        reports.append(parse_lcov("Rust", args.rust_lcov))
    if args.web_rust_lcov:
        reports.append(parse_lcov("Web Rust (host)", args.web_rust_lcov))
    if args.typescript_lcov:
        reports.append(parse_lcov("TypeScript", args.typescript_lcov))
    if args.python_lcov:
        reports.append(parse_lcov("Python", args.python_lcov))
    if args.kotlin_xml:
        reports.append(parse_kover_xml("Kotlin", args.kotlin_xml))

    if not reports:
        raise ValueError("at least one coverage report is required")

    return reports


def main(argv: list[str] | None = None) -> None:
    parser = build_parser()
    args = parser.parse_args(argv)

    reports = reports_from_args(args)
    write_outputs(reports, args.output_dir)
    print(f"Wrote coverage summary to {args.output_dir}", file=sys.stderr)


if __name__ == "__main__":
    main()
