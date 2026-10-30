"""Puts the binding package and the pure-Python engine on the path."""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import example_server  # noqa: E402,F401  (sets the rest of sys.path)
