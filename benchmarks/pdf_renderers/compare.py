#!/usr/bin/env python3
"""Run, qualify, and retain a bounded comparison; never discard failing outputs."""
from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import importlib.metadata
import itertools
import json
import os
import platform
import random
import shutil
import statistics
import subprocess
import sys
import threading
import time
from pathlib import Path

import psutil

from fixtures import FONT, NAMES, ROOT, write_inputs
from validate import check_pdf, negative_controls

ENGINES = ("fullbleed", "weasyprint", "chromium")


def save(path: Path, data) -> None:
    path.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def stats(values: list[float]) -> dict:
    if not values:
        return {}
    quartiles = statistics.quantiles(values, n=4, method="inclusive") if len(values) > 1 else values * 3
    return {"n": len(values), "median": statistics.median(values), "min": min(values),
            "max": max(values), "q1": quartiles[0], "q3": quartiles[2]}


def invoke(command: list[str], directory: Path, memory: bool) -> dict:
    samples = []
    stopped = threading.Event()
    started = time.perf_counter_ns()
    process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)

    def sample():
        parent = psutil.Process(process.pid)
        while not stopped.is_set():
            total = 0
            try:
                members = [parent, *parent.children(recursive=True)]
            except psutil.Error:
                members = []
            for member in members:
                try:
                    total += member.memory_info().rss
                except psutil.Error:
                    pass
            samples.append({"ms": (time.perf_counter_ns() - started) / 1e6, "rss_bytes": total})
            stopped.wait(0.01)

    thread = threading.Thread(target=sample, daemon=True) if memory else None
    if thread:
        thread.start()
    try:
        stdout, stderr = process.communicate(timeout=180)
    except subprocess.TimeoutExpired:
        try:
            descendants = psutil.Process(process.pid).children(recursive=True)
            for child in descendants:
                child.kill()
        except psutil.Error:
            pass
        process.kill()
        stdout, stderr = process.communicate()
        stderr += "\nWorker exceeded 180-second timeout.\n"
    elapsed = (time.perf_counter_ns() - started) / 1e6
    stopped.set()
    if thread:
        thread.join()
    (directory / "stdout.jsonl").write_text(stdout, encoding="utf-8")
    (directory / "stderr.txt").write_text(stderr, encoding="utf-8")
    if memory:
        save(directory / "rss-samples.json", samples)
    rows = []
    metadata = {}
    for line in stdout.splitlines():
        try:
            item = json.loads(line)
        except json.JSONDecodeError:
            continue
        if "metadata" in item:
            metadata.update(item["metadata"])
        elif "token" in item:
            rows.append(item)
    return {"exit_code": process.returncode, "process_ms": elapsed, "rows": rows,
            "metadata": metadata, "sampled_peak_tree_rss_bytes": max((s["rss_bytes"] for s in samples), default=0)}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--smoke", action="store_true")
    parser.add_argument("--source-revision", help="Git revision of the copied source snapshot, when outside its checkout")
    args = parser.parse_args()
    output = args.out.resolve()
    if output.is_relative_to(ROOT):
        parser.error("--out must be outside the benchmark source directory")
    if sys.platform != "linux":
        parser.error("this comparison harness currently targets Linux")
    # WSL can inherit dozens of mounted Windows paths. ctypes library discovery
    # searches them, creating a host-specific delay unrelated to rendering.
    os.environ["PATH"] = str(Path(sys.executable).parent) + ":/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
    output.mkdir(parents=True, exist_ok=False)
    inputs = output / "inputs"
    write_inputs(inputs)
    shutil.copytree(ROOT, output / "source", ignore=shutil.ignore_patterns("__pycache__"))
    source_files = {str(p.relative_to(ROOT)): sha(p) for p in ROOT.rglob("*")
                    if p.is_file() and "__pycache__" not in p.parts}
    packages = {d.metadata["Name"]: d.version for d in importlib.metadata.distributions()}
    (output / "packages.txt").write_text("\n".join(f"{k}=={v}" for k, v in sorted(packages.items())) + "\n")
    git = subprocess.run(["git", "rev-parse", "HEAD"], cwd=ROOT, capture_output=True, text=True)
    cpu = ""
    cpuinfo = Path("/proc/cpuinfo")
    if cpuinfo.exists():
        cpu = next((s.split(":", 1)[1].strip() for s in cpuinfo.read_text().splitlines() if s.startswith("model name")), "")
    environment = {"utc": dt.datetime.now(dt.timezone.utc).isoformat(), "python": sys.version,
                   "platform": platform.platform(), "machine": platform.machine(), "cpu": cpu,
                   "logical_cpus": os.cpu_count(), "total_memory_bytes": psutil.virtual_memory().total,
                   "source_revision": args.source_revision or (git.stdout.strip() if git.returncode == 0 else None),
                   "source_sha256": source_files, "packages": packages,
                   "path_policy": "venv bin plus standard Linux executable directories; no inherited Windows PATH",
                   "font_sha256": sha(FONT), "input_sha256": {p.name: sha(p) for p in inputs.iterdir()},
                   "thread_environment": {k: os.environ.get(k) for k in ["FULLBLEED_THREADS", "OMP_NUM_THREADS", "RAYON_NUM_THREADS"]}}
    for command, filename in [(["-m", "weasyprint", "--info"], "weasyprint-info.txt"),
                              (["-m", "fullbleed", "agent-contract", "--format", "json"], "agent-contract.json")]:
        run = subprocess.run([sys.executable, *command], capture_output=True, text=True, timeout=60)
        (output / filename).write_text(run.stdout + run.stderr, encoding="utf-8")
        if run.returncode:
            raise RuntimeError(f"environment discovery failed: {filename}")
    save(output / "environment.json", environment)
    plan = {"schema": "fullbleed.renderer_comparison.v1", "smoke": args.smoke,
            "seed": 20261004, "cold_rounds": 1 if args.smoke else 5,
            "warm_blocks": 1 if args.smoke else 3, "warm_samples_per_block": 2 if args.smoke else 10,
            "warmups_per_block": 2, "memory_samples": 3, "engines": ENGINES, "fixtures": NAMES}
    save(output / "plan.json", plan)
    rng = random.Random(plan["seed"])
    combinations = list(itertools.product(ENGINES, NAMES))
    jobs = []
    token_counter = 1
    summaries = {f"{engine}/{fixture}": {"engine": engine, "fixture": fixture,
                 "cold_ms": [], "warm_ms": [], "valid": True, "failures": [], "pages": [], "bytes": []}
                 for engine, fixture in combinations}
    representatives = set()
    controls = {}
    phases = [("cold", plan["cold_rounds"], 1, 0),
              ("warm", plan["warm_blocks"], plan["warm_samples_per_block"], 2),
              ("memory", 1, plan["memory_samples"], 2)]
    for phase, rounds, count, warmups in phases:
        for block in range(rounds):
            ordering = combinations.copy()
            rng.shuffle(ordering)
            for engine, fixture in ordering:
                key = f"{engine}/{fixture}"
                label = f"{phase}-{block + 1}-{engine}-{fixture}"
                directory = output / "jobs" / label
                directory.mkdir(parents=True)
                command = [sys.executable, str(ROOT / "worker.py"), "--engine", engine,
                           "--fixture", fixture, "--inputs", str(inputs), "--font", str(FONT),
                           "--out", str(directory), "--count", str(count), "--warmups", str(warmups),
                           "--start-token", str(token_counter)]
                token_counter += count + warmups
                result = invoke(command, directory, phase == "memory")
                result.update({"job": label, "engine": engine, "fixture": fixture, "phase": phase, "block": block + 1})
                entry = summaries[key]
                if result["exit_code"] or len(result["rows"]) != count + warmups:
                    entry["valid"] = False
                    entry["failures"].append(f"{label}: worker failure or missing results")
                contract = json.loads((inputs / f"{fixture}.json").read_text())
                for row in result["rows"]:
                    path = Path(row["path"])
                    preview = output / "previews" / engine / fixture if key not in representatives else None
                    try:
                        qualification = check_pdf(path, contract, row["token"], preview)
                    except Exception as error:
                        qualification = {"passed": False, "failures": [f"PDF reader error: {type(error).__name__}: {error}"],
                                         "pages": 0, "bytes": path.stat().st_size, "sha256": sha(path)}
                    if preview:
                        representatives.add(key)
                        representative = output / "pdfs" / engine
                        representative.mkdir(parents=True, exist_ok=True)
                        shutil.copy2(path, representative / f"{fixture}.pdf")
                    row["path"] = str(path.relative_to(output)).replace(os.sep, "/")
                    row["qualification"] = qualification
                    entry["pages"].append(qualification["pages"])
                    entry["bytes"].append(qualification["bytes"])
                    if not qualification["passed"]:
                        entry["valid"] = False
                        entry["failures"].append({"job": label, "token": row["token"], "failures": qualification["failures"]})
                    if phase == "cold" and fixture == "ledger" and engine not in controls:
                        controls[engine] = negative_controls(path, contract, row["token"], output / "negative-controls" / engine)
                    if phase == "warm" and not row["warmup"]:
                        entry["warm_ms"].append(row["render_write_ms"])
                if phase == "cold":
                    entry["cold_ms"].append(result["process_ms"])
                if phase == "memory":
                    entry["sampled_peak_tree_rss_bytes"] = result["sampled_peak_tree_rss_bytes"]
                jobs.append(result)
                save(directory / "result.json", result)
                save(output / "jobs.json", jobs)
                print(f"{label}: {'PASS' if entry['valid'] else 'FAIL'}", flush=True)
    for entry in summaries.values():
        entry["cold_ms"] = stats(entry["cold_ms"])
        entry["warm_ms"] = stats(entry["warm_ms"])
        entry["pages"] = sorted(set(entry["pages"]))
        entry["bytes"] = stats(entry["bytes"])
    passed = all(e["valid"] for e in summaries.values()) and len(controls) == 3 and all(c["passed"] for c in controls.values())
    save(output / "negative-controls.json", controls)
    save(output / "summary.json", {"schema": plan["schema"], "smoke": args.smoke,
                                   "all_qualified": passed, "results": list(summaries.values())})
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
