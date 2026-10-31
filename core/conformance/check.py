#!/usr/bin/env python3
"""Compares each engine's recorded runs with the expectations in cases/*/case.json.

Usage: check.py RESULT.json [RESULT.json ...] [--report report.md] [--strict]

A result file is what a runner prints (see README.md). Exits 1 with --strict
when any case fails, otherwise 0: in phase 0 failures are findings, not errors.
"""

import json
import re
import sys
from pathlib import Path

CASES_DIR = Path(__file__).resolve().parent / "cases"


def same(expected, actual):
    """Equality where 1 == 1.0, but True is not 1."""
    if isinstance(expected, bool) or isinstance(actual, bool):
        return expected is actual
    if isinstance(expected, (int, float)) and isinstance(actual, (int, float)):
        return abs(expected - actual) < 1e-9
    if isinstance(expected, dict) and isinstance(actual, dict):
        return expected.keys() == actual.keys() and all(same(v, actual[k]) for k, v in expected.items())
    if isinstance(expected, list) and isinstance(actual, list):
        return len(expected) == len(actual) and all(same(e, a) for e, a in zip(expected, actual))
    return expected == actual


def subset(expected, actual, path, problems):
    """Every key in `expected` exists in `actual` with the same value (dicts compared recursively)."""
    for key, want in expected.items():
        where = f"{path}.{key}"
        if not isinstance(actual, dict) or key not in actual:
            problems.append(f"{where}: missing (want {json.dumps(want)})")
        elif isinstance(want, dict) and isinstance(actual[key], dict):
            subset(want, actual[key], where, problems)
        elif not same(want, actual[key]):
            problems.append(f"{where}: got {json.dumps(actual[key])}, want {json.dumps(want)}")


def check_run(expect, observed):
    problems = []
    if expect.get("load_fails"):
        return [] if observed.get("load_error") else ["blueprint loaded, want it refused"]
    if observed.get("load_error"):
        return [f"blueprint did not load: {observed['load_error']}"]
    if "diagram" in observed:
        nodes = (observed.get("diagram") or {}).get("nodes") or []
        problems += [f"diagram: no node {n!r} (nodes: {nodes})" for n in expect.get("diagram_nodes", []) if n not in nodes]
        problems += [f"diagram: node {n!r} present, want it absent" for n in expect.get("diagram_nodes_absent", []) if n in nodes]
        return problems
    state = observed.get("state") or {}
    error = observed.get("error") or ""
    data = state.get("data") or {}

    if "status" in expect and state.get("status") != expect["status"]:
        problems.append(f"status: got {state.get('status')!r}, want {expect['status']!r}"
                        + (f" (execute error: {error})" if error else ""))
    subset(expect.get("fields", {}), state, "state", problems)
    for key, pattern in expect.get("fields_match", {}).items():
        if not re.fullmatch(pattern, str(state.get(key))):
            problems.append(f"state.{key}: got {json.dumps(state.get(key))}, want it to match {pattern!r}")
    if "events" in expect:
        got = observed.get("events")
        if got is None:
            problems.append("events: the runner recorded none (set \"events\": true on the run)")
        else:
            names = [e.get("event") for e in got]
            want = [e["event"] for e in expect["events"]]
            if names != want:
                problems.append(f"events: got {names}, want {want}")
            else:
                for i, (w, h) in enumerate(zip(expect["events"], got)):
                    subset(w.get("data", {}), h.get("data") or {}, f"events[{i}].data", problems)
    subset(expect.get("data", {}), data, "data", problems)
    for key in expect.get("data_absent", []):
        if key in data:
            problems.append(f"data.{key}: present ({json.dumps(data[key])}), want absent")
    for key, n in expect.get("lengths", {}).items():
        got = data.get(key)
        if not isinstance(got, list) or len(got) != n:
            problems.append(f"data.{key}: got {json.dumps(got)}, want a list of {n}")
    if "traces" in expect:
        got = state.get("traces") or []
        brief = [(t.get("step_id"), t.get("status")) for t in got]
        if len(got) != len(expect["traces"]):
            problems.append(f"traces: got {len(got)} {brief}, want {len(expect['traces'])}")
        else:
            for i, (want, have) in enumerate(zip(expect["traces"], got)):
                for key, value in want.items():
                    actual = have.get(key, 1 if key == "attempts" else None)
                    if isinstance(value, dict):
                        subset(value, actual or {}, f"traces[{i}].{key}", problems)
                    elif not same(value, actual):
                        problems.append(f"traces[{i}].{key}: got {json.dumps(have.get(key))}, want {json.dumps(value)}")
    for name, n in expect.get("calls", {}).items():
        got = (observed.get("calls") or {}).get(name, 0)
        if got != n:
            problems.append(f"calls.{name}: got {got}, want {n}")
    if "sequence" in expect and observed.get("sequence") != expect["sequence"]:
        problems.append(f"sequence: got {observed.get('sequence')}, want {expect['sequence']}")
    if "error_contains" in expect:
        texts = [error, state.get("error") or ""] + [t.get("error_message") or "" for t in state.get("traces") or []]
        if not any(expect["error_contains"] in t for t in texts):
            problems.append(f"error: none of {json.dumps([t for t in texts if t])} contains {expect['error_contains']!r}")
    for text in expect.get("text_absent", []):
        if text in json.dumps(state):
            problems.append(f"{text!r} appears in the state, want it nowhere")
    if "stored" in expect:
        stored = observed.get("stored")
        if stored is None:
            problems.append("stored: the job was not found in the state store")
        else:
            inner = {"state": stored, "calls": observed.get("calls"), "sequence": observed.get("sequence"), "error": None}
            problems += [f"stored: {p}" for p in check_run(expect["stored"], inner)]
    return problems


def main(argv):
    strict = "--strict" in argv
    report_path = None
    files = []
    args = iter(a for a in argv if a != "--strict")
    for a in args:
        if a == "--report":
            report_path = next(args)
        else:
            files.append(a)
    if not files:
        print(__doc__)
        return 2

    cases = {d.name: json.loads((d / "case.json").read_text()) for d in sorted(CASES_DIR.iterdir()) if (d / "case.json").exists()}
    results = [json.loads(Path(f).read_text()) for f in files]
    engines = [r["engine"] for r in results]

    matrix = {}  # case -> engine -> list of problems (None = not run)
    for r in results:
        for name, case in cases.items():
            observed_runs = r["cases"].get(name)
            if observed_runs is None:
                matrix.setdefault(name, {})[r["engine"]] = None
                continue
            problems = []
            for i, run in enumerate(case["runs"]):
                obs = observed_runs[i] if i < len(observed_runs) else {"load_error": "run missing from results"}
                problems += [f"run {i + 1}: {p}" for p in check_run(run["expect"], obs)]
            matrix.setdefault(name, {})[r["engine"]] = problems

    # Cases with `decision_needed` test undocumented behaviour: a mismatch there
    # is a divergence to settle (⚠️), not a failure (❌).
    decided = [n for n in cases if "decision_needed" not in cases[n]]
    lines = ["| Case | " + " | ".join(engines) + " |", "|---|" + "---|" * len(engines)]
    failed = 0
    for name in cases:
        cells = []
        for e in engines:
            p = matrix[name][e]
            open_question = "decision_needed" in cases[name]
            cells.append("not run" if p is None else ("✅" if not p else ("⚠️" if open_question else "❌")))
            failed += bool(p) and not open_question
        lines.append(f"| {name} | " + " | ".join(cells) + " |")
    totals = []
    for e in engines:
        passed = sum(1 for n in decided if matrix[n][e] == [])
        totals.append(f"{e}: {passed}/{len(decided)}")
    lines += ["", "Passed (documented behaviour): " + ", ".join(totals),
              f"⚠️ = undocumented behaviour the engines disagree on ({len(cases) - len(decided)} cases need a decision).", ""]
    details = []
    for name in decided:
        for e in engines:
            if matrix[name][e]:
                details.append(f"### {name} — {e}\n" + "\n".join(f"- {p}" for p in matrix[name][e]))
    if details:
        lines += ["## Failures", ""] + details + [""]
    questions = []
    for name in cases:
        if "decision_needed" in cases[name]:
            diverging = [e for e in engines if matrix[name][e]]
            questions.append(f"### {name}\n{cases[name]['decision_needed']}\n\n"
                             + (f"Not matching the case as written: {', '.join(diverging)}." if diverging else "All engines match the case as written."))
    if questions:
        lines += ["## Needs a decision", ""] + questions

    text = "\n".join(lines) + "\n"
    print(text)
    if report_path:
        Path(report_path).write_text("# Conformance report\n\n" + text)
    return 1 if strict and failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
