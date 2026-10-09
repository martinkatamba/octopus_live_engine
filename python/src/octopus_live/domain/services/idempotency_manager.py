import collections
from typing import Any, Dict, Optional


class IdempotencyManager:
    """In-memory LRU idempotency cache preventing duplicate execution within a time window."""

    def __init__(self, capacity: int = 10000):
        self.capacity = capacity
        self._cache: collections.OrderedDict[str, Dict[str, Any]] = collections.OrderedDict()

    def get(self, key: str) -> Optional[Dict[str, Any]]:
        if key in self._cache:
            self._cache.move_to_end(key)
            return self._cache[key]
        return None

    def put(self, key: str, value: Dict[str, Any]) -> None:
        if key in self._cache:
            self._cache.move_to_end(key)
        self._cache[key] = value
        if len(self._cache) > self.capacity:
            self._cache.popitem(last=False)
