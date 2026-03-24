#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import shutil
import statistics
import subprocess
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_CLI_BIN = REPO_ROOT / "target" / "release" / "notioncli"
DEFAULT_OUT_DIR = REPO_ROOT / "tmp" / "bench-strengthened"
SUITES = {
    "read": {
        "case_file": REPO_ROOT / "scripts" / "bench_cases.readsuite.json",
        "isolated_case_fixtures": False,
    },
    "write": {
        "case_file": REPO_ROOT / "scripts" / "bench_cases.writesuite.json",
        "isolated_case_fixtures": True,
    },
}


def run_command(command: list[str], *, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        command,
        cwd=REPO_ROOT,
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )


def check_command(command: list[str], *, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    completed = run_command(command, env=env)
    if completed.returncode != 0:
        message = completed.stderr.strip() or completed.stdout.strip()
        raise RuntimeError(f"command failed ({completed.returncode}): {' '.join(command)}\n{message}")
    return completed


def load_case_names(case_file: Path) -> list[str]:
    payload = json.loads(case_file.read_text(encoding="utf-8"))
    cases = payload.get("cases") if isinstance(payload, dict) else payload
    if not isinstance(cases, list):
        raise ValueError(f"invalid case file: {case_file}")
    names: list[str] = []
    for case in cases:
        if isinstance(case, dict) and isinstance(case.get("name"), str):
            names.append(case["name"])
    return names


def prepare_fixture(cli_bin: Path, root_parent_id: str | None, prefix: str) -> dict[str, str]:
    command = [
        "python3",
        "scripts/bench_prepare_fixture.py",
        "--cli-bin",
        str(cli_bin),
        "--format",
        "json",
        "--prefix",
        prefix,
    ]
    if root_parent_id:
        command.extend(["--root-parent-id", root_parent_id])
    completed = check_command(command)
    return json.loads(completed.stdout)


def cleanup_fixture(cli_bin: Path, container_page_id: str) -> None:
    check_command(
        [
            "python3",
            "scripts/bench_cleanup_fixture.py",
            "--cli-bin",
            str(cli_bin),
            "--container-page-id",
            container_page_id,
        ]
    )


def run_hyperfine(
    *,
    cli_bin: Path,
    case_file: Path,
    out_dir: Path,
    runs: int,
    warmup: int,
    env_updates: dict[str, str],
    cases: list[str] | None = None,
) -> None:
    env = os.environ.copy()
    env.update(env_updates)
    env["CASE_FILE"] = str(case_file)
    env["OUT_DIR"] = str(out_dir)
    env["RUNS"] = str(runs)
    env["WARMUP"] = str(warmup)
    env["CLI_BIN"] = str(cli_bin)
    command = ["scripts/hyperfine_compare.sh"]
    if cases:
        command.extend(cases)
    check_command(command, env=env)


def collect_case_summary(path: Path) -> dict[str, Any]:
    payload = json.loads(path.read_text(encoding="utf-8"))
    cli = next(item for item in payload["results"] if item["command"] == "cli")
    mcp = next(item for item in payload["results"] if item["command"] == "mcp")
    return {
        "cli_median_ms": cli["median"] * 1000.0,
        "mcp_median_ms": mcp["median"] * 1000.0,
        "cli_mean_ms": cli["mean"] * 1000.0,
        "mcp_mean_ms": mcp["mean"] * 1000.0,
    }


def summarize_sessions(session_dirs: list[Path]) -> dict[str, Any]:
    per_case: dict[str, dict[str, list[float]]] = {}
    for session_dir in session_dirs:
        for path in sorted(session_dir.glob("*.json")):
            metrics = collect_case_summary(path)
            case = path.stem
            bucket = per_case.setdefault(
                case,
                {
                    "cli_median_ms": [],
                    "mcp_median_ms": [],
                    "cli_mean_ms": [],
                    "mcp_mean_ms": [],
                },
            )
            for key, value in metrics.items():
                bucket[key].append(value)

    summary_cases = []
    for case, metrics in sorted(per_case.items()):
        cli_med = metrics["cli_median_ms"]
        mcp_med = metrics["mcp_median_ms"]
        cli_median_of_medians = statistics.median(cli_med)
        mcp_median_of_medians = statistics.median(mcp_med)
        if cli_median_of_medians < mcp_median_of_medians:
            ratio_text = f"CLI {mcp_median_of_medians / cli_median_of_medians:.2f}x faster"
        else:
            ratio_text = f"MCP {cli_median_of_medians / mcp_median_of_medians:.2f}x faster"
        summary_cases.append(
            {
                "case": case,
                "cli_session_medians_ms": [round(value, 1) for value in cli_med],
                "mcp_session_medians_ms": [round(value, 1) for value in mcp_med],
                "cli_median_of_medians_ms": round(cli_median_of_medians, 1),
                "mcp_median_of_medians_ms": round(mcp_median_of_medians, 1),
                "ratio_text": ratio_text,
            }
        )

    return {"cases": summary_cases}


def run_suite(
    *,
    suite_name: str,
    cli_bin: Path,
    root_parent_id: str | None,
    sessions: int,
    runs: int,
    warmup: int,
    out_dir: Path,
) -> dict[str, Any]:
    suite = SUITES[suite_name]
    case_file = Path(suite["case_file"])
    isolated_case_fixtures = bool(suite["isolated_case_fixtures"])
    case_names = load_case_names(case_file)
    suite_out_dir = out_dir / suite_name
    if suite_out_dir.exists():
        shutil.rmtree(suite_out_dir)
    suite_out_dir.mkdir(parents=True, exist_ok=True)

    session_dirs: list[Path] = []

    for session_index in range(1, sessions + 1):
        session_dir = suite_out_dir / f"session-{session_index:02d}"
        session_dir.mkdir(parents=True, exist_ok=True)
        session_dirs.append(session_dir)

        if isolated_case_fixtures:
            for case_name in case_names:
                prefix = f"bench-{suite_name}-{session_index:02d}-{case_name}"
                fixture = prepare_fixture(cli_bin, root_parent_id, prefix)
                try:
                    run_hyperfine(
                        cli_bin=cli_bin,
                        case_file=case_file,
                        out_dir=session_dir,
                        runs=runs,
                        warmup=warmup,
                        env_updates=fixture,
                        cases=[case_name],
                    )
                finally:
                    cleanup_fixture(cli_bin, fixture["CONTAINER_PAGE_ID"])
        else:
            prefix = f"bench-{suite_name}-{session_index:02d}"
            fixture = prepare_fixture(cli_bin, root_parent_id, prefix)
            try:
                run_hyperfine(
                    cli_bin=cli_bin,
                    case_file=case_file,
                    out_dir=session_dir,
                    runs=runs,
                    warmup=warmup,
                    env_updates=fixture,
                )
            finally:
                cleanup_fixture(cli_bin, fixture["CONTAINER_PAGE_ID"])

    summary = summarize_sessions(session_dirs)
    summary["suite"] = suite_name
    summary["sessions"] = sessions
    summary["runs_per_case"] = runs
    summary["warmup_per_case"] = warmup
    (suite_out_dir / "summary.json").write_text(
        json.dumps(summary, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return summary


def print_markdown_table(summary: dict[str, Any]) -> None:
    print(f"## {summary['suite'].title()} Suite")
    print("")
    print("| Case | CLI median of medians (ms) | MCP median of medians (ms) | Ratio |")
    print("|---|---:|---:|---:|")
    for case in summary["cases"]:
        print(
            f"| `{case['case']}` | {case['cli_median_of_medians_ms']:.1f} | "
            f"{case['mcp_median_of_medians_ms']:.1f} | {case['ratio_text']} |"
        )
    print("")


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Run a stronger repeated-session benchmark pass for the Notion CLI and hosted MCP."
    )
    parser.add_argument(
        "--suite",
        choices=("read", "write", "all"),
        default="all",
        help="Which benchmark suite to run",
    )
    parser.add_argument(
        "--sessions",
        type=int,
        default=2,
        help="How many full benchmark sessions to run per suite",
    )
    parser.add_argument(
        "--read-runs",
        type=int,
        default=20,
        help="Measured runs per read case",
    )
    parser.add_argument(
        "--read-warmup",
        type=int,
        default=3,
        help="Warmup runs per read case",
    )
    parser.add_argument(
        "--write-runs",
        type=int,
        default=10,
        help="Measured runs per write case",
    )
    parser.add_argument(
        "--write-warmup",
        type=int,
        default=2,
        help="Warmup runs per write case",
    )
    parser.add_argument(
        "--root-parent-id",
        default=os.environ.get("NOTION_TEST_PARENT_ID", ""),
        help="Shared parent page for fixture creation; falls back to search discovery",
    )
    parser.add_argument(
        "--cli-bin",
        type=Path,
        default=DEFAULT_CLI_BIN,
        help="Path to the notioncli binary",
    )
    parser.add_argument(
        "--out-dir",
        type=Path,
        default=DEFAULT_OUT_DIR,
        help="Directory to write benchmark artifacts into",
    )
    args = parser.parse_args()

    suites = ["read", "write"] if args.suite == "all" else [args.suite]
    args.out_dir.mkdir(parents=True, exist_ok=True)

    for suite_name in suites:
        if suite_name == "read":
            summary = run_suite(
                suite_name=suite_name,
                cli_bin=args.cli_bin,
                root_parent_id=args.root_parent_id or None,
                sessions=args.sessions,
                runs=args.read_runs,
                warmup=args.read_warmup,
                out_dir=args.out_dir,
            )
        else:
            summary = run_suite(
                suite_name=suite_name,
                cli_bin=args.cli_bin,
                root_parent_id=args.root_parent_id or None,
                sessions=args.sessions,
                runs=args.write_runs,
                warmup=args.write_warmup,
                out_dir=args.out_dir,
            )
        print_markdown_table(summary)


if __name__ == "__main__":
    main()
