"""Phase 2 benchmark: the pure-Python engine (`octopus_live`) against the core,
called through the PyO3 binding and through the C ABI (ctypes).

Workloads:
- noop_N: N sequential steps whose activity returns {}. What remains is the
  engine's own cost per step, plus crossing the boundary.
- order_*: the order fulfillment example (persistence removed, so all three
  engines use their in-memory store) with its real activities.
- order_vip_50_items / order_vip_100_items: the same order with more items
  (the loop's `max_iterations: 20` raised so every item is reserved).
- order_standard_sqlite: the example's blueprint with its SQLite store, each
  engine writing to its own file. Not run on the C ABI, which is built
  without drivers.
- http_order_standard: the same order through FastAPI's TestClient.
- concurrent_*: throughput with 8 orders in flight at once (threads for the
  core, asyncio tasks for the Python engine, whose API is async).

Engines run in interleaved rounds so that load on the machine affects them
alike. Run with the release build:

    ./build.sh --release
    (cd ../.. && cargo build -p octopus-live-ffi --release --no-default-features)
    PYTHONPATH=python ../../../python/.venv/bin/python bench/bench.py
"""

import argparse
import asyncio
import ctypes
import json
import os
import platform
import re
import statistics
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "tests"))
from example_server import EXAMPLE_YAML, load_example_server  # noqa: E402  (also sets sys.path)

import octopus_core  # noqa: E402
from octopus_live import LiveEngine  # noqa: E402

CORE_ROOT = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
FFI_LIB = os.path.join(CORE_ROOT, "target", "release", "liboctopus_ffi.dylib")

STANDARD = {
    "customer_id": "cust_reg_99",
    "customer_tier": "STANDARD",
    "order_id": "ord_1002",
    "items": [{"sku": "SKU_CABLE_USB_C", "quantity": 2, "unit_price": 19.99}],
    "shipping_address": "123 Main St, Austin, TX",
}


def vip(items):
    return {
        "customer_id": "cust_vip_440",
        "customer_tier": "VIP",
        "order_id": "ord_1001",
        "items": [{"sku": f"SKU_{i}", "quantity": 1, "unit_price": 99.0} for i in range(items)],
        "shipping_address": "1 Infinite Loop, Cupertino, CA",
    }


VIP = vip(10)


def noop(_payload):
    return {}


def noop_blueprint(n):
    steps = "".join(f"  - id: s{i}\n    activity: Noop\n" for i in range(n))
    return f'workflow_name: noop_{n}\nversion: "1.0.0"\nsteps:\n{steps}'


def order_blueprint(sqlite_path=None):
    """The example's blueprint without `mode: hybrid`, so no engine
    replicates to Temporal, and with the loop allowed 1000 items. Without
    `sqlite_path` its `persistence:` is removed too, so what is measured is
    the engine with its in-memory store."""
    text = open(EXAMPLE_YAML).read()
    if sqlite_path is None:
        text = re.sub(r"\npersistence:\n(?:  .*\n|\n)*", "\n", text)
    else:
        text = re.sub(r"(?m)^(\s*sqlite_path:).*$", lambda m: f'{m.group(1)} "{sqlite_path}"', text)
        text = re.sub(r"(?m)^  temporal:\n(?:    .*\n)*", "", text)
    text = text.replace("max_iterations: 20", "max_iterations: 1000")
    return re.sub(r"(?m)^mode: .*\n", "", text)


def activity_calls(workload, payload):
    """Activity calls per run: the cost unit for "per step"."""
    if workload.startswith("noop_"):
        return int(workload.split("_")[1])
    # ValidateOrder, one ReserveInventoryItem per item, a shipping step,
    # SendCustomerNotification, FinalizeOrderResponse.
    return 4 + len(payload["items"])


# --- Engines -----------------------------------------------------------------


class PythonEngine:
    name = "python"

    def __init__(self, blueprints, activities):
        self.loop = asyncio.new_event_loop()
        self.engines = {}
        for wf, text in blueprints.items():
            path = os.path.join(tempfile.mkdtemp(), f"{wf}.yaml")
            with open(path, "w") as f:
                f.write(text)
            eng = LiveEngine.from_yaml(path, engine="python")
            for name, fn in activities.items():
                eng.register_activity(name, fn)
            self.engines[wf] = eng

    def run(self, wf, payload):
        state = self.loop.run_until_complete(self.engines[wf].execute_sync(wf, dict(payload)))
        return len(state.traces), state.status.value if hasattr(state.status, "value") else str(state.status)


class CoreEngine:
    name = "core (PyO3)"

    def __init__(self, blueprints, activities):
        self.engine = octopus_core.Engine()
        for name, fn in activities.items():
            self.engine.register_activity(name, fn)
        for text in blueprints.values():
            self.engine.register_blueprint(text)

    def run(self, wf, payload):
        state = self.engine.execute(wf, payload)
        return len(state["traces"]), state["status"]


class Buf(ctypes.Structure):
    _fields_ = [("ptr", ctypes.POINTER(ctypes.c_uint8)), ("len", ctypes.c_size_t), ("cap", ctypes.c_size_t)]


ACTIVITY_FN = ctypes.CFUNCTYPE(ctypes.c_int32, ctypes.c_void_p, ctypes.POINTER(ctypes.c_uint8), ctypes.c_size_t, ctypes.c_void_p)


class CAbiEngine:
    """The C ABI from Python with ctypes: JSON text both ways, as the .NET,
    Java and Go hosts will use it."""

    name = "core (C ABI)"

    def __init__(self, blueprints, activities):
        lib = self.lib = ctypes.CDLL(FFI_LIB)
        lib.octopus_engine_new.restype = ctypes.c_void_p
        lib.octopus_engine_new.argtypes = [ctypes.c_char_p]
        lib.octopus_register_blueprint.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
        lib.octopus_register_activity.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ACTIVITY_FN, ctypes.c_void_p]
        lib.octopus_execute.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_char_p, ctypes.POINTER(Buf)]
        lib.octopus_execute.restype = ctypes.c_int32
        lib.octopus_out_write.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_size_t]
        lib.octopus_buf_free.argtypes = [Buf]
        self.engine = lib.octopus_engine_new(None)
        self.callbacks = []  # ctypes callbacks must outlive the engine
        for name, fn in activities.items():
            cb = ACTIVITY_FN(self._trampoline(fn))
            self.callbacks.append(cb)
            assert lib.octopus_register_activity(self.engine, name.encode(), cb, None) == 0
        for text in blueprints.values():
            assert lib.octopus_register_blueprint(self.engine, text.encode()) == 0

    def _trampoline(self, fn):
        lib = self.lib

        def call(_user_data, data, length, out):
            try:
                result = fn(json.loads(ctypes.string_at(data, length))) or {}
                status, text = 0, json.dumps(result).encode()
            except Exception as e:  # reported to the engine as the step's error
                status, text = 1, f"{type(e).__name__}: {e}".encode()
            lib.octopus_out_write(out, text, len(text))
            return status

        return call

    def run(self, wf, payload):
        buf = Buf()
        status = self.lib.octopus_execute(self.engine, wf.encode(), json.dumps(payload).encode(), ctypes.byref(buf))
        state = json.loads(ctypes.string_at(buf.ptr, buf.len))
        self.lib.octopus_buf_free(buf)
        assert status == 0, state
        return len(state["traces"]), state["status"]


# --- Measuring ---------------------------------------------------------------


def measure(fn, runs):
    times = []
    for _ in range(runs):
        t0 = time.perf_counter_ns()
        fn()
        times.append(time.perf_counter_ns() - t0)
    return times


def summarize(times_ns):
    s = sorted(times_ns)
    return {
        "runs": len(s),
        "median_us": statistics.median(s) / 1000,
        "p95_us": s[int(len(s) * 0.95) - 1] / 1000,
    }


def activity_cost_us(activities, payload):
    """What the order workflow's activities cost when called directly, per run."""
    calls = [("ValidateOrder", payload)]
    calls += [("ReserveInventoryItem", dict(payload, item=i)) for i in payload["items"]]
    calls += [("ScheduleStandardShipping", payload), ("SendCustomerNotification", payload), ("FinalizeOrderResponse", payload)]
    t = measure(lambda: [activities[n](p) for n, p in calls], 2000)
    return statistics.median(t) / 1000


def interleaved(engines, workloads, rounds, runs, times, steps):
    """Times each workload on each engine, in rounds that take turns, after
    checking that every engine records the same traces and status."""
    for wl, (wf, payload, share) in workloads.items():
        seen = {e.name: e.run(wf, payload) for e in engines}
        assert len(set(seen.values())) == 1, (wl, seen)
        steps[wl] = activity_calls(wl, payload)
    for _ in range(rounds):
        for wl, (wf, payload, share) in workloads.items():
            n = max(int(runs * share), 5)
            for e in engines:
                times.setdefault((wl, e.name), []).extend(measure(lambda: e.run(wf, payload), n))


def throughput(engine, wf, payload, in_flight, total):
    """Runs per second with `in_flight` runs at once."""
    if isinstance(engine, PythonEngine):
        eng = engine.engines[wf]

        async def batch():
            sem = asyncio.Semaphore(in_flight)

            async def one():
                async with sem:
                    await eng.execute_sync(wf, dict(payload))

            await asyncio.gather(*(one() for _ in range(total)))

        t0 = time.perf_counter()
        engine.loop.run_until_complete(batch())
    else:
        from concurrent.futures import ThreadPoolExecutor

        with ThreadPoolExecutor(in_flight) as pool:
            t0 = time.perf_counter()
            list(pool.map(lambda _: engine.run(wf, payload), range(total)))
    return total / (time.perf_counter() - t0)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--rounds", type=int, default=10)
    ap.add_argument("--runs", type=int, default=100, help="runs per engine per round")
    ap.add_argument("--json", default=os.path.join(HERE, "results.json"))
    args = ap.parse_args()

    server = load_example_server(tempfile.mkdtemp())
    acts = server.OrderFulfillmentActivities()
    activities = {n: getattr(acts, n) for n in dir(acts) if not n.startswith("_") and callable(getattr(acts, n))}
    activities["Noop"] = noop

    blueprints = {f"noop_{n}": noop_blueprint(n) for n in (1, 10, 50)}
    blueprints["order_fulfillment_pipeline"] = order_blueprint()

    engines = [PythonEngine(blueprints, activities), CoreEngine(blueprints, activities)]
    if os.path.exists(FFI_LIB):
        engines.append(CAbiEngine(blueprints, activities))
    else:
        print(f"skipping the C ABI: {FFI_LIB} not built", file=sys.stderr)

    order = "order_fulfillment_pipeline"
    # workload -> (workflow, payload, share of --runs per round)
    workloads = {f"noop_{n}": (f"noop_{n}", {}, 1) for n in (1, 10, 50)}
    workloads["order_standard"] = (order, STANDARD, 1)
    workloads["order_vip_10_items"] = (order, VIP, 1)
    workloads["order_vip_50_items"] = (order, vip(50), 0.2)
    workloads["order_vip_100_items"] = (order, vip(100), 0.1)

    times, steps = {}, {}
    load_before = os.getloadavg()
    interleaved(engines, workloads, args.rounds, args.runs, times, steps)

    # SQLite against SQLite: each engine with its own file.
    sqlite_dir = tempfile.mkdtemp()
    sqlite_engines = [
        cls({order: order_blueprint(os.path.join(sqlite_dir, f"{i}.sqlite"))}, activities)
        for i, cls in enumerate((PythonEngine, CoreEngine))
    ]
    interleaved(sqlite_engines, {"order_standard_sqlite": (order, STANDARD, 0.5)}, args.rounds, args.runs, times, steps)

    # HTTP: the same order through FastAPI (fewer runs; TestClient is slow).
    from fastapi.testclient import TestClient
    from octopus_core.fastapi import create_app

    py_path = os.path.join(tempfile.mkdtemp(), "order.yaml")
    with open(py_path, "w") as f:
        f.write(order_blueprint())
    py_app = LiveEngine.from_yaml(py_path, activity_instance=acts, engine="python").create_api_app()
    core_app = create_app(octopus_core.from_yaml(py_path, activity_instance=acts))
    http = {"python": TestClient(py_app), "core (PyO3)": TestClient(core_app)}
    for name, client in http.items():
        assert client.post("/api/v1/orders/fulfill", json=STANDARD).json()["status"] == "FULFILLED", name
    http_runs = max(args.runs // 2, 10)
    for _ in range(args.rounds):
        for name, client in http.items():
            times.setdefault(("http_order_standard", name), []).extend(
                measure(lambda: client.post("/api/v1/orders/fulfill", json=STANDARD), http_runs)
            )
    steps["http_order_standard"] = steps["order_standard"]

    # Throughput with 8 runs in flight, against 1 at a time.
    rates = {}
    total = max(args.runs * 2, 50)
    for wl, payload in (("order_standard", STANDARD), ("order_vip_10_items", VIP)):
        for e in engines[:2]:
            for in_flight in (1, 8):
                rates[(wl, e.name, in_flight)] = statistics.median(
                    throughput(e, order, payload, in_flight, total) for _ in range(3)
                )
    load_after = os.getloadavg()

    activity_us = activity_cost_us(activities, STANDARD)
    results = {
        "machine": {
            "platform": platform.platform(),
            "python": sys.version.split()[0],
            "cpu_count": os.cpu_count(),
            "load_avg_before": load_before,
            "load_avg_after": load_after,
        },
        "standard_order_activities_direct_us": activity_us,
        "results": [],
        "throughput": [
            {"workload": wl, "engine": name, "in_flight": n, "runs_per_s": r} for (wl, name, n), r in rates.items()
        ],
    }
    for (wl, name), t in times.items():
        r = summarize(t)
        r.update(workload=wl, engine=name, steps=steps[wl], per_step_us=r["median_us"] / steps[wl])
        results["results"].append(r)
    with open(args.json, "w") as f:
        json.dump(results, f, indent=2)

    print(f"load average {load_before[0]:.0f} -> {load_after[0]:.0f}; Python {results['machine']['python']}")
    print(f"order_standard activities called directly: {activity_us:.1f} µs per run\n")
    print("| workload | activity calls | engine | median µs | p95 µs | µs per step | vs python |")
    print("|---|---:|---|---:|---:|---:|---:|")
    base = {r["workload"]: r["median_us"] for r in results["results"] if r["engine"] == "python"}
    for r in results["results"]:
        print(
            f"| {r['workload']} | {r['steps']} | {r['engine']} | {r['median_us']:.1f} | {r['p95_us']:.1f} "
            f"| {r['per_step_us']:.2f} | {base[r['workload']] / r['median_us']:.1f}× |"
        )
    print("\n| workload | engine | runs/s, 1 in flight | runs/s, 8 in flight |")
    print("|---|---|---:|---:|")
    for wl, name, n in rates:
        if n == 1:
            print(f"| {wl} | {name} | {rates[(wl, name, 1)]:.0f} | {rates[(wl, name, 8)]:.0f} |")


if __name__ == "__main__":
    main()
