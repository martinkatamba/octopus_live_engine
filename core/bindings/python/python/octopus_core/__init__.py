"""The OCTOPUS engine core for Python (phase 2 spike).

`Engine` is the Rust core, loaded from `_native`. Runs release the GIL; each
activity call takes it again, possibly on another thread, so activities must
be thread-safe. `fastapi.create_app` mounts an engine's declared routes on a
FastAPI app.

Treat a returned state as read-only: data the engine shares (a trace's input
and the job's data, for example) is the same dict in each place.
"""

from typing import Any, Optional

from ._native import CONTRACT_VERSION, CORE_VERSION, Engine, OctopusError, visualizer_html

__all__ = ["CONTRACT_VERSION", "CORE_VERSION", "Engine", "OctopusError", "from_yaml", "visualizer_html"]

# The `_native` contract this package was written against.
EXPECTED_CONTRACT_VERSION = 0
if CONTRACT_VERSION != EXPECTED_CONTRACT_VERSION:
    raise ImportError(
        f"octopus_core._native has contract version {CONTRACT_VERSION}, "
        f"this package needs {EXPECTED_CONTRACT_VERSION}"
    )


def register_instance(engine: Engine, instance: Any) -> None:
    """Registers each public callable of `instance` as an activity, by name
    (as `octopus_live` does)."""
    for name in dir(instance):
        if name.startswith("_"):
            continue
        attr = getattr(instance, name)
        if callable(attr):
            engine.register_activity(name, attr)


def from_yaml(
    path: str,
    activity_instance: Optional[Any] = None,
    api_key: str = "",
    options: Optional[dict] = None,
) -> Engine:
    """An engine with one blueprint file and, optionally, an object's
    methods as activities."""
    engine = Engine(api_key, options)
    if activity_instance is not None:
        register_instance(engine, activity_instance)
    engine.load_blueprint_file(path)
    return engine
