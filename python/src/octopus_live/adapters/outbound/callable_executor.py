import inspect
from typing import Any, Callable, Dict
from ...domain.ports.outbound import IActivityExecutor


class CallableActivityExecutor(IActivityExecutor):
    """Outbound adapter executing registered in-memory Python sync or async functions."""

    def __init__(self):
        self._activities: Dict[str, Callable[..., Any]] = {}

    def register(self, name: str, func: Callable[..., Any]) -> None:
        self._activities[name] = func
        # Also register normalized snake_case name
        snake = self._to_snake_case(name)
        if snake != name:
            self._activities[snake] = func

    def register_instance(self, instance: Any) -> None:
        for attr_name in dir(instance):
            if attr_name.startswith("_"):
                continue
            attr = getattr(instance, attr_name)
            if callable(attr):
                self.register(attr_name, attr)

    def has_activity(self, activity_name: str) -> bool:
        return activity_name in self._activities or self._to_snake_case(activity_name) in self._activities

    async def execute_activity(self, activity_name: str, payload: Dict[str, Any]) -> Dict[str, Any]:
        fn = self._activities.get(activity_name) or self._activities.get(self._to_snake_case(activity_name))
        if not fn:
            raise KeyError(f"Activity '{activity_name}' not registered in in-memory Live Engine")

        if inspect.iscoroutinefunction(fn):
            result = await fn(payload)
        else:
            result = fn(payload)

        return result if isinstance(result, dict) else {"result": result}

    async def execute(self, activity_name: str, payload: Dict[str, Any]) -> Dict[str, Any]:
        return await self.execute_activity(activity_name, payload)

    async def execute_async(self, activity_name: str, payload: Dict[str, Any]) -> Dict[str, Any]:
        return await self.execute_activity(activity_name, payload)

    @staticmethod
    def _to_snake_case(name: str) -> str:
        out = []
        for i, c in enumerate(name):
            if c.isupper():
                if i > 0 and name[i - 1] != "_":
                    out.append("_")
                out.append(c.lower())
            else:
                out.append(c)
        return "".join(out)
