"""Paths, and a loader for the order fulfillment example's server module (its
activities, and the pure-Python engine it builds)."""

import importlib.util
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", "..", "..", ".."))  # octopus_live_engine/
EXAMPLE_DIR = os.path.join(ROOT, "examples", "python", "order_fulfillment_api")
EXAMPLE_YAML = os.path.join(EXAMPLE_DIR, "order_fulfillment_workflow.yaml")

sys.path.insert(0, os.path.join(HERE, "..", "python"))
sys.path.insert(0, os.path.join(ROOT, "python", "src"))


def load_example_server(workdir: str):
    """Imports the example's server.py with `workdir` as the current
    directory, so the SQLite file its blueprint names is created there."""
    cwd = os.getcwd()
    os.chdir(workdir)
    try:
        spec = importlib.util.spec_from_file_location("order_fulfillment_server", os.path.join(EXAMPLE_DIR, "server.py"))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module
    finally:
        os.chdir(cwd)
