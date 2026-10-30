#!/usr/bin/env python3
"""Runs the conformance cases on the Python live engine and prints the results as JSON."""

import asyncio
import json
import time
import sys
from pathlib import Path

ENGINE_SRC = Path(__file__).resolve().parents[4] / "python" / "src"
sys.path.insert(0, str(ENGINE_SRC))

from octopus_live.engine import OctopusLiveEngine  # noqa: E402


def scripted(name, script, calls, sequence):
    def activity(payload):
        n = calls.get(name, 0)
        calls[name] = n + 1
        sequence.append(name)
        entry = script[min(n, len(script) - 1)]
        if entry.get("delay_ms"):
            time.sleep(entry["delay_ms"] / 1000)
        if "fail" in entry:
            raise RuntimeError(entry["fail"])
        out = {key: payload.get(src) for key, src in entry.get("echo", {}).items()}
        out.update(entry.get("return", {}))
        return out
    return activity


ASYNC_WAIT_SECONDS = 5.0


def dump(st):
    out = st.model_dump(mode="json") if hasattr(st, "model_dump") else json.loads(json.dumps(st, default=vars))
    # The real output for a declared route's response is never part of the
    # serialized state (decision D5), as in the Go and Rust engines.
    out.pop("route_output", None)
    return out


async def stored_state(engine, job_id, wait_async):
    """The job as the state store holds it; with wait_async, once no step is still ASYNC_DISPATCHED."""
    deadline = asyncio.get_running_loop().time() + ASYNC_WAIT_SECONDS
    while True:
        try:
            st = dump(await engine.get_execution_state(job_id))
        except Exception:  # noqa: BLE001
            st = None
        pending = st and any(t.get("status") == "ASYNC_DISPATCHED" for t in st.get("traces") or [])
        if not wait_async or not pending or asyncio.get_running_loop().time() > deadline:
            return st
        await asyncio.sleep(0.02)


def new_engine(case_dir, case, calls, sequence):
    engine = OctopusLiveEngine()
    engine.load_blueprint_file(str(case_dir / "blueprint.yaml"))
    for name, script in case["activities"].items():
        engine.register_activity(name, scripted(name, script, calls, sequence))
    return engine


async def run_case(case_dir):
    case = json.loads((case_dir / "case.json").read_text())
    runs = []
    calls, sequence = {}, []
    engine = None
    for run in case["runs"]:
        if engine is None or not case.get("shared_engine"):
            calls, sequence = {}, []
            try:
                engine = new_engine(case_dir, case, calls, sequence)
            except Exception as e:  # noqa: BLE001
                runs.append({"load_error": f"{type(e).__name__}: {e}"})
                engine = None
                continue
        state, error, stored = None, None, None
        try:
            state = dump(await engine.execute_sync(case["workflow"], dict(run["input"])))
        except Exception as e:  # noqa: BLE001
            error = f"{type(e).__name__}: {e}"
        if state and state.get("job_id"):
            stored = await stored_state(engine, state["job_id"], run.get("wait_async", False))
        runs.append({"state": state, "stored": stored, "error": error, "calls": dict(calls), "sequence": list(sequence)})
    return runs


async def main(cases_dir):
    results = {}
    for d in sorted(Path(cases_dir).iterdir()):
        if (d / "case.json").exists():
            results[d.name] = await run_case(d)
    print(json.dumps({"engine": "python", "cases": results}, default=str))


if __name__ == "__main__":
    asyncio.run(main(sys.argv[1] if len(sys.argv) > 1 else Path(__file__).resolve().parents[2] / "cases"))
