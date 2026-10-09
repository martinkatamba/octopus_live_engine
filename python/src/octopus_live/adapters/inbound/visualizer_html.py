from typing import Any, Dict, List, Optional
from .workflow_visualizer import render_x6_visualizer_html, blueprint_to_x6_graph
from ...domain.models.models import ExecutionState, WorkflowBlueprint
from ...domain.services.masking_service import mask_payload


class VisualizerHtmlRenderer:
    """Inbound adapter rendering the embedded interactive AntV X6 Visualizer Web Application."""

    @classmethod
    def render_html(
        cls,
        domain: str,
        api_prefix: str,
        domains_registry: Optional[List[Dict[str, Any]]] = None
    ) -> str:
        return render_x6_visualizer_html(domain, api_prefix, domains_registry)

    @classmethod
    def compile_x6_diagram(
        cls,
        blueprint: WorkflowBlueprint,
        state: Optional[ExecutionState] = None,
        endpoint_index: Optional[int] = None,
        route: Optional[str] = None,
        method: Optional[str] = None,
        available_activities: Optional[Dict[str, Any]] = None
    ) -> Dict[str, Any]:
        raw_dict = dict(blueprint.metadata) if (blueprint.metadata and isinstance(blueprint.metadata, dict)) else {}
        if "workflow_name" not in raw_dict:
            raw_dict["workflow_name"] = blueprint.workflow_name
        if "version" not in raw_dict:
            raw_dict["version"] = blueprint.version

        routes_list = []
        if blueprint.api:
            for ep in blueprint.api:
                routes_list.append({
                    "route": ep.route,
                    "method": ep.method,
                    "summary": ep.summary or ep.route,
                    "description": ep.description or "",
                    "step_count": len(ep.steps)
                })

        selected_route = None
        selected_method = None
        target_steps = blueprint.steps
        if blueprint.api:
            selected_ep = None
            if endpoint_index is not None and 0 <= endpoint_index < len(blueprint.api):
                selected_ep = blueprint.api[endpoint_index]
            elif route or method:
                for ep in blueprint.api:
                    if (not route or ep.route == route) and (not method or ep.method.upper() == method.upper()):
                        selected_ep = ep
                        break
            elif state and (state.route or state.method):
                for ep in blueprint.api:
                    if (not state.route or ep.route == state.route) and (not state.method or ep.method.upper() == str(state.method).upper()):
                        selected_ep = ep
                        break
            elif state and state.traces:
                trace_step_ids = {t.step_id for t in state.traces if t.step_id}
                trace_act_ids = {t.activity_name for t in state.traces if t.activity_name}
                for ep in blueprint.api:
                    ep_step_ids = {s.id for s in ep.steps if s.id}
                    ep_act_ids = {s.activity for s in ep.steps if s.activity}
                    if trace_step_ids.intersection(ep_step_ids) or trace_act_ids.intersection(ep_act_ids):
                        selected_ep = ep
                        break

            if selected_ep is None and len(blueprint.api) > 0:
                selected_ep = blueprint.api[0]

            if selected_ep is not None:
                selected_route = selected_ep.route
                selected_method = selected_ep.method
                if selected_ep.steps:
                    target_steps = selected_ep.steps
        elif blueprint.steps:
            selected_route = "default"
            selected_method = "EXECUTE"

        raw_dict["selected_route"] = selected_route
        raw_dict["selected_method"] = selected_method
        raw_dict["route"] = selected_route
        raw_dict["method"] = selected_method

        if blueprint.middlewares:
            mw_list = []
            for mw in blueprint.middlewares:
                mw_dict = {
                    "name": mw.name,
                    "type": mw.type,
                    "activity": mw.activity,
                    "is_async": mw.is_async,
                    "excludes": mw.excludes,
                    "includes": mw.includes,
                    "config": mw.config,
                    "exceptions": [
                        {
                            "exception": e.exception,
                            "code": e.code,
                            "message": e.message,
                            "activity": e.activity,
                            "is_async": e.is_async,
                            "headers": e.headers
                        }
                        for e in mw.exceptions
                    ] if mw.exceptions else []
                }
                mw_list.append(mw_dict)
            raw_dict["middlewares"] = mw_list

        if blueprint.error_handling and blueprint.error_handling.global_handler:
            gh = blueprint.error_handling.global_handler
            raw_dict["error_handling"] = {
                "global_handler": {
                    "activity": gh.activity,
                    "code": gh.code or 500,
                    "message": gh.message,
                    "async": gh.is_async,
                    "headers": gh.headers
                }
            }
        elif blueprint.global_exception_handler:
            gh = blueprint.global_exception_handler
            raw_dict["error_handling"] = {
                "global_handler": {
                    "activity": gh.activity,
                    "code": gh.code or 500,
                    "message": gh.message,
                    "async": gh.is_async,
                    "headers": gh.headers
                }
            }

        def _serialize_step(s) -> Dict[str, Any]:
            s_dict = {
                "id": s.id,
                "activity": s.activity,
                "description": s.description,
                "timeout_seconds": s.timeout_seconds,
                "compensation": s.compensation,
                "loop": s.loop,
                "for_each": s.for_each,
                "jump_to": s.jump_to,
                "async": s.is_async,
                "is_async": s.is_async,
                "is_output": s.is_output,
                "exceptions": [
                    {
                        "exception": e.exception if hasattr(e, "exception") else e.get("exception"),
                        "code": e.code if hasattr(e, "code") else e.get("code", 400),
                        "message": e.message if hasattr(e, "message") else e.get("message"),
                        "activity": e.activity if hasattr(e, "activity") else e.get("activity"),
                        "is_async": e.is_async if hasattr(e, "is_async") else (e.get("is_async") or e.get("async", False)),
                        "headers": e.headers if hasattr(e, "headers") else e.get("headers", {})
                    }
                    for e in s.exceptions
                ] if getattr(s, "exceptions", None) else []
            }
            if getattr(s, "condition", None):
                then_steps_list = [_serialize_step(ts) for ts in s.condition.then_steps]
                else_steps_list = [_serialize_step(es) for es in s.condition.else_steps]
                s_dict["branching_step"] = s.id
                s_dict["then_steps"] = then_steps_list
                s_dict["else_steps"] = else_steps_list
                s_dict["condition"] = {
                    "rule": s.condition.rule,
                    "operator": s.condition.operator,
                    "variable": s.condition.field,
                    "field": s.condition.field,
                    "value": s.condition.value,
                    "then_steps": then_steps_list,
                    "else_steps": else_steps_list
                }
                s_dict["conditions"] = s_dict["condition"]
            return s_dict

        raw_dict["steps"] = [_serialize_step(s) for s in target_steps]

        timeline = None
        executed_path = None
        current_step = None
        overall_status = None
        job_data = None

        if state:
            timeline = [
                {
                    "step_id": t.step_id,
                    "activity_name": t.activity_name,
                    "status": t.status,
                    "duration_ms": t.duration_ms,
                    "input": t.input_payload,
                    "output": t.output_payload,
                    "error": t.error_message,
                    "started_at": t.started_at,
                    "finished_at": t.finished_at
                }
                for t in state.traces
            ]
            executed_path = []
            for t in state.traces:
                if t.step_id:
                    executed_path.append(t.step_id)
                if t.activity_name:
                    executed_path.append(t.activity_name)
            current_step = state.current_step_id or (state.traces[-1].step_id if state.traces else None)
            overall_status = state.status.value if hasattr(state.status, "value") else str(state.status)
            fields, mask_val = blueprint.get_mask_config()
            job_data = mask_payload(state.data, fields, mask_val) if state.data else None

        diagram = blueprint_to_x6_graph(
            blueprint_cfg=raw_dict,
            timeline=timeline,
            current_step_name=current_step,
            overall_status=overall_status,
            executed_path=executed_path,
            job_data=job_data,
            available_activities=available_activities
        )
        diagram["routes"] = routes_list
        diagram["selected_route"] = selected_route
        diagram["selected_method"] = selected_method
        return diagram
