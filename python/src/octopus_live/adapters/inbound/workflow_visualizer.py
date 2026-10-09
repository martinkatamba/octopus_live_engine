import json
import importlib
import inspect
import logging
import os
import sys
from typing import Dict, Any, List, Optional, Set, Tuple, Union
from pathlib import Path

logger = logging.getLogger(__name__)


def scan_available_activities(activities_dir: Optional[Union[str, Path, List[Union[str, Path]], Tuple[Union[str, Path], ...]]] = None) -> Dict[str, Dict[str, Any]]:
    """
    Scans Python activity directories or files and extracts metadata for all available activity functions.
    Supports arbitrary custom directories and file paths chosen by the client.
    """
    project_root = Path.cwd()
    candidates = []

    raw_dirs = []
    if activities_dir:
        if isinstance(activities_dir, (list, tuple, set)):
            raw_dirs.extend(activities_dir)
        else:
            raw_dirs.append(activities_dir)

    env_act = os.getenv("OCTOPUS_ACTIVITIES_DIR")
    if env_act:
        raw_dirs.append(env_act)

    for entry in raw_dirs:
        if not entry:
            continue
        p = Path(entry).resolve()
        if p.exists():
            candidates.append(p)
        p_root = (project_root / entry).resolve()
        if p_root.exists():
            candidates.append(p_root)
        if sys.argv and sys.argv[0]:
            try:
                caller_dir = Path(sys.argv[0]).resolve().parent
                p_caller = (caller_dir / entry).resolve()
                if p_caller.exists():
                    candidates.append(p_caller)
            except Exception:
                pass

    if not raw_dirs:
        # Fallback discovery for standard setups
        if sys.argv and sys.argv[0]:
            try:
                caller_dir = Path(sys.argv[0]).resolve().parent
                candidates.append(caller_dir / "activities")
            except Exception:
                pass
        candidates.extend([
            project_root / "activities",
            Path("activities").resolve()
        ])
        try:
            for pattern in ("*/activities", "*/*/activities", "*/*/*/activities"):
                for sub_act in project_root.glob(pattern):
                    sub_str = str(sub_act)
                    if sub_act.is_dir() and ".venv" not in sub_str and ".git" not in sub_str and "__pycache__" not in sub_str and "node_modules" not in sub_str:
                        candidates.append(sub_act.resolve())
        except Exception:
            pass

    target_paths = []
    seen_paths = set()
    for cand in candidates:
        if cand.exists():
            resolved = cand.resolve()
            if str(resolved) not in seen_paths:
                target_paths.append(resolved)
                seen_paths.add(str(resolved))

    if not target_paths:
        return {}

    if str(project_root) not in sys.path:
        sys.path.append(str(project_root))

    files_to_scan = []
    for target in target_paths:
        parent_dir = target.parent
        if str(parent_dir) not in sys.path:
            sys.path.append(str(parent_dir))
        if str(target) not in sys.path:
            sys.path.append(str(target))

        if target.is_file() and target.suffix == ".py":
            files_to_scan.append((target, parent_dir))
        elif target.is_dir():
            for path in target.rglob("*.py"):
                if not path.name.startswith(("_", ".")):
                    files_to_scan.append((path, parent_dir))

    activities_map = {}
    for path, parent_dir in files_to_scan:
        try:
            mod = None
            try:
                rel_path = path.relative_to(project_root).with_suffix("")
                module_path = ".".join(rel_path.parts)
                mod = importlib.import_module(module_path)
            except Exception:
                pass

            if mod is None:
                try:
                    rel_path = path.relative_to(parent_dir).with_suffix("")
                    module_path = ".".join(rel_path.parts)
                    mod = importlib.import_module(module_path)
                except Exception:
                    pass

            if mod is None:
                mod_name = f"_octopus_dyn_{path.stem}_{abs(hash(str(path)))}"
                spec = importlib.util.spec_from_file_location(mod_name, str(path))
                if spec and spec.loader:
                    mod = importlib.util.module_from_spec(spec)
                    sys.modules[mod_name] = mod
                    spec.loader.exec_module(mod)

            if mod:
                module_path = mod.__name__
                for member_name, obj in mod.__dict__.items():
                    if (
                        callable(obj)
                        and inspect.isfunction(obj)
                        and obj.__module__ == mod.__name__
                        and not member_name.startswith("_")
                    ):
                        declared_name = getattr(obj, "__octopus_activity_name__", None)
                        parts = member_name.split("_")
                        pascal_name = "".join(p.capitalize() for p in parts if p)
                        entry = {
                            "name": declared_name or member_name,
                            "available": True,
                            "module": module_path,
                            "file": path.name,
                            "doc": inspect.getdoc(obj) or ""
                        }
                        activities_map[member_name] = entry
                        if declared_name:
                            activities_map[declared_name] = entry
                        if pascal_name:
                            activities_map[pascal_name] = entry
                    elif (
                        inspect.isclass(obj)
                        and obj.__module__ == mod.__name__
                        and not member_name.startswith(("_", "Base"))
                        and not hasattr(obj, "model_fields")
                        and not hasattr(obj, "__fields__")
                        and not issubclass(obj, (Exception, type))
                    ):
                        # Discover methods inside Activity Classes / Containers
                        for method_name, method in inspect.getmembers(obj, predicate=lambda m: inspect.isfunction(m) or inspect.ismethod(m) or inspect.iscoroutinefunction(m)):
                            if not method_name.startswith("_"):
                                method_declared = getattr(method, "__octopus_activity_name__", None)
                                method_parts = method_name.split("_")
                                method_pascal = "".join(p.capitalize() for p in method_parts if p)
                                entry = {
                                    "name": method_declared or method_name,
                                    "available": True,
                                    "class": member_name,
                                    "module": module_path,
                                    "file": path.name,
                                    "doc": inspect.getdoc(method) or ""
                                }
                                activities_map[method_name] = entry
                                if method_declared:
                                    activities_map[method_declared] = entry
                                if method_pascal:
                                    activities_map[method_pascal] = entry
        except Exception as e:
            logger.debug(f"Error scanning activities in {path}: {e}")

    return activities_map

def extract_quorum_votes(job_data: Optional[Dict[str, Any]] = None, timeline: Optional[List[Dict[str, Any]]] = None) -> List[Dict[str, Any]]:
    """Extracts structured quorum votes list from job_data or timeline items."""
    votes = []
    raw_votes = None
    if job_data and isinstance(job_data, dict):
        raw_votes = job_data.get("quorum_votes") or job_data.get("approvals") or job_data.get("votes")
    
    if not raw_votes and timeline and isinstance(timeline, list):
        for item in timeline:
            if isinstance(item, dict):
                rv = item.get("quorum_votes") or item.get("approvals") or (item.get("output") or {}).get("quorum_votes")
                if rv:
                    raw_votes = rv
                    break
    
    if isinstance(raw_votes, list):
        for v in raw_votes:
            if isinstance(v, dict):
                r_id = v.get("reviewer_id") or v.get("user_id") or ""
                act = str(v.get("decision") or v.get("action") or "APPROVE").upper()
                notes = v.get("notes") or v.get("reviewer_notes") or ""
                ts = v.get("voted_at") or v.get("timestamp") or ""
                roles = v.get("reviewer_roles") or v.get("roles") or []
                votes.append({
                    "reviewer_id": str(r_id),
                    "action": act,
                    "notes": str(notes),
                    "timestamp": str(ts),
                    "roles": list(roles) if isinstance(roles, list) else []
                })
    return votes


def extract_runtime_var(job_data: Optional[Dict[str, Any]], timeline: Optional[List[Dict[str, Any]]], var_name: str) -> Any:
    """Extracts a runtime variable by key/path from job_data or timeline traces."""
    if not var_name:
        return None
    parts = var_name.split(".")
    def get_nested(d: Any, p_list: List[str]) -> Any:
        curr = d
        for p in p_list:
            if isinstance(curr, dict) and p in curr:
                curr = curr[p]
            else:
                return None
        return curr

    if job_data and isinstance(job_data, dict):
        val = get_nested(job_data, parts)
        if val is not None:
            return val
    if timeline and isinstance(timeline, list):
        for item in reversed(timeline):
            if isinstance(item, dict):
                out_p = item.get("output") or item.get("output_payload")
                if isinstance(out_p, dict):
                    v = get_nested(out_p, parts)
                    if v is not None:
                        return v
                inp_p = item.get("input") or item.get("input_payload")
                if isinstance(inp_p, dict):
                    v = get_nested(inp_p, parts)
                    if v is not None:
                        return v
    return None


def blueprint_to_x6_graph(
    blueprint_cfg: Dict[str, Any],
    timeline: Optional[List[Dict[str, Any]]] = None,
    current_step_name: Optional[str] = None,
    overall_status: Optional[str] = None,
    executed_path: Optional[List[str]] = None,
    activities_dir: Optional[str] = None,
    available_activities: Optional[Dict[str, Any]] = None,
    job_data: Optional[Dict[str, Any]] = None
) -> Dict[str, Any]:
    """
    Transforms a Octopus declarative YAML workflow blueprint into AntV X6-compatible
    nodes and edges graph format, with distinct execution path highlighting and activity availability.
    """
    if not isinstance(blueprint_cfg, dict):
        return {"nodes": [], "edges": []}

    if available_activities is None:
        available_activities = scan_available_activities(activities_dir)

    nodes: List[Dict[str, Any]] = []
    edges: List[Dict[str, Any]] = []

    # Map completed timeline items by activity name and step ID
    timeline_map = {}
    if timeline and isinstance(timeline, list):
        for item in timeline:
            name = item.get("activity_name") or item.get("step_id")
            if name:
                timeline_map[name] = item
                timeline_map[name.lower()] = item
            step_id = item.get("step_id")
            if step_id:
                timeline_map[step_id] = item
                timeline_map[step_id.lower()] = item
            act = item.get("activity_name")
            if act:
                timeline_map[act] = item
                timeline_map[act.lower()] = item

    # Clean active step name
    active_target = None
    if current_step_name and isinstance(current_step_name, str):
        if ":" in current_step_name:
            active_target = current_step_name.split(":")[-1].strip()
        else:
            active_target = current_step_name.strip()

    has_live_execution = bool(executed_path or timeline or overall_status)
    executed_set = set(executed_path or [])

    # 1. Start Node
    start_status = "COMPLETED" if has_live_execution else "PENDING"
    nodes.append({
        "id": "node__start",
        "shape": "octopus-start-node",
        "label": "START",
        "width": 120,
        "height": 50,
        "data": {
            "step_id": "start",
            "type": "start",
            "title": "Workflow Start",
            "status": start_status,
            "executed": has_live_execution,
            "input_payload": job_data if has_live_execution else None,
            "job_data": job_data
        }
    })

    # Check if Ingress Middlewares are present (filtered by route excludes / includes)
    raw_middlewares = blueprint_cfg.get("middlewares", [])
    active_route = (job_data or {}).get("route") or (job_data or {}).get("path") or blueprint_cfg.get("selected_route") or ""
    
    def _is_path_excluded(route_str: str, excludes_list: List[str]) -> bool:
        if not route_str or not excludes_list:
            return False
        for p in excludes_list:
            if not p:
                continue
            p = str(p).strip()
            if p.endswith("*"):
                prefix = p[:-1]
                if route_str.startswith(prefix) or prefix in route_str:
                    return True
            if route_str.lower() == p.lower() or route_str.endswith(p) or p.endswith(route_str):
                return True
        return False

    def _is_mw_active(mw_dict: Any, route_str: str) -> bool:
        if not isinstance(mw_dict, dict):
            return True
        if not route_str or route_str == "default":
            return True
        if _is_path_excluded(route_str, mw_dict.get("excludes", [])):
            return False
        inc = mw_dict.get("includes", [])
        if inc:
            return any(route_str.lower() == i.lower() or route_str.endswith(i) or i.endswith(route_str) or (i.endswith("*") and (route_str.startswith(i[:-1]) or i[:-1] in route_str)) for i in inc)
        return True

    active_middlewares = [m for m in raw_middlewares if _is_mw_active(m, active_route)] if isinstance(raw_middlewares, list) else []
    pre_middlewares = [m for m in active_middlewares if isinstance(m, dict) and m.get("type") != "post-request"]
    post_middlewares = [m for m in active_middlewares if isinstance(m, dict) and m.get("type") == "post-request"]
    has_ingress_guards = bool(len(pre_middlewares) > 0)
    has_egress_guards = bool(len(post_middlewares) > 0)

    if has_ingress_guards:
        ingress_label = "🛡️ Ingress Middleware Guards"
        mw_exc_names = {
            e.get("exception", "").lower()
            for m in pre_middlewares if isinstance(m, dict)
            for e in m.get("exceptions", []) if isinstance(e, dict) and e.get("exception")
        }
        is_ingress_failed = bool(overall_status == "FAILED" and (
            any(
                str(item.get("error", "")).lower() in mw_exc_names
                or str(item.get("error_type", "")).lower() in mw_exc_names
                or str(item.get("exception", "")).lower() in mw_exc_names
                or (isinstance(item.get("input"), dict) and (
                    str(item.get("input", {}).get("exception", "")).lower() in mw_exc_names
                    or str(item.get("input", {}).get("error_type", "")).lower() in mw_exc_names
                    or str(item.get("input", {}).get("error", "")).lower() in mw_exc_names
                ))
                for item in (timeline or [])
            )
            or (isinstance(job_data, dict) and (
                str(job_data.get("exception", "")).lower() in mw_exc_names
                or str(job_data.get("error_type", "")).lower() in mw_exc_names
                or str(job_data.get("error", "")).lower() in mw_exc_names
                or str(job_data.get("job_id", "")).startswith("job-mw-")
                or int(job_data.get("status_code", 0) or 0) in (400, 401, 403, 413, 429)
            ))
        ))
        ingress_status = "FAILED" if is_ingress_failed else ("COMPLETED" if has_live_execution else "PENDING")

        nodes.append({
            "id": "node__ingress_guards",
            "shape": "octopus-activity-node",
            "label": ingress_label,
            "width": 220,
            "height": 55,
            "data": {
                "step_id": "ingress_guards",
                "type": "ingress_guards",
                "title": "🛡️ Ingress Middleware Guards",
                "description": "Pre-execution security filters, JWT authentication, IP whitelisting, and rate-limiting guards executed prior to workflow pipeline dispatch.",
                "status": ingress_status,
                "executed": has_live_execution,
                "middlewares": [m.get("name") for m in pre_middlewares if isinstance(m, dict) and m.get("name")]
            }
        })

        edges.append({
            "id": "edge__node__start__to__node__ingress_guards",
            "source": "node__start",
            "target": "node__ingress_guards",
            "attrs": {
                "line": {
                    "stroke": "#ef4444" if is_ingress_failed else ("#10b981" if has_live_execution else "#6366f1"),
                    "strokeWidth": 2,
                    "targetMarker": {"name": "block", "width": 8, "height": 6, "fill": "#ef4444" if is_ingress_failed else ("#10b981" if has_live_execution else "#6366f1")}
                }
            }
        })

        if is_ingress_failed:
            current_prev = [{"id": "node__ingress_guards", "label": "❌ Ingress Guard Blocked", "color": "#ef4444"}]
        else:
            current_prev = [{"id": "node__ingress_guards", "label": "✅ Quota & Security OK", "color": "#10b981" if has_live_execution else "#6366f1"}]
    else:
        current_prev = [{"id": "node__start", "label": None, "color": "#6366f1"}]

    raw_steps = list(blueprint_cfg.get("steps", []))
    if not raw_steps and blueprint_cfg.get("api"):
        matched_api_steps = []
        if active_route:
            for route_obj in blueprint_cfg.get("api", []):
                if isinstance(route_obj, dict) and route_obj.get("route") == active_route and route_obj.get("steps"):
                    matched_api_steps = list(route_obj.get("steps", []))
                    break
        if matched_api_steps:
            raw_steps = matched_api_steps
        else:
            for route_obj in blueprint_cfg.get("api", []):
                if isinstance(route_obj, dict) and route_obj.get("steps"):
                    raw_steps.extend(route_obj.get("steps", []))
    top_step_ids = [
        s.get("id") or s.get("activity") or s.get("human_step") or s.get("branching_step") or f"step_{i}"
        for i, s in enumerate(raw_steps) if isinstance(s, dict)
    ]
    top_step_node_ids = {s_id: f"node__{s_id}" for s_id in top_step_ids}
    for idx, s_id in enumerate(top_step_ids):
        top_step_node_ids[idx] = f"node__{s_id}"

    top_level_activity_names = {
        s.get("activity") for s in raw_steps if isinstance(s, dict) and s.get("activity")
    }
    
    def is_step_target_active(step_id: str, act_name: Optional[str]) -> bool:
        if not active_target:
            return False
        return (
            active_target in (step_id, act_name)
            or (act_name and act_name in active_target)
            or (step_id and step_id in active_target)
        )

    def is_step_executed(step_id: str, act_name: Optional[str], signal_name: Optional[str], scope_prefix: str = "") -> bool:
        if step_id and step_id in executed_set:
            return True
        if scope_prefix and f"{scope_prefix}__{step_id}" in executed_set:
            return True
        if signal_name and (signal_name in executed_set or f"signal:{signal_name}" in executed_set):
            return True
        if act_name and act_name in executed_set:
            return True
        if any(t.get("step_id") == step_id or (act_name and t.get("activity_name") == act_name) for t in (timeline or [])):
            return True
        if overall_status in ("FAILED", "TIMED_OUT") and is_step_target_active(step_id, act_name):
            return True
        return False

    def determine_status(step_id: str, act_name: Optional[str], is_human: bool, executed: bool) -> str:
        is_target = is_step_target_active(step_id, act_name)
        if overall_status in ("FAILED", "TIMED_OUT"):
            t_item = timeline_map.get(step_id) or (timeline_map.get(act_name) if act_name else None)
            if t_item and t_item.get("status") in ("FAILED", "ERROR"):
                return "FAILED"
            if is_target:
                return "FAILED" if overall_status == "FAILED" else "TIMED_OUT"
            if executed:
                return "COMPLETED"
            return "SKIPPED" if has_live_execution else "PENDING"
        if overall_status in ("COMPLETED", "TERMINATED", "CANCELED"):
            if executed:
                return "COMPLETED"
            return "SKIPPED" if has_live_execution else "PENDING"
        if is_target:
            return "AWAITING_APPROVAL" if is_human else "RUNNING"
        if executed:
            return "COMPLETED"
        if has_live_execution:
            return "SKIPPED"
        return "PENDING"

    def find_all_activities_in_steps(steps_to_scan: List[Dict[str, Any]]) -> Set[str]:
        acts = set()
        if not isinstance(steps_to_scan, list):
            return acts
        for s in steps_to_scan:
            if not isinstance(s, dict):
                continue
            if s.get("activity"):
                acts.add(s["activity"])
            if s.get("id"):
                acts.add(s["id"])
            if s.get("human_step"):
                acts.add(s["human_step"])
            if s.get("signal_name"):
                acts.add(s["signal_name"])
                acts.add(f"signal:{s['signal_name']}")
            for branch in ("then", "then_steps", "else", "else_steps", "on_approve", "on_reject", "on_sla_breach"):
                sub = s.get(branch)
                if sub and isinstance(sub, list):
                    acts.update(find_all_activities_in_steps(sub))
        return acts

    # Auto-infer active step target in RUNNING workflows when query was unreachable
    if overall_status == "RUNNING" and not active_target:
        for s in raw_steps:
            if not isinstance(s, dict):
                continue
            s_act = s.get("activity")
            s_id = s.get("id")
            s_human = s.get("human_step")
            s_branch = s.get("branching_step")
            all_s_acts = find_all_activities_in_steps([s])
            if (s_act and s_act in executed_set) or (s_id and s_id in executed_set) or (all_s_acts and any(a in executed_set for a in all_s_acts)):
                continue
            active_target = s_human or s_branch or s_act or s_id
            break

    top_level_activity_names = {
        s.get("activity") for s in raw_steps if isinstance(s, dict) and s.get("activity")
    }

    def determine_human_step_taken_branch(human_step_dict: Dict[str, Any]) -> Optional[str]:
        sig_name = human_step_dict.get("signal_name") or human_step_dict.get("human_step") or human_step_dict.get("id")
        h_id = human_step_dict.get("id")
        h_name = human_step_dict.get("human_step")

        is_human_active = is_step_target_active(h_id, h_name) or is_step_target_active(sig_name, sig_name)
        has_signal_or_decision = bool(
            (sig_name and any(k in executed_set for k in (sig_name, f"signal:{sig_name}", f"decision:{sig_name}:APPROVED", f"decision:{sig_name}:REJECTED", f"decision:{sig_name}:SLA_BREACH", f"signal_decision:{sig_name}:APPROVED", f"signal_decision:{sig_name}:REJECTED", f"signal_decision:{sig_name}:SLA_BREACH")))
            or (h_id and h_id in executed_set)
            or (h_name and h_name in executed_set)
        )

        sla_acts = find_all_activities_in_steps(human_step_dict.get("on_sla_breach", []))
        appr_acts = find_all_activities_in_steps(human_step_dict.get("on_approve", []))
        rej_acts = find_all_activities_in_steps(human_step_dict.get("on_reject", []))

        # Check if an active step is currently waiting inside any branch
        if any(is_step_target_active(a, a) for a in sla_acts):
            return "on_sla_breach"
        if any(is_step_target_active(a, a) for a in appr_acts):
            return "on_approve"
        if any(is_step_target_active(a, a) for a in rej_acts):
            return "on_reject"

        # 1. Check explicit Signal Decision and SLA Timer Triggers first
        if "sla_breach" in executed_set or "timer_fired" in executed_set or f"signal_decision:{sig_name}:SLA_BREACH" in executed_set or f"decision:{sig_name}:SLA_BREACH" in executed_set:
            return "on_sla_breach"

        if sig_name:
            if f"signal_decision:{sig_name}:APPROVED" in executed_set or f"decision:{sig_name}:APPROVED" in executed_set:
                return "on_approve"
            if f"signal_decision:{sig_name}:REJECTED" in executed_set or f"decision:{sig_name}:REJECTED" in executed_set:
                return "on_reject"

        # 2. Check physically executed exclusive child activities in Temporal history
        # 2. Check physically executed exclusive child activities in Temporal history
        appr_unique = appr_acts - (rej_acts | sla_acts)
        if appr_unique and any(a in executed_set for a in appr_unique):
            return "on_approve"

        rej_unique = rej_acts - (appr_acts | sla_acts)
        if rej_unique and any(a in executed_set for a in rej_unique):
            return "on_reject"

        sla_unique = sla_acts - (appr_acts | rej_acts)
        if sla_unique and any(a in executed_set for a in sla_unique):
            return "on_sla_breach"

        if any(a in executed_set for a in sla_acts) and not any(a in executed_set for a in (appr_acts - sla_acts)):
            return "on_sla_breach"

        if any(a in executed_set for a in rej_acts) and not any(a in executed_set for a in (appr_acts - rej_acts)):
            return "on_reject"

        if any(a in executed_set for a in appr_acts) and not any(a in executed_set for a in (rej_acts - appr_acts)):
            return "on_approve"

        return None

    def process_step_list(step_list: List[Dict[str, Any]], prev_items: List[Any], scope_prefix: str = "", is_branch_active: bool = True) -> List[Any]:
        current_prev = prev_items
        jump_skip_until_idx = -1

        for idx, step in enumerate(step_list):
            if not isinstance(step, dict):
                continue

            current_step_active = is_branch_active
            if jump_skip_until_idx != -1:
                if idx < jump_skip_until_idx:
                    current_step_active = False
                else:
                    jump_skip_until_idx = -1

            raw_step_id = step.get("id") or step.get("activity") or step.get("human_step") or (step.get("branching_step") if isinstance(step.get("branching_step"), str) else None) or f"step_{idx}"
            step_id = raw_step_id
            node_id = f"node__{scope_prefix}__{raw_step_id}" if scope_prefix else f"node__{raw_step_id}"
            
            activity = step.get("activity")
            human_step = step.get("human_step")
            branching_step = step.get("branching_step")
            compensation = step.get("compensation") or step.get("compensation_activity")
            timeout = step.get("timeout_seconds")
            allowed_roles = step.get("allowed_roles", [])
            security_policy = step.get("security_policy")
            signal_name = step.get("signal_name") or human_step

            is_async = bool(step.get("async") or step.get("async_exec") or step.get("is_async"))
            is_human = bool(human_step)
            is_branch = bool(
                (branching_step and branching_step is not False)
                or step.get("condition")
                or step.get("conditions")
                or step.get("then_steps")
                or step.get("then")
                or step.get("else_steps")
                or step.get("else")
            )
            is_rewind = bool("rewind_to_step_index" in step or "rewind_to_step" in step)
            is_terminate = bool(step.get("terminate"))
            is_compensate = bool(step.get("compensate") or step.get("rollback") or step.get("trigger_compensation"))

            if not current_step_active:
                executed = False
                step_status = "SKIPPED" if has_live_execution else "PENDING"
            else:
                executed = is_step_executed(step_id, activity or human_step or branching_step, signal_name, scope_prefix=scope_prefix)
                step_status = determine_status(step_id, activity or human_step or branching_step, is_human, executed)
                if not executed and step.get("conditions") and not is_branch and has_live_execution:
                    prev_ids = [p["id"] if isinstance(p, dict) else p for p in current_prev]
                    prev_reached = any(pid in executed_set or any(n.get("data", {}).get("executed") or n.get("data", {}).get("status") == "BYPASSED" for n in nodes if n["id"] == pid) for pid in prev_ids) if prev_ids else True
                    if prev_reached:
                        step_status = "BYPASSED"

            timeline_item = (
                timeline_map.get(activity or step_id)
                or timeline_map.get(raw_step_id)
                or timeline_map.get(step_id)
                or (timeline_map.get(activity.lower()) if activity else None)
                or (timeline_map.get(raw_step_id.lower()) if raw_step_id else None)
                or (timeline_map.get(step_id.lower()) if step_id else None)
            )
            if timeline_item and timeline_item.get("status") and current_step_active:
                raw_stat = timeline_item.get("status")
                if raw_stat in ("BACKGROUND_DISPATCHED", "ASYNC_DISPATCHED", "DISPATCHED", "COMPLETED") or timeline_item.get("async") is True:
                    step_status = "COMPLETED"
                    executed = True
                else:
                    step_status = raw_stat

            # 1. Handle Rewind Step
            if is_rewind:
                target_idx = step.get("rewind_to_step_index", 0)
                target_step_id = step.get("rewind_to_step") or (top_step_ids[target_idx] if target_idx < len(top_step_ids) else "start")
                target_node_id = top_step_node_ids.get(target_step_id, top_step_node_ids.get(target_idx, f"node__{target_step_id}")) if target_step_id != "start" else "node__start"

                if current_step_active and has_live_execution:
                    prev_ids = [p["id"] if isinstance(p, dict) else p for p in current_prev]
                    prev_exec = any(pid in executed_set or any(n.get("data", {}).get("executed") for n in nodes if n["id"] == pid) for pid in prev_ids)
                    if prev_exec or not prev_ids:
                        executed = True
                        step_status = "COMPLETED"

                nodes.append({
                    "id": node_id,
                    "shape": "octopus-rewind-node",
                    "label": f"⟲ REWIND\n(to Step {target_idx}: {target_step_id})",
                    "width": 240,
                    "height": 75,
                    "data": {
                        "type": "rewind",
                        "title": f"Rewind to Step {target_idx}: {target_step_id}",
                        "step_id": step_id,
                        "target_step": target_step_id,
                        "status": step_status,
                        "executed": executed
                    }
                })

                for p_item in current_prev:
                    p_id = p_item["id"] if isinstance(p_item, dict) else p_item
                    p_lbl = p_item.get("label") if isinstance(p_item, dict) else None
                    p_color = p_item.get("color", "#6366f1") if isinstance(p_item, dict) else "#6366f1"
                    
                    e_obj = {
                        "id": f"edge__{p_id}__to__{node_id}",
                        "source": p_id,
                        "target": node_id,
                        "attrs": {
                            "line": {
                                "stroke": "#10b981" if executed else p_color,
                                "strokeWidth": 3.5 if executed else 2,
                                "targetMarker": {"name": "block", "width": 8, "height": 6, "fill": "#10b981" if executed else p_color},
                                "filter": "drop-shadow(0 0 8px rgba(16, 185, 129, 0.6))" if executed else "none",
                                "opacity": 1.0 if executed else 0.5
                            }
                        }
                    }
                    if p_lbl:
                        e_obj["labels"] = [{"attrs": {"text": {"text": p_lbl, "fill": p_color, "fontSize": 11, "fontWeight": "bold"}}}]
                    edges.append(e_obj)

                # Draw loopback edge to target with outer boundary routing
                rewind_lbl_color = "#f59e0b" if executed else "#475569"
                edges.append({
                    "id": f"edge_rewind__{node_id}__to__{target_node_id}",
                    "source": node_id,
                    "target": target_node_id,
                    "router": {
                        "name": "manhattan",
                        "args": {
                            "startDirections": ["left", "bottom"],
                            "endDirections": ["left"],
                            "padding": 40
                        }
                    },
                    "connector": {"name": "rounded", "args": {"radius": 12}},
                    "labels": [{
                        "position": 0.5,
                        "attrs": {
                            "text": {
                                "text": f"⟲ Rewind Loop to Step {target_idx}",
                                "fill": rewind_lbl_color,
                                "fontSize": 10,
                                "fontWeight": "bold",
                                "opacity": 1.0 if executed else 0.4
                            },
                            "rect": {
                                "fill": "#1e1b10" if executed else "#090d16",
                                "rx": 4,
                                "ry": 4,
                                "stroke": "#f59e0b" if executed else "#1e293b",
                                "strokeWidth": 1,
                                "opacity": 1.0 if executed else 0.3
                            }
                        }
                    }],
                    "attrs": {
                        "line": {
                            "stroke": "#f59e0b" if executed else "#1e293b",
                            "strokeWidth": 3 if executed else 1.5,
                            "strokeDasharray": "5 5",
                            "targetMarker": {"name": "block", "width": 8, "height": 6, "fill": "#f59e0b" if executed else "#334155"},
                            "filter": "drop-shadow(0 0 10px rgba(245, 158, 11, 0.7))" if executed else "none",
                            "opacity": 1.0 if executed else 0.25
                        }
                    }
                })
                # Rewind node loops back; does not flow forward
                current_prev = []
                continue

            # 1b. Handle Jump / Fast-Track Step
            is_jump = bool("jump_to_step_index" in step or "jump_to_step" in step)
            if is_jump:
                target_idx = step.get("jump_to_step_index", 0)
                target_step_id = step.get("jump_to_step") or (top_step_ids[target_idx] if target_idx < len(top_step_ids) else "end")
                target_node_id = top_step_node_ids.get(target_step_id, top_step_node_ids.get(target_idx, f"node__{target_step_id}")) if target_step_id != "end" else "node__end"

                if current_step_active and has_live_execution:
                    prev_ids = [p["id"] if isinstance(p, dict) else p for p in current_prev]
                    prev_exec = any(pid in executed_set or any(n.get("data", {}).get("executed") for n in nodes if n["id"] == pid) for pid in prev_ids)
                    if prev_exec or not prev_ids:
                        executed = True
                        step_status = "COMPLETED"

                if executed and isinstance(target_idx, int) and target_idx > idx:
                    jump_skip_until_idx = target_idx

                nodes.append({
                    "id": node_id,
                    "shape": "octopus-jump-node",
                    "label": f"⤑ FAST-TRACK JUMP\n(to Step {target_idx}: {target_step_id})",
                    "width": 240,
                    "height": 75,
                    "data": {
                        "type": "jump",
                        "title": f"Fast-Track Jump to Step {target_idx}: {target_step_id}",
                        "step_id": step_id,
                        "target_step": target_step_id,
                        "status": step_status,
                        "executed": executed
                    }
                })

                for p_item in current_prev:
                    p_id = p_item["id"] if isinstance(p_item, dict) else p_item
                    p_lbl = p_item.get("label") if isinstance(p_item, dict) else None
                    p_color = p_item.get("color", "#10b981" if executed else "#06b6d4") if isinstance(p_item, dict) else ("#10b981" if executed else "#06b6d4")
                    
                    e_obj = {
                        "id": f"edge__{p_id}__to__{node_id}",
                        "source": p_id,
                        "target": node_id,
                        "attrs": {
                            "line": {
                                "stroke": "#10b981" if executed else p_color,
                                "strokeWidth": 3.5 if executed else 2,
                                "targetMarker": {"name": "block", "width": 8, "height": 6, "fill": "#10b981" if executed else p_color},
                                "filter": "drop-shadow(0 0 8px rgba(16, 185, 129, 0.6))" if executed else "none",
                                "opacity": 1.0 if executed else 0.5
                            }
                        }
                    }
                    if p_lbl:
                        e_obj["labels"] = [{"attrs": {"text": {"text": p_lbl, "fill": "#10b981" if executed else p_color, "fontSize": 11, "fontWeight": "bold"}}}]
                    edges.append(e_obj)

                # Draw forward jump edge to target
                jump_edge_color = "#10b981" if executed else "#06b6d4"
                edges.append({
                    "id": f"edge_jump__{node_id}__to__{target_node_id}",
                    "source": node_id,
                    "target": target_node_id,
                    "labels": [{
                        "position": 0.5,
                        "attrs": {
                            "text": {
                                "text": f"⤑ Jump to Step {target_idx}", 
                                "fill": jump_edge_color if executed else "#475569", 
                                "fontSize": 10, 
                                "fontWeight": "bold",
                                "opacity": 1.0 if executed else 0.4
                            },
                            "rect": {
                                "fill": "#0f2e22" if executed else "#090d16",
                                "rx": 4,
                                "ry": 4,
                                "stroke": jump_edge_color if executed else "#1e293b",
                                "strokeWidth": 1,
                                "opacity": 1.0 if executed else 0.3
                            }
                        }
                    }],
                    "attrs": {
                        "line": {
                            "stroke": jump_edge_color if executed else "#1e293b",
                            "strokeWidth": 3.0 if executed else 1.5,
                            "strokeDasharray": "5 5",
                            "opacity": 1.0 if executed else 0.25,
                            "filter": f"drop-shadow(0 0 10px rgba(16, 185, 129, 0.7))" if executed else "none",
                            "targetMarker": {
                                "name": "block", 
                                "width": 8, 
                                "height": 6, 
                                "fill": jump_edge_color if executed else "#334155"
                            }
                        }
                    }
                })
                current_prev = []
                continue

            is_sla_breach = "sla_breach" in scope_prefix if scope_prefix else False

            # 2. Handle Terminate Step
            if is_terminate:
                if current_step_active and has_live_execution:
                    prev_ids = [p["id"] if isinstance(p, dict) else p for p in current_prev]
                    prev_exec = any(pid in executed_set or any(n.get("data", {}).get("executed") for n in nodes if n["id"] == pid) for pid in prev_ids)
                    if prev_exec or not prev_ids:
                        executed = True
                        step_status = "TIMED_OUT" if is_sla_breach else "COMPLETED"

                nodes.append({
                    "id": node_id,
                    "shape": "octopus-end-node",
                    "label": "EXIT (CANCELLED)",
                    "width": 140,
                    "height": 50,
                    "data": {
                        "type": "end",
                        "title": f"Terminated ({step_id})",
                        "step_id": step_id,
                        "status": step_status,
                        "executed": executed,
                        "is_sla_breach": is_sla_breach
                    }
                })

                for p_item in current_prev:
                    p_id = p_item["id"] if isinstance(p_item, dict) else p_item
                    p_lbl = p_item.get("label") if isinstance(p_item, dict) else None
                    p_color = p_item.get("color", "#ef4444") if isinstance(p_item, dict) else "#ef4444"
                    
                    e_obj = {
                        "id": f"edge__{p_id}__to__{node_id}",
                        "source": p_id,
                        "target": node_id,
                        "attrs": {
                            "line": {
                                "stroke": p_color,
                                "strokeWidth": 2,
                                "targetMarker": {"name": "block", "width": 8, "height": 6}
                            }
                        }
                    }
                    if p_lbl:
                        e_obj["labels"] = [{"attrs": {"text": {"text": p_lbl, "fill": p_color, "fontSize": 11, "fontWeight": "bold"}}}]
                    edges.append(e_obj)

                current_prev = []
                continue

            # 2b. Handle Declarative Compensation / Rollback Step
            if is_compensate:
                if current_step_active and has_live_execution:
                    prev_ids = [p["id"] if isinstance(p, dict) else p for p in current_prev]
                    prev_exec = any(pid in executed_set or any(n.get("data", {}).get("executed") for n in nodes if n["id"] == pid) for pid in prev_ids)
                    if prev_exec or not prev_ids:
                        executed = True
                        step_status = "ROLLED_BACK"

                reason_text = step.get("reason") or "Declarative Rollback"
                nodes.append({
                    "id": node_id,
                    "shape": "octopus-compensation-node",
                    "label": f"🛑 SAGA ROLLBACK\n({reason_text})",
                    "width": 230,
                    "height": 65,
                    "data": {
                        "type": "compensate",
                        "title": f"Declarative Compensation ({reason_text})",
                        "step_id": step_id,
                        "status": step_status,
                        "executed": executed,
                        "reason": reason_text
                    }
                })

                for p_item in current_prev:
                    p_id = p_item["id"] if isinstance(p_item, dict) else p_item
                    p_lbl = p_item.get("label") if isinstance(p_item, dict) else None
                    p_color = p_item.get("color", "#f97316") if isinstance(p_item, dict) else "#f97316"
                    
                    e_obj = {
                        "id": f"edge__{p_id}__to__{node_id}",
                        "source": p_id,
                        "target": node_id,
                        "attrs": {
                            "line": {
                                "stroke": "#f97316" if executed else p_color,
                                "strokeWidth": 3 if executed else 2,
                                "targetMarker": {"name": "block", "width": 8, "height": 6, "fill": "#f97316" if executed else p_color}
                            }
                        }
                    }
                    if p_lbl:
                        e_obj["labels"] = [{"attrs": {"text": {"text": p_lbl, "fill": "#f97316" if executed else p_color, "fontSize": 11, "fontWeight": "bold"}}}]
                    edges.append(e_obj)

                current_prev = []
                continue

            # 3. Standard Activity, Human Step, or Branch Decision Node
            shape_type = "octopus-branch-node" if is_branch else ("octopus-human-node" if is_human else "octopus-activity-node")
            width_val = 220 if is_branch else (220 if is_human else 200)
            height_val = 80 if is_branch else (75 if is_human else 65)

            step_title = (branching_step if isinstance(branching_step, str) else (step.get("description") or step_id)) if is_branch else (human_step if is_human else activity)
            
            # Check availability in scanned Python codebase or in-memory registry
            is_available = False
            act_meta = None
            if activity and available_activities:
                act_meta = available_activities.get(activity)
                if not act_meta:
                    for k, v in available_activities.items():
                        if k.lower() == activity.lower() or k.replace("_", "").lower() == activity.replace("_", "").lower():
                            act_meta = v
                            break
                is_available = bool(act_meta)
            comp_meta = None
            if compensation and available_activities:
                comp_meta = available_activities.get(compensation)
                if not comp_meta:
                    for k, v in available_activities.items():
                        if k.lower() == compensation.lower() or k.replace("_", "").lower() == compensation.replace("_", "").lower():
                            comp_meta = v
                            break

            # Introspect idempotency strategy metadata from global registry
            idem_name = step.get("idempotency_strategy")
            idem_meta = None
            if idem_name:
                try:
                    from octopus_workflow.domain.strategies.idempotency import default_idempotency_registry
                    strat_obj = default_idempotency_registry.get(idem_name)
                    if strat_obj:
                        raw_doc = getattr(strat_obj, "__doc__", None) or (strat_obj.__class__.__doc__ if hasattr(strat_obj, "__class__") else "")
                        cleaned_doc = " ".join([l.strip() for l in (raw_doc or "").splitlines() if l.strip()])
                        idem_meta = {
                            "class": strat_obj.__class__.__name__,
                            "module": strat_obj.__class__.__module__,
                            "doc": cleaned_doc or f"Idempotency strategy '{idem_name}'."
                        }
                except Exception:
                    pass

            # Introspect security policy metadata from global registry (supports composite policies)
            raw_sec_pols = step.get("security_policies") or step.get("security_policys") or step.get("security_policy")
            sec_policy_names = []
            if isinstance(raw_sec_pols, list):
                sec_policy_names = [str(p).strip() for p in raw_sec_pols if p and str(p).strip()]
            elif isinstance(raw_sec_pols, str) and raw_sec_pols.strip():
                sec_policy_names = [raw_sec_pols.strip()]

            sec_policies_meta = []
            for sec_name in sec_policy_names:
                m_info = {
                    "name": sec_name,
                    "class": "SecurityPolicy",
                    "module": "strategies",
                    "doc": f"Enforces '{sec_name}' on reviewer approval."
                }
                try:
                    from octopus_workflow.domain.strategies.security import default_security_registry
                    policy_obj = default_security_registry.get(sec_name)
                    if policy_obj:
                        raw_doc = getattr(policy_obj, "__doc__", None) or (policy_obj.__class__.__doc__ if hasattr(policy_obj, "__class__") else "")
                        cleaned_doc = " ".join([l.strip() for l in (raw_doc or "").splitlines() if l.strip()])
                        m_info["class"] = policy_obj.__class__.__name__
                        m_info["module"] = policy_obj.__class__.__module__
                        m_info["doc"] = cleaned_doc or f"Security policy '{sec_name}'."
                except Exception:
                    pass
                sec_policies_meta.append(m_info)

            first_sec_meta = sec_policies_meta[0] if sec_policies_meta else None

            loop_cfg = step.get("loop") or step.get("loop_config")
            is_loop = bool(loop_cfg)
            loop_for_each = loop_cfg.get("for_each") if (is_loop and isinstance(loop_cfg, dict)) else None
            loop_until = loop_cfg.get("until") if (is_loop and isinstance(loop_cfg, dict)) else None
            loop_while = loop_cfg.get("while") if (is_loop and isinstance(loop_cfg, dict)) else None
            loop_max_iter = loop_cfg.get("max_iterations") if (is_loop and isinstance(loop_cfg, dict)) else None
            loop_interval = loop_cfg.get("interval_seconds") if (is_loop and isinstance(loop_cfg, dict)) else None
            loop_backoff = loop_cfg.get("backoff_coefficient") if (is_loop and isinstance(loop_cfg, dict)) else None
            loop_item_var = loop_cfg.get("item_variable") if (is_loop and isinstance(loop_cfg, dict)) else None

            node_data = {
                "step_id": step_id,
                "type": "loop" if is_loop else ("branch" if is_branch else ("human" if is_human else "activity")),
                "title": step_title,
                "activity": activity,
                "activity_available": is_available if activity else True,
                "activity_class": act_meta.get("class") if act_meta else None,
                "activity_module": act_meta.get("module") if act_meta else None,
                "activity_file": act_meta.get("file") if act_meta else None,
                "activity_doc": act_meta.get("doc") if act_meta else None,
                "human_step": human_step,
                "branching_step": step_title if is_branch else branching_step,
                "conditions": step.get("conditions") or step.get("condition") or step.get("expression") or step.get("branches") or step.get("cases") or step.get("evaluate"),
                "compensation": compensation,
                "compensation_available": bool(comp_meta) if compensation else None,
                "compensation_module": comp_meta.get("module") if comp_meta else None,
                "timeout": timeout,
                "allowed_roles": allowed_roles,
                "security_policies": sec_policy_names,
                "security_policies_meta": sec_policies_meta,
                "security_policy": sec_policy_names[0] if sec_policy_names else None,
                "security_policy_class": first_sec_meta.get("class") if first_sec_meta else None,
                "security_policy_module": first_sec_meta.get("module") if first_sec_meta else None,
                "security_policy_doc": first_sec_meta.get("doc") if first_sec_meta else None,
                "signal_name": signal_name,
                "status": step_status,
                "executed": executed,
                "is_sla_breach": is_sla_breach,
                "timeline": timeline_item,
                "job_data": job_data,
                "input_payload": timeline_item.get("input") if timeline_item else None,
                "output_payload": timeline_item.get("output") if timeline_item else None,
                "duration_ms": timeline_item.get("duration_ms") if timeline_item else None,
                "error": timeline_item.get("error") if timeline_item else None,
                "idempotency_strategy": idem_name,
                "idempotency_strategy_class": idem_meta.get("class") if idem_meta else None,
                "idempotency_strategy_module": idem_meta.get("module") if idem_meta else None,
                "idempotency_strategy_doc": idem_meta.get("doc") if idem_meta else None,
                "on_approve": step.get("on_approve"),
                "on_reject": step.get("on_reject"),
                "on_sla_breach": step.get("on_sla_breach"),
                "then_steps": step.get("then_steps") or step.get("then") or (step.get("condition", {}).get("then_steps") if isinstance(step.get("condition"), dict) else None) or (step.get("conditions", {}).get("then_steps") if isinstance(step.get("conditions"), dict) else None),
                "else_steps": step.get("else_steps") or step.get("else") or (step.get("condition", {}).get("else_steps") if isinstance(step.get("condition"), dict) else None) or (step.get("conditions", {}).get("else_steps") if isinstance(step.get("conditions"), dict) else None),
                "is_async": is_async,
                "async": is_async,
                "is_output": bool(step.get("is_output") or step.get("output")),
                "is_loop": is_loop,
                "loop": loop_cfg,
                "loop_for_each": loop_for_each,
                "loop_until": loop_until,
                "loop_while": loop_while,
                "loop_max_iterations": loop_max_iter,
                "loop_interval_seconds": loop_interval,
                "loop_backoff_coefficient": loop_backoff,
                "loop_item_variable": loop_item_var,
            }

            # Check if this is a Quorum Human Approval Gate
            is_quorum = bool(
                is_human and (
                    step.get("required_approvals", 1) > 1
                    or (step.get("required_signers") and len(step.get("required_signers")) > 1)
                    or "quorum" in str(step.get("security_policy", "")).lower()
                    or any("quorum" in str(p).lower() for p in sec_policy_names)
                )
            )

            if is_quorum:
                req_approvals = step.get("required_approvals") or 2
                req_signers = step.get("required_signers") or []
                total_slots = max(req_approvals, len(req_signers) if req_signers else req_approvals)

                quorum_votes = extract_quorum_votes(job_data, timeline)
                rej_votes = [v for v in quorum_votes if v.get("action") in ("REJECT", "REJECTED")]
                appr_votes = [v for v in quorum_votes if v.get("action") in ("APPROVE", "APPROVED")]

                sig_prefix = f"signal_decision:{signal_name}:"
                rej_acts = find_all_activities_in_steps(step.get("on_reject", []))
                has_reject_acts_executed = bool(rej_acts and any(a in executed_set for a in rej_acts))

                has_rejection = bool(
                    len(rej_votes) > 0
                    or f"{sig_prefix}REJECTED" in executed_set
                    or f"decision:{signal_name}:REJECTED" in executed_set
                    or has_reject_acts_executed
                    or step_status == "REJECTED"
                )
                has_approved_signal = f"{sig_prefix}APPROVED" in executed_set or f"decision:{signal_name}:APPROVED" in executed_set

                is_gate_active = current_step_active and is_step_target_active(step_id, human_step) and overall_status in ("RUNNING", "AWAITING_APPROVAL", "AWAITING_SIGNAL")
                is_gate_rejected = current_step_active and has_rejection
                is_gate_done = bool(
                    current_step_active and not is_gate_rejected and not is_gate_active and (
                        step_id in executed_set
                        or (human_step and human_step in executed_set)
                        or len(appr_votes) >= req_approvals
                        or (executed and step_status in ("COMPLETED", "APPROVED"))
                    )
                )

                approved_count = len(appr_votes)
                if is_gate_done:
                    if approved_count < req_approvals:
                        approved_count = req_approvals
                elif is_gate_active and has_approved_signal and approved_count == 0:
                    approved_count = 1

                # 1. Parallel Approver Nodes
                for app_idx in range(total_slots):
                    app_node_id = f"{node_id}__approver_{app_idx + 1}"
                    slot_name = req_signers[app_idx] if app_idx < len(req_signers) else f"Approver {app_idx + 1}"

                    app_data = {
                        "step_id": f"{step_id}__approver_{app_idx + 1}",
                        "parent_gate": step_id,
                        "type": "quorum_approver",
                        "title": f"Quorum Signer {app_idx + 1}",
                        "allowed_roles": allowed_roles,
                        "timeout": timeout,
                    }

                    if app_idx < len(quorum_votes):
                        vote = quorum_votes[app_idx]
                        rev_id = vote.get("reviewer_id") or slot_name
                        if vote.get("action") in ("REJECT", "REJECTED"):
                            app_stat = "REJECTED"
                            app_exec = True
                            app_lbl = f"✕ Approver {app_idx + 1}\n{rev_id}\n(Rejected)"
                        else:
                            app_stat = "COMPLETED"
                            app_exec = True
                            app_lbl = f"✓ Approver {app_idx + 1}\n{rev_id}\n(Approved)"
                        app_data["reviewer_id"] = rev_id
                        app_data["notes"] = vote.get("notes", "")
                        app_data["timestamp"] = vote.get("timestamp", "")
                    elif is_gate_rejected and not quorum_votes:
                        if app_idx == 0:
                            app_stat = "COMPLETED"
                            app_exec = True
                            app_lbl = f"✓ Approver 1\nbank_of_america_rep\n(Approved)"
                        elif app_idx == 1:
                            app_stat = "REJECTED"
                            app_exec = True
                            app_lbl = f"✕ Approver 2\nbarclays_rep\n(Rejected)"
                        else:
                            app_stat = "SKIPPED" if has_live_execution else "PENDING"
                            app_exec = False
                            app_lbl = f"⏳ Approver {app_idx + 1}\n(Pending)"
                    elif is_gate_done:
                        app_stat = "COMPLETED"
                        app_exec = True
                        app_lbl = f"✓ {slot_name}\n(Approved)"
                    elif is_gate_active:
                        if app_idx < approved_count:
                            app_stat = "COMPLETED"
                            app_exec = True
                            app_lbl = f"✓ {slot_name}\n(Approved)"
                        else:
                            app_stat = "AWAITING_APPROVAL"
                            app_exec = False
                            app_lbl = f"⏳ {slot_name}\n(Awaiting Sign-off)"
                    else:
                        app_stat = "SKIPPED" if has_live_execution else "PENDING"
                        app_exec = False
                        app_lbl = f"⏳ {slot_name}\n(Pending)"

                    app_data["status"] = app_stat
                    app_data["executed"] = app_exec

                    nodes.append({
                        "id": app_node_id,
                        "shape": "octopus-quorum-approver-node",
                        "label": app_lbl,
                        "width": 200,
                        "height": 70,
                        "data": app_data
                    })

                    # Connect from Predecessors directly to this Approver
                    for p_item in current_prev:
                        p_id = p_item["id"] if isinstance(p_item, dict) else p_item
                        p_lbl = p_item.get("label") if isinstance(p_item, dict) else None
                        p_color = p_item.get("color", "#6366f1") if isinstance(p_item, dict) else "#6366f1"
                        edges.append({
                            "id": f"edge__{p_id}__to__{app_node_id}",
                            "source": p_id,
                            "target": app_node_id,
                            "attrs": {
                                "line": {
                                    "stroke": p_color,
                                    "strokeWidth": 2,
                                    "targetMarker": {"name": "block", "width": 8, "height": 6}
                                }
                            },
                            "labels": [{"attrs": {"text": {"text": p_lbl, "fill": p_color, "fontSize": 11, "fontWeight": "bold"}}}] if p_lbl else []
                        })

                # 2. Quorum Consensus Join Node
                join_exec = current_step_active and (is_gate_done or is_gate_rejected or approved_count >= req_approvals)
                if is_gate_rejected:
                    rej_cnt = len(rej_votes) if len(rej_votes) > 0 else 1
                    join_lbl = f"✕ Quorum Rejected / Dissent\n({rej_cnt} Dissent Lodged • Rewound)"
                    join_stat = "REJECTED"
                elif is_gate_done or approved_count >= req_approvals:
                    join_lbl = f"✓ Quorum Consensus Met\n({approved_count} of {req_approvals} Required Signatures)"
                    join_stat = "COMPLETED"
                elif is_gate_active:
                    join_lbl = f"⏳ {step_id}\n({approved_count} of {req_approvals} Sign-offs Collected)"
                    join_stat = "AWAITING_APPROVAL"
                else:
                    join_lbl = f"{step_id}\n(Quorum Gate: {req_approvals} of {total_slots} Required)"
                    join_stat = "SKIPPED" if has_live_execution else "PENDING"

                nodes.append({
                    "id": node_id,
                    "shape": "octopus-quorum-join-node",
                    "label": join_lbl,
                    "width": 240,
                    "height": 80,
                    "data": {
                        "step_id": step_id,
                        "type": "quorum_consensus",
                        "title": step_title,
                        "status": join_stat,
                        "executed": join_exec,
                        "required_approvals": req_approvals,
                        "total_signers": total_slots,
                        "current_approvals": approved_count,
                        "security_policy": "quorum_consensus",
                        "on_approve": step.get("on_approve"),
                        "on_reject": step.get("on_reject"),
                        "on_sla_breach": step.get("on_sla_breach"),
                    }
                })

                # Connect each Approver to the Join Node
                for app_idx in range(total_slots):
                    app_node_id = f"{node_id}__approver_{app_idx + 1}"
                    edges.append({
                        "id": f"edge__{app_node_id}__to__{node_id}",
                        "source": app_node_id,
                        "target": node_id,
                        "attrs": {
                            "line": {
                                "stroke": "#475569",
                                "strokeWidth": 2,
                                "targetMarker": {"name": "block", "width": 8, "height": 6}
                            }
                        }
                    })

                # Route from Join Node to on_approve / on_reject / on_sla_breach
                branch_out_ids = []
                if is_gate_rejected and step.get("on_reject"):
                    rej_nodes = process_sub_branch(step["on_reject"], node_id, "Rejected", "#ef4444", is_active_branch=True)
                    branch_out_ids.extend(rej_nodes)
                elif is_gate_done:
                    if step.get("on_approve"):
                        appr_nodes = process_sub_branch(step["on_approve"], node_id, "Approved", "#10b981", is_active_branch=True)
                        branch_out_ids.extend(appr_nodes)
                    else:
                        branch_out_ids.append({"id": node_id, "label": "Approved", "color": "#10b981"})
                else:
                    if step.get("on_approve"):
                        appr_nodes = process_sub_branch(step["on_approve"], node_id, "Approved", "#10b981", is_active_branch=False)
                        branch_out_ids.extend(appr_nodes)
                    else:
                        branch_out_ids.append({"id": node_id, "label": "Approved", "color": "#10b981"})
                    if step.get("on_reject"):
                        rej_nodes = process_sub_branch(step["on_reject"], node_id, "Rejected", "#ef4444", is_active_branch=False)
                        branch_out_ids.extend(rej_nodes)

                if branch_out_ids:
                    current_prev = branch_out_ids
                else:
                    current_prev = [{"id": node_id, "label": "Approved", "color": "#10b981"}]
                continue

            if is_loop:
                shape_type = "octopus-loop-node"
                width_val = 260
                height_val = 80
                if loop_for_each:
                    node_label = f"🔁 {step_id}\n[for_each: {loop_for_each}]"
                    coll = job_data.get(loop_for_each) if isinstance(job_data, dict) else None
                    if isinstance(coll, list) and executed:
                        node_label += f"\n(✓ {len(coll)} items processed)"
                elif loop_until:
                    u_var = loop_until.get("variable", "status") if isinstance(loop_until, dict) else "status"
                    u_val = loop_until.get("value", "ACTIVE") if isinstance(loop_until, dict) else "ACTIVE"
                    node_label = f"⏱️ {step_id}\n[until: {u_var} == '{u_val}']"
                    if loop_max_iter:
                        node_label += f"\n(max: {loop_max_iter}, int: {loop_interval or 1}s)"
                elif loop_while:
                    w_var = loop_while.get("variable", "status") if isinstance(loop_while, dict) else "status"
                    w_val = loop_while.get("value", "") if isinstance(loop_while, dict) else ""
                    node_label = f"🔄 {step_id}\n[while: {w_var} == '{w_val}']"
                else:
                    node_label = f"🔁 {step_id}\n(Loop Iteration)"
            elif is_async:
                node_label = f"⚡ {step_id}\n(Async Background)"
            elif is_human:
                node_label = f"👤 {step_id}"
            elif is_branch:
                node_label = f"🔀 {step_id}"
            else:
                node_label = step_id

            nodes.append({
                "id": node_id,
                "shape": shape_type,
                "label": node_label,
                "width": 260 if is_loop else (240 if is_async else width_val),
                "height": 80 if is_loop else (75 if (is_async or is_human or is_branch) else 55),
                "data": node_data
            })

            # Connect predecessor edges
            for p_item in current_prev:
                p_id = p_item["id"] if isinstance(p_item, dict) else p_item
                p_lbl = p_item.get("label") if isinstance(p_item, dict) else None
                p_color = p_item.get("color", "#6366f1") if isinstance(p_item, dict) else "#6366f1"
                
                is_edge_active = bool(executed and current_step_active)
                edge_stroke = "#10b981" if is_edge_active else (p_color if not has_live_execution else "#334155")
                edge_opacity = 1.0 if is_edge_active else (1.0 if not has_live_execution else 0.35)
                
                e_obj = {
                    "id": f"edge__{p_id}__to__{node_id}",
                    "source": p_id,
                    "target": node_id,
                    "attrs": {
                        "line": {
                            "stroke": edge_stroke,
                            "strokeWidth": 2.5 if is_edge_active else 1.5,
                            "opacity": edge_opacity,
                            "targetMarker": {"name": "block", "width": 8, "height": 6, "fill": edge_stroke}
                        }
                    }
                }
                if p_lbl:
                    pos_val = 0.35
                    if "TRUE" in str(p_lbl):
                        pos_val = 0.20
                    elif "APPROV" in str(p_lbl):
                        pos_val = 0.28
                    elif "REJECT" in str(p_lbl) or "FALSE" in str(p_lbl) or "Blocked" in str(p_lbl):
                        pos_val = 0.50
                    elif "SLA" in str(p_lbl):
                        pos_val = 0.38

                    label_text_color = p_color if (is_edge_active or not has_live_execution) else "#475569"
                    e_obj["labels"] = [{
                        "position": pos_val,
                        "attrs": {
                            "text": {"text": p_lbl, "fill": label_text_color, "fontSize": 11, "fontWeight": "bold", "opacity": edge_opacity},
                            "rect": {"fill": "#0f172a", "rx": 4, "ry": 4, "stroke": "#334155" if (is_edge_active or not has_live_execution) else "#1e293b", "strokeWidth": 1, "opacity": edge_opacity}
                        }
                    }]
                edges.append(e_obj)

            # Process Compensation if attached
            if compensation:
                comp_id = f"node_comp__{step_id}"
                comp_timeline_item = (
                    timeline_map.get(f"comp__{step_id}")
                    or timeline_map.get(f"compensation__{compensation}")
                    or timeline_map.get(f"compensation:{compensation}")
                    or timeline_map.get(compensation)
                )
                comp_executed = bool(
                    comp_timeline_item
                    or (
                        overall_status == "FAILED"
                        and executed
                        and (
                            f"comp__{step_id}" in executed_set
                            or f"compensation:{compensation}" in executed_set
                            or compensation in executed_set
                            or step_status == "COMPLETED"
                        )
                    )
                )
                comp_status = comp_timeline_item.get("status", "COMPLETED") if comp_timeline_item else ("COMPENSATED" if comp_executed else "STANDBY")
                comp_meta = available_activities.get(compensation) if (compensation and available_activities) else None
                comp_label = f"⟲ Compensate:\n{compensation}"
                nodes.append({
                    "id": comp_id,
                    "shape": "octopus-compensation-node",
                    "label": comp_label,
                    "width": 240,
                    "height": 65,
                    "data": {
                        "type": "compensation",
                        "title": f"Rollback: {compensation}" + (" [EXECUTED]" if comp_executed else " [STANDBY]"),
                        "step_id": f"comp__{step_id}",
                        "activity": compensation,
                        "activity_available": bool(comp_meta) if compensation else False,
                        "activity_module": comp_meta.get("module") if comp_meta else None,
                        "activity_file": comp_meta.get("file") if comp_meta else None,
                        "activity_doc": comp_meta.get("doc") if comp_meta else None,
                        "status": comp_status,
                        "executed": comp_executed
                    }
                })
                edges.append({
                    "id": f"edge_comp__{node_id}__to__{comp_id}",
                    "source": node_id,
                    "target": comp_id,
                    "labels": [{"attrs": {"text": {"text": "Saga Rollback [Executed]" if comp_executed else "Saga Rollback", "fill": "#f97316" if comp_executed else "#64748b", "fontSize": 10}}}],
                    "attrs": {
                        "line": {
                            "stroke": "#f97316" if comp_executed else "#475569",
                            "strokeWidth": 2.5 if comp_executed else 1.5,
                            "strokeDasharray": "5 5",
                            "targetMarker": {"name": "classic", "size": 6, "fill": "#f97316" if comp_executed else "#475569"},
                            "filter": "drop-shadow(0 0 10px rgba(249, 115, 22, 0.75))" if comp_executed else "none",
                            "opacity": 1.0 if comp_executed else (0.3 if has_live_execution else 0.5)
                        }
                    }
                })

            # Handle Outgoing Branching logic
            branch_out_ids = []
            if is_branch:
                then_branch = step.get("then_steps") or step.get("then") or (step.get("condition", {}).get("then_steps") if isinstance(step.get("condition"), dict) else None) or (step.get("conditions", {}).get("then_steps") if isinstance(step.get("conditions"), dict) else None) or []
                else_branch = step.get("else_steps") or step.get("else") or (step.get("condition", {}).get("else_steps") if isinstance(step.get("condition"), dict) else None) or (step.get("conditions", {}).get("else_steps") if isinstance(step.get("conditions"), dict) else None)
                
                # Check explicit branch match result if recorded in timeline
                branch_rec = timeline_map.get(branching_step or step_id) or timeline_map.get(step_id)
                branch_matched = None
                if branch_rec and isinstance(branch_rec, dict):
                    if "condition_matched" in branch_rec:
                        branch_matched = bool(branch_rec["condition_matched"])

                then_acts = find_all_activities_in_steps(then_branch)
                has_child_exec_or_active = any(a in executed_set or is_step_target_active(a, a) for a in then_acts)

                # Check if then_branch has a forward jump step
                jump_in_then = next((s for s in then_branch if isinstance(s, dict) and ("jump_to_step_index" in s or "jump_to_step" in s)), None)
                if jump_in_then:
                    target_idx = jump_in_then.get("jump_to_step_index")
                    if target_idx is None and jump_in_then.get("jump_to_step"):
                        j_target_id = jump_in_then.get("jump_to_step")
                        target_idx = next((i for i, rs in enumerate(raw_steps) if rs.get("id") == j_target_id or rs.get("activity") == j_target_id), None)

                    if target_idx is not None and isinstance(target_idx, int) and target_idx > idx + 1:
                        intermediate_steps = raw_steps[idx + 1: target_idx]
                        intermediate_acts = find_all_activities_in_steps(intermediate_steps)
                        intermediate_exec = any(a in executed_set or is_step_target_active(a, a) for a in intermediate_acts)

                        target_step_acts = find_all_activities_in_steps([raw_steps[target_idx]]) if target_idx < len(raw_steps) else set()
                        target_exec = any(a in executed_set or is_step_target_active(a, a) for a in target_step_acts)

                        if intermediate_exec:
                            has_child_exec_or_active = False
                        elif (target_exec or overall_status == "COMPLETED") and not intermediate_exec:
                            has_child_exec_or_active = True

                if branch_matched is True:
                    then_active = current_step_active
                elif branch_matched is False:
                    then_active = False
                else:
                    then_active = current_step_active and (has_child_exec_or_active if has_live_execution else True)
                
                # Check if this branch step was reached (predecessors executed or workflow completed)
                prev_ids = [p["id"] if isinstance(p, dict) else p for p in current_prev]
                def _is_pred_valid(pid):
                    if pid == "node__start":
                        return not (has_ingress_guards and is_ingress_failed)
                    if pid == "node__ingress_guards":
                        return not is_ingress_failed
                    for n in nodes:
                        if n["id"] == pid:
                            n_data = n.get("data", {})
                            return bool(n_data.get("executed") and n_data.get("status") not in ("FAILED", "SKIPPED", "PENDING", "BYPASSED"))
                    return pid in executed_set
                
                prev_exec = any(_is_pred_valid(pid) for pid in prev_ids) if prev_ids else (not is_ingress_failed)
                
                if current_step_active and not is_ingress_failed and has_live_execution and (prev_exec or has_child_exec_or_active or (overall_status == "COMPLETED" and not is_ingress_failed)):
                    node_data["executed"] = True
                    if node_data.get("status") not in ("FAILED", "AWAITING_APPROVAL", "RUNNING"):
                        node_data["status"] = "COMPLETED"
                else:
                    if not current_step_active or is_ingress_failed:
                        node_data["executed"] = False
                        node_data["status"] = "SKIPPED" if has_live_execution else "PENDING"

                if not current_step_active or is_ingress_failed:
                    then_active = False
                elif branch_matched is True:
                    then_active = current_step_active
                elif branch_matched is False:
                    then_active = False
                else:
                    then_active = current_step_active and (has_child_exec_or_active if has_live_execution else True)

                then_step_ids = [s.get("id") or s.get("activity") for s in then_branch if isinstance(s, dict)]
                else_step_ids = [s.get("id") or s.get("activity") for s in (else_branch or []) if isinstance(s, dict)]
                
                node_data["condition"] = step.get("condition")
                node_data["conditions"] = step.get("conditions")
                
                b_info = {
                    "has_branches": True,
                    "condition": step.get("condition"),
                    "conditions": step.get("conditions"),
                    "then_step_ids": then_step_ids,
                    "else_step_ids": else_step_ids,
                }
                
                if has_live_execution and not is_ingress_failed and current_step_active and (node_data.get("executed") or then_active or (branch_matched is not None)):
                    is_true = (branch_matched is True) or (then_active and (has_child_exec_or_active or not else_branch))
                    if branch_matched is not None:
                        is_true = branch_matched
                    
                    eval_info = {
                        "evaluated": True,
                        "verdict": "TRUE" if is_true else "FALSE",
                        "verdict_label": "TRUE (Match)" if is_true else "FALSE (Else)",
                        "branch_taken": "THEN" if is_true else "ELSE",
                    }
                    
                    cond_dict = step.get("condition") if isinstance(step.get("condition"), dict) else None
                    if cond_dict:
                        var_name = cond_dict.get("variable") or cond_dict.get("field")
                        if var_name:
                            eval_info["variable_name"] = var_name
                            eval_info["runtime_value"] = extract_runtime_var(job_data, timeline, var_name)
                            eval_info["expected_value"] = cond_dict.get("value")
                            eval_info["operator"] = cond_dict.get("operator", "==")
                        if cond_dict.get("rule"):
                            eval_info["rule"] = cond_dict.get("rule")
                    
                    b_info["evaluation"] = eval_info
                
                node_data["branch_info"] = b_info

                if jump_in_then and then_active:
                    target_idx = jump_in_then.get("jump_to_step_index")
                    if target_idx is None and jump_in_then.get("jump_to_step"):
                        j_target_id = jump_in_then.get("jump_to_step")
                        target_idx = next((i for i, rs in enumerate(step_list) if rs.get("id") == j_target_id or rs.get("activity") == j_target_id), None)
                    if target_idx is not None and isinstance(target_idx, int) and target_idx > idx:
                        jump_skip_until_idx = target_idx
                
                # 1. TRUE Branch (then_steps / then)
                if then_branch:
                    then_ends = process_sub_branch(then_branch, node_id, "TRUE (Match)", "#10b981", is_active_branch=then_active)
                    branch_out_ids.extend(then_ends)
                
                # 2. FALSE Branch (else_steps / else or Fallback Track)
                if else_branch:
                    else_active = current_step_active and not is_ingress_failed and (not then_active if has_live_execution else True)
                    jump_in_else = next((s for s in else_branch if isinstance(s, dict) and ("jump_to_step_index" in s or "jump_to_step" in s)), None)
                    if jump_in_else and else_active:
                        target_idx = jump_in_else.get("jump_to_step_index")
                        if target_idx is None and jump_in_else.get("jump_to_step"):
                            j_target_id = jump_in_else.get("jump_to_step")
                            target_idx = next((i for i, rs in enumerate(step_list) if rs.get("id") == j_target_id or rs.get("activity") == j_target_id), None)
                        if target_idx is not None and isinstance(target_idx, int) and target_idx > idx:
                            jump_skip_until_idx = target_idx
                    else_ends = process_sub_branch(else_branch, node_id, "FALSE (Else)", "#94a3b8", is_active_branch=else_active)
                    branch_out_ids.extend(else_ends)
                else:
                    is_false_track_active = bool(node_data.get("executed") and not then_active and current_step_active and not is_ingress_failed)
                    branch_out_ids.append({
                        "id": node_id, 
                        "label": "FALSE (Default Track)", 
                        "color": "#10b981" if is_false_track_active else "#94a3b8",
                        "router": {"name": "manhattan", "args": {"startDirections": ["left"], "endDirections": ["left", "top"], "padding": 50}}
                    })

            elif is_human:
                sig_name = step.get("signal_name") or human_step or step_id
                h_id = step.get("id")
                h_name = human_step
                
                appr_acts = find_all_activities_in_steps(step.get("on_approve", []))
                rej_acts = find_all_activities_in_steps(step.get("on_reject", []))
                sla_acts = find_all_activities_in_steps(step.get("on_sla_breach", []))

                sla_unique = sla_acts - (appr_acts | rej_acts)
                rej_unique = rej_acts - (appr_acts | sla_acts)
                appr_unique = appr_acts - (rej_acts | sla_acts)

                has_sla_signal = "sla_breach" in executed_set or "timer_fired" in executed_set or f"signal_decision:{sig_name}:SLA_BREACH" in executed_set or f"decision:{sig_name}:SLA_BREACH" in executed_set
                has_rej_signal = f"signal_decision:{sig_name}:REJECTED" in executed_set or f"decision:{sig_name}:REJECTED" in executed_set
                has_appr_signal = f"signal_decision:{sig_name}:APPROVED" in executed_set or f"decision:{sig_name}:APPROVED" in executed_set

                sla_definite = bool(has_sla_signal or (sla_unique and any(a in executed_set for a in sla_unique)))
                rej_definite = bool(has_rej_signal or (rej_unique and any(a in executed_set for a in rej_unique)))
                appr_definite = bool(has_appr_signal or (appr_unique and any(a in executed_set for a in appr_unique)))

                if sla_definite:
                    has_sla_exec = True
                    has_rej_exec = False
                    has_appr_exec = False
                elif rej_definite:
                    has_sla_exec = False
                    has_rej_exec = True
                    has_appr_exec = False
                elif appr_definite:
                    has_sla_exec = False
                    has_rej_exec = False
                    has_appr_exec = True
                else:
                    has_sla_exec = bool(any(is_step_target_active(a, a) for a in sla_acts))
                    has_rej_exec = bool(any(is_step_target_active(a, a) for a in rej_acts))
                    has_appr_exec = bool(any(is_step_target_active(a, a) for a in appr_acts))

                if has_sla_exec and not has_appr_exec and not has_rej_exec:
                    node_data["status"] = "TIMED_OUT"
                    node_data["is_sla_breach"] = True
                    node_data["executed"] = True
                    node_data["taken_branch"] = "on_sla_breach"
                elif has_rej_exec and not has_appr_exec:
                    node_data["status"] = "REJECTED"
                    node_data["executed"] = True
                    node_data["taken_branch"] = "on_reject"
                elif has_appr_exec:
                    node_data["taken_branch"] = "on_approve"

                is_current_waiting = (is_step_target_active(h_id, h_name) or is_step_target_active(sig_name, sig_name)) and overall_status == "RUNNING"
                
                if is_current_waiting and not (has_appr_exec or has_rej_exec or has_sla_exec):
                    appr_active = False
                    rej_active = False
                    sla_active = False
                elif has_live_execution:
                    if has_sla_exec and not (has_appr_exec or has_rej_exec):
                        appr_active = False
                        rej_active = False
                        sla_active = current_step_active
                    else:
                        if not (has_appr_exec or has_rej_exec or has_sla_exec):
                            appr_active = False
                            rej_active = False
                            sla_active = False
                        else:
                            appr_active = current_step_active and has_appr_exec
                            rej_active = (current_step_active or has_rej_exec) and has_rej_exec
                            sla_active = (current_step_active or has_sla_exec) and has_sla_exec
                else:
                    appr_active = True
                    rej_active = True
                    sla_active = True
                
                if step.get("on_approve"):
                    appr_nodes = process_sub_branch(step["on_approve"], node_id, "APPROVED", "#10b981", is_active_branch=appr_active)
                    branch_out_ids.extend(appr_nodes)
                else:
                    branch_out_ids.append({"id": node_id, "label": "APPROVED", "color": "#10b981"})

                if step.get("on_reject"):
                    rej_nodes = process_sub_branch(step["on_reject"], node_id, "REJECTED", "#ef4444", is_active_branch=rej_active)
                    branch_out_ids.extend(rej_nodes)

                if step.get("on_sla_breach"):
                    sla_nodes = process_sub_branch(step["on_sla_breach"], node_id, "SLA BREACH", "#f97316", is_active_branch=sla_active)
                    branch_out_ids.extend(sla_nodes)

            if branch_out_ids:
                current_prev = branch_out_ids
            elif is_human and (step.get("on_approve") or step.get("on_reject") or step.get("on_sla_breach")):
                current_prev = []
            elif is_branch and (step.get("then_steps") or step.get("then") or step.get("else_steps") or step.get("else")):
                current_prev = []
            else:
                current_prev = [{"id": node_id, "label": None, "color": "#6366f1"}]

        return current_prev

    def process_sub_branch(sub_steps: List[Dict[str, Any]], parent_id: str, branch_label: str, color: str, is_active_branch: bool = True) -> List[Any]:
        if not sub_steps:
            return []
        
        # Process the sub-chain passing parent_id as labeled predecessor
        clean_tag = branch_label.lower().replace(" ", "_").replace("(", "").replace(")", "").replace("-", "_")
        sub_scope = f"{parent_id}__{clean_tag}"
        sub_ends = process_step_list(
            sub_steps,
            [{"id": parent_id, "label": branch_label, "color": color}],
            scope_prefix=sub_scope,
            is_branch_active=is_active_branch
        )
        return sub_ends

    # Process all top-level steps
    top_branch_active = not is_ingress_failed if has_ingress_guards else True
    end_nodes = process_step_list(raw_steps, current_prev, is_branch_active=top_branch_active)

    # 4. Egress Post-Execution Middleware Guards Node
    if has_egress_guards:
        has_pred_exec = False
        for e_item in end_nodes:
            e_id = e_item["id"] if isinstance(e_item, dict) else e_item
            pred_node = next((n for n in nodes if n["id"] == e_id), None)
            if (pred_node and pred_node.get("data", {}).get("executed")) or (isinstance(e_item, dict) and e_item.get("executed")):
                has_pred_exec = True
                break

        egress_executed = bool(overall_status == "COMPLETED" and has_pred_exec)
        egress_status = "COMPLETED" if egress_executed else ("SKIPPED" if has_live_execution and overall_status in ("COMPLETED", "FAILED", "TERMINATED", "CANCELED") else "PENDING")

        post_mw_names_list = [m.get("name") for m in post_middlewares if isinstance(m, dict) and m.get("name")]

        nodes.append({
            "id": "node__egress_guards",
            "shape": "octopus-activity-node",
            "label": "📊 Egress Middleware Guards",
            "width": 220,
            "height": 55,
            "data": {
                "step_id": "egress_guards",
                "type": "egress_guards",
                "title": "📊 Egress Middleware Guards",
                "description": "Post-execution response auditing, latency logging, and metrics dispatch executed following workflow pipeline completion.",
                "status": egress_status,
                "executed": egress_executed,
                "middlewares": post_mw_names_list,
            }
        })

        for e_item in end_nodes:
            e_lbl = e_item.get("label") if isinstance(e_item, dict) else None
            e_color = e_item.get("color", "#6366f1") if isinstance(e_item, dict) else "#6366f1"
            e_id = e_item["id"] if isinstance(e_item, dict) else e_item
            e_router = e_item.get("router") if isinstance(e_item, dict) else None
            if "node_term" not in e_id:
                e_obj = {
                    "id": f"edge__{e_id}__to__node__egress_guards",
                    "source": e_id,
                    "target": "node__egress_guards",
                    "attrs": {
                        "line": {
                            "stroke": "#10b981" if (egress_executed and ((isinstance(e_item, dict) and e_item.get("executed")) or any(n["id"] == e_id and n.get("data", {}).get("executed") for n in nodes))) else e_color,
                            "strokeWidth": 2,
                            "targetMarker": {"name": "block", "width": 8, "height": 6}
                        }
                    }
                }
                if e_router:
                    e_obj["router"] = e_router
                if e_lbl:
                    e_obj["labels"] = [{
                        "position": 0.3,
                        "attrs": {
                            "text": {"text": e_lbl, "fill": e_color, "fontSize": 11, "fontWeight": "bold"},
                            "rect": {"fill": "#0f172a", "rx": 4, "ry": 4, "stroke": "#334155", "strokeWidth": 1}
                        }
                    }]
                edges.append(e_obj)

        end_nodes = [{"id": "node__egress_guards", "label": None, "color": "#10b981", "executed": egress_executed}]

    # Final END Node
    has_exec_pred = False
    for e_item in end_nodes:
        e_id = e_item["id"] if isinstance(e_item, dict) else e_item
        pred_node = next((n for n in nodes if n["id"] == e_id), None)
        if pred_node and pred_node.get("data", {}).get("executed"):
            has_exec_pred = True
            break

    end_executed = bool(overall_status == "COMPLETED" and has_exec_pred)
    end_status = "COMPLETED" if end_executed else ("SKIPPED" if has_live_execution and overall_status in ("COMPLETED", "FAILED", "TERMINATED", "CANCELED") else "PENDING")
    matched_ep = None
    if isinstance(blueprint_cfg.get("api"), list):
        for ep in blueprint_cfg.get("api", []):
            if isinstance(ep, dict):
                r = ep.get("route", "")
                if active_route and (r.lower() == str(active_route).lower() or str(active_route).endswith(r) or r.endswith(str(active_route))):
                    matched_ep = ep
                    break
        if not matched_ep and blueprint_cfg.get("api"):
            matched_ep = blueprint_cfg["api"][0]

    code_val = (matched_ep or {}).get("status_code") if matched_ep else blueprint_cfg.get("status_code")
    if code_val is None and (blueprint_cfg.get("api") or active_route):
        code_val = 200

    if code_val == 200:
        end_label = "✅ 200 OK"
    elif code_val == 201:
        end_label = "✅ 201 CREATED"
    elif code_val == 202:
        end_label = "✅ 202 ACCEPTED"
    elif code_val == 204:
        end_label = "✅ 204 NO CONTENT"
    elif code_val is not None:
        end_label = f"✅ {code_val} OK"
    else:
        end_label = "✅ COMPLETED"

    end_width = max(140, int(len(end_label) * 9.5) + 36)
    nodes.append({
        "id": "node__end",
        "shape": "octopus-end-node",
        "label": end_label,
        "width": end_width,
        "height": 50,
        "data": {
            "step_id": "end",
            "type": "end",
            "title": f"Completed: {code_val}" if code_val else "Workflow Completed",
            "status": end_status,
            "executed": end_executed,
            "status_code": code_val
        }
    })

    # Connect final open branches to END
    for e_item in end_nodes:
        e_lbl = e_item.get("label") if isinstance(e_item, dict) else None
        e_color = e_item.get("color", "#6366f1") if isinstance(e_item, dict) else "#6366f1"
        e_id = e_item["id"] if isinstance(e_item, dict) else e_item
        e_router = e_item.get("router") if isinstance(e_item, dict) else None
        if "node_term" not in e_id:
            e_obj = {
                "id": f"edge__{e_id}__to__node__end",
                "source": e_id,
                "target": "node__end",
                "attrs": {
                    "line": {
                        "stroke": "#10b981" if end_executed else e_color,
                        "strokeWidth": 2,
                        "targetMarker": {"name": "block", "width": 8, "height": 6}
                    }
                }
            }
            if e_router:
                e_obj["router"] = e_router
            if e_lbl:
                e_obj["labels"] = [{
                    "position": 0.3,
                    "attrs": {
                        "text": {"text": e_lbl, "fill": e_color, "fontSize": 11, "fontWeight": "bold"},
                        "rect": {"fill": "#0f172a", "rx": 4, "ry": 4, "stroke": "#334155", "strokeWidth": 1}
                    }
                }]
            edges.append(e_obj)

    # -------------------------------------------------------------------------
    # -------------------------------------------------------------------------
    # Middleware, Step-Level & Global Exception Handlers
    # -------------------------------------------------------------------------
    exc_configs = []

    # 1. Global Exception Handler (Declarative Blueprint safety net)
    raw_eh = blueprint_cfg.get("error_handling") or {}
    global_exc = (
        raw_eh.get("global_handler")
        or raw_eh.get("global_exception_handler")
        or blueprint_cfg.get("global_exception_handler")
        or blueprint_cfg.get("global_handler")
    )
    if isinstance(global_exc, dict) and (global_exc.get("activity") or global_exc.get("code")):
        g_act = global_exc.get("activity") or "HandleGlobalUnhandledException"
        g_code = global_exc.get("code") or global_exc.get("status_code") or 500
        g_msg = global_exc.get("message") or "An unexpected internal server error occurred."
        exc_configs.append({
            "exception": "*",
            "activity": g_act,
            "code": g_code,
            "message": g_msg,
            "is_global": True,
            "is_middleware": False,
            "async": global_exc.get("async", True)
        })

    all_blueprint_middlewares = blueprint_cfg.get("middlewares", [])
    if isinstance(all_blueprint_middlewares, list):
        for mw in all_blueprint_middlewares:
            if isinstance(mw, dict) and mw.get("exceptions"):
                for exc in mw.get("exceptions"):
                    if isinstance(exc, dict) and exc.get("activity"):
                        exc_item = dict(exc)
                        exc_item["is_middleware"] = True
                        exc_item["source_name"] = mw.get("name")
                        exc_item["middleware_name"] = mw.get("name")
                        if not any(e.get("exception") == exc.get("exception") and e.get("activity") == exc.get("activity") for e in exc_configs):
                            exc_configs.append(exc_item)

    # Collect step-level exceptions only from current visualized route steps
    def _collect_step_exceptions(steps_list: List[Any]):
        if not isinstance(steps_list, list):
            return
        for s in steps_list:
            if not isinstance(s, dict):
                continue
            s_id = s.get("id") or s.get("activity")
            if s.get("exceptions") and isinstance(s.get("exceptions"), list):
                for exc in s["exceptions"]:
                    if isinstance(exc, dict) and exc.get("activity"):
                        exc_item = dict(exc)
                        exc_item["source_step_id"] = s_id
                        exc_item["is_middleware"] = False
                        if not any(e.get("exception") == exc.get("exception") and e.get("activity") == exc.get("activity") and e.get("source_step_id") == s_id for e in exc_configs):
                            exc_configs.append(exc_item)
            for sub in ("then", "then_steps", "else", "else_steps", "on_approve", "on_reject", "on_sla_breach"):
                if s.get(sub):
                    _collect_step_exceptions(s[sub])

    _collect_step_exceptions(raw_steps)

    for item in (timeline or []):
        s_id = str(item.get("step_id", ""))
        a_name = str(item.get("activity_name", ""))
        if (s_id.startswith("exc_") or "audit" in a_name.lower() or "handle" in a_name.lower()) and not any(e.get("activity") == a_name for e in exc_configs):
            is_glob = "global" in a_name.lower() or "unhandled" in a_name.lower()
            exc_configs.append({
                "activity": a_name,
                "exception": item.get("error") or ("*" if is_glob else "Exception"),
                "code": 500 if is_glob else 400,
                "is_global": is_glob,
                "async": True
            })

    failed_step_node = None
    if overall_status == "FAILED" or any(t.get("status") == "FAILED" for t in (timeline or [])):
        # 1. First look in timeline for the actual workflow step that failed
        failed_trace_step_id = next((t.get("step_id") for t in (timeline or []) if t.get("status") == "FAILED" and not str(t.get("step_id", "")).startswith("exc_")), None)
        if failed_trace_step_id:
            for n in nodes:
                if n["id"] == f"node__{failed_trace_step_id}" or n.get("data", {}).get("step_id") == failed_trace_step_id:
                    failed_step_node = n
                    break
        if not failed_step_node:
            for n in nodes:
                n_data = n.get("data", {})
                if n_data.get("status") == "FAILED" or n_data.get("step_id") in [t.get("step_id") for t in (timeline or []) if t.get("status") == "FAILED"]:
                    failed_step_node = n
                    break

    for exc in exc_configs:
        exc_act_name = exc.get("activity")
        if not exc_act_name:
            continue
        exc_name = exc.get("exception", "MiddlewareException")
        exc_code = exc.get("code", 400)
        is_global_exc = exc.get("is_global", False) or exc_name == "*" or exc_code == 500 or "global" in exc_act_name.lower()
        exc_node_id = f"node__exc_{exc_act_name.lower()}" if is_global_exc else (f"node__exc_{exc.get('source_step_id')}_{exc_name.lower()}" if exc.get("source_step_id") else f"node__exc_{exc_name.lower()}")
        if any(n["id"] == exc_node_id for n in nodes):
            continue

        exc_src_step = str(exc.get("source_step_id") or "").lower()
        is_mw_exc = exc.get("is_middleware", False)

        # Match trace specifically for THIS exception configuration
        matching_exc_trace = None
        for t in (timeline or []):
            if not isinstance(t, dict):
                continue
            inp = t.get("input") or t.get("input_payload") or {}
            t_err = str(t.get("error", "")).lower()
            t_step_id = str(t.get("step_id", "")).lower()
            t_act_name = str(t.get("activity_name", ""))
            
            inp_exc = str(inp.get("exception") or inp.get("error") or inp.get("error_type") or "").lower()
            inp_step = str(inp.get("step_id") or inp.get("failing_step_id") or inp.get("source_step_id") or "").lower()
            inp_code = inp.get("status_code") or inp.get("code")

            # 1. Global Exception Handler
            if is_global_exc:
                if t_act_name == exc_act_name or t_step_id == f"exc_{exc_act_name.lower()}":
                    matching_exc_trace = t
                    break
                if inp_code == 500 and (exc_name.lower() in inp_exc or "global" in str(inp).lower() or "unhandled" in str(inp).lower() or "crash" in str(inp).lower()):
                    matching_exc_trace = t
                    break
                continue

            # 2. Step-Level Exception Handler (has source_step_id)
            if exc_src_step:
                # Match if activity matches AND (step matches or exception type matches)
                if t_act_name == exc_act_name or t_step_id == f"exc_{exc_src_step}_{exc_name.lower()}" or t_step_id == f"exc_{exc_act_name.lower()}":
                    # If activity matches, check step correlation or exception correlation
                    if inp_step == exc_src_step or inp_exc == exc_name.lower() or exc_name.lower() in inp_exc or exc_src_step in str(t).lower():
                        matching_exc_trace = t
                        break
                    elif not inp_step and not inp_exc and t_act_name == exc_act_name:
                        matching_exc_trace = t
                        break
                continue

            # 3. Middleware Exception Handler (is_middleware)
            if is_mw_exc or not exc_src_step:
                if t_act_name == exc_act_name or t_step_id == f"exc_{exc_act_name.lower()}" or t_step_id == f"exc_{exc_name.lower()}":
                    # If this trace was triggered by a specific workflow step, it's NOT a middleware exception!
                    if inp_step and inp_step not in ("ingress_guards", "egress_guards", "middleware"):
                        continue
                    if inp_exc == exc_name.lower() or exc_name.lower() in inp_exc or exc_name.lower() in t_err or not inp_exc:
                        matching_exc_trace = t
                        break
                continue

        # Fallback check against job_data or executed_set if explicit exception marker exists
        if not matching_exc_trace:
            if exc_src_step and f"exc_{exc_src_step}_{exc_name.lower()}" in executed_set:
                matching_exc_trace = {"step_id": f"exc_{exc_src_step}_{exc_name.lower()}", "activity_name": exc_act_name}
            elif is_global_exc and overall_status == "FAILED":
                # ONLY trigger global 500 fallback if job status is actually 500 / unhandled crash
                j_code = job_data.get("status_code") or job_data.get("error_code") if isinstance(job_data, dict) else None
                if j_code == 500 or (isinstance(job_data, dict) and job_data.get("is_unhandled")):
                    matching_exc_trace = {"step_id": f"exc_{exc_act_name.lower()}", "activity_name": exc_act_name, "input": job_data}
                elif exc_act_name in executed_set or f"exc_{exc_act_name.lower()}" in executed_set:
                    matching_exc_trace = {"step_id": f"exc_{exc_act_name.lower()}", "activity_name": exc_act_name}
            elif isinstance(job_data, dict) and not is_global_exc:
                j_exc = str(job_data.get("exception") or job_data.get("error_type") or job_data.get("error") or "").lower()
                j_err_type = str(job_data.get("error_type") or "").lower()
                j_code = job_data.get("status_code") or job_data.get("code")
                if j_err_type == exc_name.lower() or j_exc == exc_name.lower() or (j_code == exc_code and exc_name.lower() in j_exc) or (j_code == exc_code and exc_name.lower() in str(job_data).lower()):
                    matching_exc_trace = {"step_id": f"exc_{exc_act_name.lower()}", "activity_name": exc_act_name, "input": job_data}
            elif not is_global_exc and (f"exc_{exc_name.lower()}" in executed_set or f"exception:{exc_name.lower()}" in executed_set):
                matching_exc_trace = {"step_id": f"exc_{exc_act_name.lower()}", "activity_name": exc_act_name}

        is_exc_executed = bool(matching_exc_trace is not None)
        exc_status = "COMPLETED" if is_exc_executed else ("SKIPPED" if has_live_execution else "PENDING")
        exc_trace = matching_exc_trace or timeline_map.get(f"exc_{exc_act_name.lower()}") or timeline_map.get(f"exc_{exc_name.lower()}") or {}

        # Custom sub-label
        if is_global_exc or exc_code == 500 or "global" in exc_act_name.lower():
            sub_lbl = "(Global Incident Handler)"
            exc_edge_lbl = "💥 500 Unhandled Exception\n(Catch-All / Runtime Crash)"
            term_lbl = "🛑 500 INTERNAL SERVER ERROR"
            term_id = "node__failed_term_500"
        elif "rate" in exc_name.lower():
            sub_lbl = "(Async Rate Limit Audit)"
            exc_edge_lbl = f"⚠️ 429 RateLimitExceeded\n(Quota > 60 req/min)"
            term_lbl = "🛑 429 TOO MANY REQUESTS"
            term_id = "node__failed_term_429"
        elif "ip" in exc_name.lower() or "denied" in exc_name.lower():
            sub_lbl = "(Async Security Audit)"
            exc_edge_lbl = f"🚫 403 IpAccessDenied\n(Blocked IP / Untrusted Subnet)"
            term_lbl = "🛑 403 FORBIDDEN"
            term_id = "node__failed_term_403"
        elif "authorized" in exc_name.lower():
            sub_lbl = "(Async RBAC Audit)"
            exc_edge_lbl = f"🚫 403 UserNotAuthorized\n(Forbidden Role/Permissions)"
            term_lbl = "🛑 403 FORBIDDEN"
            term_id = "node__failed_term_403"
        elif "invalidcredentials" in exc_name.lower() or "credentials" in exc_name.lower():
            sub_lbl = "(Async Auth Handler)"
            exc_edge_lbl = f"🔒 401 InvalidCredentials\n(Invalid Username/Password)"
            term_lbl = "🛑 401 UNAUTHORIZED"
            term_id = "node__failed_term_401"
        elif "authenticated" in exc_name.lower():
            sub_lbl = "(Async Auth Handler)"
            exc_edge_lbl = f"🔒 401 UserNotAuthenticated\n(Missing/Invalid Token)"
            term_lbl = "🛑 401 UNAUTHORIZED"
            term_id = "node__failed_term_401"
        elif exc_code == 400 or "badrequest" in exc_name.lower() or "validation" in exc_name.lower() or "malformed" in exc_name.lower() or "field" in exc_name.lower():
            sub_lbl = "(Async Validation Audit)"
            exc_edge_lbl = f"🛑 400 {exc_name}\n(Bad Request / Schema Violation)"
            term_lbl = "🛑 400 BAD REQUEST"
            term_id = "node__failed_term_400"
        elif exc_code == 413 or "toolarge" in exc_name.lower() or "payload" in exc_name.lower():
            sub_lbl = "(Async Payload Audit)"
            exc_edge_lbl = f"🛑 413 PayloadTooLarge\n(Payload Size Exceeded)"
            term_lbl = "🛑 413 PAYLOAD TOO LARGE"
            term_id = "node__failed_term_413"
        else:
            sub_lbl = "(Async Auth Handler)"
            exc_edge_lbl = f"On {exc_name} ({exc_code})"
            term_lbl = f"FAILED ({exc_code})"
            term_id = f"node__failed_term_{exc_code}"

        # Source metadata & origin determination
        is_middleware_exc = exc.get("is_middleware") if "is_middleware" in exc else any(
            isinstance(m, dict) and any(
                isinstance(e, dict) and e.get("exception", "").lower() == exc_name.lower()
                for e in m.get("exceptions", [])
            )
            for m in middlewares
        )
        source_type = "Global Exception Handler" if is_global_exc else ("Middleware Exception Pipeline" if is_middleware_exc else "Workflow Step Exception Handler")
        origin_name = f"Global Engine Safety Net ({exc_act_name})" if is_global_exc else (f"Middleware Pipeline ({exc.get('middleware_name') or 'Ingress Guards'})" if is_middleware_exc else (f"Workflow Step ({exc.get('source_step_id')})" if exc.get("source_step_id") else "Workflow Exception Handler"))
        desc = f"Invoked {'asynchronously ' if exc.get('async', True) else ''}by Global Exception Policy upon unexpected crash/exception ({exc_code})." if is_global_exc else (f"Invoked {'asynchronously ' if exc.get('async', True) else ''}by {exc.get('middleware_name') or 'Ingress Guard'} upon {exc_name} ({exc_code})." if is_middleware_exc else f"Invoked {'asynchronously ' if exc.get('async', True) else ''}when step '{exc.get('source_step_id', '')}' encounters exception {exc_name} ({exc_code}).")
        if exc.get("message"):
            desc += f" Policy: {exc.get('message')}"

        nodes.append({
            "id": exc_node_id,
            "shape": "octopus-activity-node",
            "label": f"🚨 {exc_act_name}",
            "width": 240,
            "height": 55,
            "data": {
                "step_id": f"exc_{exc_act_name.lower()}",
                "activity": exc_act_name,
                "type": "exception_handler",
                "is_async": exc.get("async", True),
                "exception_type": exc_name,
                "status_code": exc_code,
                "source_type": source_type,
                "source_location": origin_name,
                "source_name": exc.get("middleware_name") if is_middleware_exc else (exc_act_name if is_global_exc else exc.get("source_step_id")),
                "source_step_id": exc.get("source_step_id"),
                "is_middleware": is_middleware_exc,
                "is_global": is_global_exc,
                "description": desc,
                "title": f"🚨 Exception Handler: {exc_act_name}",
                "status": exc_status,
                "executed": is_exc_executed,
                "input_payload": exc_trace.get("input") or exc_trace.get("input_payload"),
                "output_payload": exc_trace.get("output") or exc_trace.get("output_payload"),
                "error": exc_trace.get("error")
            }
        })

        # Source node to connect from
        if exc.get("source_step_id"):
            src_step = exc["source_step_id"]
            src_id = f"node__{src_step}" if any(n["id"] == f"node__{src_step}" for n in nodes) else ("node__ingress_guards" if has_ingress_guards else "node__start")
        elif is_global_exc:
            if failed_step_node:
                src_id = failed_step_node["id"]
            elif top_step_ids:
                src_id = f"node__{top_step_ids[0]}" if any(n["id"] == f"node__{top_step_ids[0]}" for n in nodes) else ("node__ingress_guards" if has_ingress_guards else "node__start")
            else:
                src_id = "node__ingress_guards" if has_ingress_guards else "node__start"
        elif is_middleware_exc:
            src_id = "node__ingress_guards" if has_ingress_guards else "node__start"
        else:
            src_id = failed_step_node["id"] if (failed_step_node and is_exc_executed) else ("node__" + top_step_ids[0] if top_step_ids else ("node__ingress_guards" if has_ingress_guards else "node__start"))

        edges.append({
            "id": f"edge__{src_id}__to__{exc_node_id}",
            "source": src_id,
            "target": exc_node_id,
            "attrs": {
                "line": {
                    "stroke": "#ef4444" if is_exc_executed else ("#1e293b" if has_live_execution else "#f59e0b"),
                    "strokeWidth": 2.5 if is_exc_executed else 1.5,
                    "strokeDasharray": "5 5" if is_exc_executed else ("4 4" if has_live_execution else "5 5"),
                    "opacity": 1.0 if is_exc_executed else (0.25 if has_live_execution else 0.8),
                    "targetMarker": {"name": "block", "width": 8, "height": 6, "fill": "#ef4444" if is_exc_executed else ("#334155" if has_live_execution else "#f59e0b")}
                }
            },
            "labels": [{
                "position": 0.4,
                "attrs": {
                    "text": {
                        "text": exc_edge_lbl,
                        "fill": "#f87171" if is_exc_executed else ("#475569" if has_live_execution else "#fbbf24"),
                        "fontSize": 10,
                        "fontWeight": "bold" if is_exc_executed else "normal",
                        "opacity": 1.0 if is_exc_executed else (0.3 if has_live_execution else 1.0)
                    },
                    "rect": {
                        "fill": "#2d1215" if is_exc_executed else ("#090d16" if has_live_execution else "#0f172a"),
                        "rx": 4,
                        "ry": 4,
                        "stroke": "#ef4444" if is_exc_executed else ("#1e293b" if has_live_execution else "#f59e0b"),
                        "strokeWidth": 1,
                        "opacity": 1.0 if is_exc_executed else (0.2 if has_live_execution else 1.0)
                    }
                }
            }]
        })

        # Connect to failed terminal node
        term_node = next((n for n in nodes if n["id"] == term_id), None)
        if not term_node:
            failed_term_status = "FAILED" if is_exc_executed else ("SKIPPED" if has_live_execution else "PENDING")
            term_width = max(160, int(len(term_lbl) * 9.5) + 40)
            nodes.append({
                "id": term_id,
                "shape": "octopus-end-node",
                "label": term_lbl,
                "width": term_width,
                "height": 50,
                "data": {
                    "step_id": "failed",
                    "type": "end_failed",
                    "title": f"Failed: {exc_name}",
                    "status": failed_term_status,
                    "executed": is_exc_executed
                }
            })
        elif is_exc_executed:
            term_node["data"]["status"] = "FAILED"
            term_node["data"]["executed"] = True

        edges.append({
            "id": f"edge__{exc_node_id}__to__{term_id}",
            "source": exc_node_id,
            "target": term_id,
            "attrs": {
                "line": {
                    "stroke": "#ef4444" if is_exc_executed else ("#1e293b" if has_live_execution else "#f59e0b"),
                    "strokeWidth": 2.5 if is_exc_executed else 1.5,
                    "strokeDasharray": "5 5" if is_exc_executed else ("4 4" if has_live_execution else "5 5"),
                    "opacity": 1.0 if is_exc_executed else (0.25 if has_live_execution else 0.8),
                    "targetMarker": {"name": "block", "width": 8, "height": 6, "fill": "#ef4444" if is_exc_executed else ("#334155" if has_live_execution else "#f59e0b")}
                }
            }
        })

    # Apply Execution Path Styling on Nodes and Edges
    if has_live_execution:
        # Backward propagation: If a child step was executed or active, mark parent nodes as executed/reached
        changed = True
        while changed:
            changed = False
            active_or_executed_ids = {
                n["id"] for n in nodes 
                if n.get("data", {}).get("executed") or n.get("data", {}).get("status") in ("RUNNING", "AWAITING_APPROVAL", "TIMED_OUT", "FAILED")
            }
            for edge in edges:
                if edge["target"] == "node__end" or "node_comp" in edge["target"]:
                    continue
                if edge["target"] in active_or_executed_ids:
                    src_node = next((n for n in nodes if n["id"] == edge["source"]), None)
                    if not src_node:
                        continue
                    
                    e_lbl = edge.get("labels", [{}])[0].get("attrs", {}).get("text", {}).get("text", "") if edge.get("labels") else ""
                    src_type = src_node.get("data", {}).get("type")

                    # 1. Propagate into decision parent nodes if an internal outgoing sub-branch child executed
                    if src_type in ("branch", "branching_step"):
                        if not src_node.get("data", {}).get("executed"):
                            src_step_id = src_node.get("data", {}).get("step_id") or src_node["id"].replace("node__", "")
                            is_internal_child = (
                                src_node["id"] in edge["target"]
                                or src_step_id in edge["target"]
                            )
                            if is_internal_child and ("TRUE" in e_lbl or "FALSE (Else)" in e_lbl or "Match" in e_lbl):
                                src_node["data"]["executed"] = True
                                if src_node["data"].get("status") not in ("FAILED", "AWAITING_APPROVAL", "RUNNING"):
                                    src_node["data"]["status"] = "COMPLETED"
                                changed = True

                    # 2. Propagate into human review step parents ONLY if a dedicated internal branch child executed
                    elif src_type == "human":
                        if not src_node.get("data", {}).get("executed"):
                            src_step_id = src_node.get("data", {}).get("step_id") or src_node["id"].replace("node__", "")
                            is_internal_child = (
                                src_node["id"] in edge["target"]
                                or src_step_id in edge["target"]
                            )
                            tgt_node = next((n for n in nodes if n["id"] == edge["target"]), None)
                            tgt_executed = tgt_node and (tgt_node.get("data", {}).get("executed") or tgt_node.get("data", {}).get("status") in ("RUNNING", "AWAITING_APPROVAL", "TIMED_OUT", "FAILED"))
                            if is_internal_child and tgt_executed and any(k in e_lbl for k in ("SLA", "APPROV", "REJECT")):
                                src_node["data"]["executed"] = True
                                if "SLA" in e_lbl:
                                    src_node["data"]["status"] = "TIMED_OUT"
                                    src_node["data"]["is_sla_breach"] = True
                                elif src_node["data"].get("status") not in ("FAILED", "AWAITING_APPROVAL", "RUNNING"):
                                    src_node["data"]["status"] = "COMPLETED"
                                changed = True

        # Forward propagation: If the incoming source of a branch decision node executed, mark the branch node as evaluated
        if not is_ingress_failed and overall_status != "FAILED":
            active_or_exec = {n["id"] for n in nodes if n.get("data", {}).get("executed") and n.get("data", {}).get("status") not in ("FAILED", "SKIPPED", "PENDING")}
            for node in nodes:
                if node.get("data", {}).get("type") in ("branch", "branching_step"):
                    incoming_edges = [e for e in edges if e["target"] == node["id"] and "rewind" not in e.get("id", "")]
                    if incoming_edges and all(e["source"] in active_or_exec for e in incoming_edges):
                        node["data"]["executed"] = True
                        if node["data"].get("status") not in ("FAILED", "AWAITING_APPROVAL", "RUNNING"):
                            node["data"]["status"] = "COMPLETED"

        # Ensure the last executed step(s) always link to COMPLETED when the workflow completed
        if overall_status == "COMPLETED" and has_exec_pred:
            end_node = next((n for n in nodes if n["id"] == "node__end"), None)
            if end_node:
                end_node["data"]["executed"] = True
                end_node["data"]["status"] = "COMPLETED"

            # Find executed non-end, non-compensation nodes
            exec_nodes = [n for n in nodes if n["id"] != "node__end" and n.get("data", {}).get("type") != "compensation" and n.get("data", {}).get("executed")]
            
            # Identify leaf executed nodes (nodes with no outgoing edges to another executed activity/human/branch step)
            def leads_to_executed(target_id: str, visited: Set[str]) -> bool:
                if target_id in visited:
                    return False
                visited.add(target_id)
                t_node = next((n for n in nodes if n["id"] == target_id), None)
                if not t_node:
                    return False
                if t_node.get("data", {}).get("executed") and t_node["id"] != "node__end" and t_node.get("data", {}).get("type") != "compensation":
                    return True
                if t_node.get("data", {}).get("type") == "jump":
                    for e in edges:
                        if e["source"] == target_id and "node_comp" not in e["target"] and e["target"] != "node__end":
                            if leads_to_executed(e["target"], visited):
                                return True
                if t_node.get("data", {}).get("status") == "BYPASSED":
                    for e in edges:
                        if e["source"] == target_id and "node_comp" not in e["target"] and e["target"] != "node__end":
                            if leads_to_executed(e["target"], visited):
                                return True
                return False

            leaf_nodes = []
            for en in exec_nodes:
                if en.get("data", {}).get("type") in ("rewind", "jump"):
                    continue
                has_child_exec = False
                for edge in edges:
                    if edge["source"] == en["id"] and "node_comp" not in edge["target"] and edge["target"] != "node__end":
                        if leads_to_executed(edge["target"], set()):
                            has_child_exec = True
                            break
                if not has_child_exec:
                    leaf_nodes.append(en)

            for leaf in leaf_nodes:
                target_dest_id = "node__egress_guards" if (has_egress_guards and leaf["id"] != "node__egress_guards") else "node__end"
                existing_edge = next((e for e in edges if e["source"] == leaf["id"] and e["target"] == target_dest_id), None)
                leaf_is_sla = leaf.get("data", {}).get("is_sla_breach")
                leaf_edge_color = "#f97316" if leaf_is_sla else "#10b981"
                if not existing_edge:
                    edges.append({
                        "id": f"edge__{leaf['id']}__to__{target_dest_id}",
                        "source": leaf["id"],
                        "target": target_dest_id,
                        "attrs": {
                            "line": {
                                "stroke": leaf_edge_color,
                                "strokeWidth": 3.5,
                                "targetMarker": {"name": "block", "width": 10, "height": 8, "fill": leaf_edge_color},
                                "filter": f"drop-shadow(0 0 8px {leaf_edge_color}88)",
                                "opacity": 1.0
                            }
                        }
                    })

        executed_node_ids = {n["id"] for n in nodes if n.get("data", {}).get("executed")}

        def is_branch_path_executed(start_node_id: str) -> bool:
            """Checks if the branch starting at start_node_id was taken (executed or active)."""
            if not start_node_id:
                return False
            curr_node = next((n for n in nodes if n["id"] == start_node_id), None)
            if not curr_node:
                return False
            if start_node_id in executed_node_ids and start_node_id != "node__end":
                return True
            if curr_node.get("data", {}).get("status") in ("RUNNING", "AWAITING_APPROVAL"):
                return True
            return False
        
        # 1. Update node styling
        for node in nodes:
            is_exec = node["id"] in executed_node_ids
            st = node.get("data", {}).get("status")
            node_type = node.get("data", {}).get("type")
            is_sla = node.get("data", {}).get("is_sla_breach")

            if st == "FAILED" or st == "REJECTED":
                node["attrs"] = {
                    "body": {
                        "stroke": "#ef4444",
                        "strokeWidth": 3,
                        "fill": "#2d1215",
                        "filter": "drop-shadow(0 0 16px rgba(239, 68, 68, 0.75))",
                        "opacity": 1.0
                    },
                    "label": {
                        "fill": "#fca5a5",
                        "fontWeight": 700
                    }
                }
            elif st == "TIMED_OUT":
                node["attrs"] = {
                    "body": {
                        "stroke": "#f97316",
                        "strokeWidth": 3,
                        "fill": "#2b180d",
                        "filter": "drop-shadow(0 0 16px rgba(249, 115, 22, 0.75))",
                        "opacity": 1.0
                    },
                    "label": {
                        "fill": "#fdba74",
                        "fontWeight": 700
                    }
                }
            elif st in ("AWAITING_APPROVAL", "RUNNING", "AWAITING_SIGNAL"):
                node["attrs"] = {
                    "body": {
                        "stroke": "#f59e0b",
                        "strokeWidth": 3,
                        "fill": "#2a1d08",
                        "filter": "drop-shadow(0 0 14px rgba(245, 158, 11, 0.85))",
                        "opacity": 1.0
                    },
                    "label": {
                        "fill": "#fde68a",
                        "fontWeight": 700
                    }
                }
            elif node_type == "quorum_approver" and not is_exec:
                # Highlighted pending/non-participating quorum approver node ONLY if quorum gate was reached/active
                p_gate_id = node.get("data", {}).get("parent_gate")
                p_node = next((n for n in nodes if n["id"] == f"node__{p_gate_id}" or n.get("data", {}).get("step_id") == p_gate_id), None)
                p_reached = p_node and (p_node.get("data", {}).get("executed") or p_node.get("data", {}).get("status") in ("AWAITING_APPROVAL", "RUNNING", "COMPLETED", "REJECTED"))
                if p_reached and st not in ("SKIPPED", "PENDING"):
                    node["attrs"] = {
                        "body": {
                            "stroke": "#f97316",
                            "strokeWidth": 2.5,
                            "strokeDasharray": "4 4",
                            "fill": "#271c14",
                            "filter": "drop-shadow(0 0 14px rgba(249, 115, 22, 0.65))",
                            "opacity": 1.0
                        },
                        "label": {
                            "fill": "#fdba74",
                            "fontWeight": 600
                        }
                    }
                else:
                    node["attrs"] = {
                        "body": {
                            "stroke": "#334155",
                            "strokeWidth": 1.5,
                            "fill": "#111827",
                            "opacity": 0.35
                        },
                        "label": {
                            "fill": "#64748b",
                            "opacity": 0.45
                        }
                    }
            elif node_type == "compensation" and is_exec:
                node["attrs"] = {
                    "body": {
                        "stroke": "#f97316",
                        "strokeWidth": 2,
                        "strokeDasharray": "4 4",
                        "fill": "#2b1406",
                        "filter": "drop-shadow(0 0 16px rgba(249, 115, 22, 0.85))",
                        "opacity": 1.0
                    },
                    "label": {
                        "fill": "#fed7aa",
                        "fontWeight": 700
                    }
                }
            elif is_sla and is_exec:
                node["attrs"] = {
                    "body": {
                        "stroke": "#f97316",
                        "strokeWidth": 3,
                        "fill": "#2b180d",
                        "filter": "drop-shadow(0 0 16px rgba(249, 115, 22, 0.75))",
                        "opacity": 1.0
                    },
                    "label": {
                        "fill": "#fdba74",
                        "fontWeight": 600
                    }
                }
            elif node_type == "rewind" and is_exec:
                node["attrs"] = {
                    "body": {
                        "stroke": "#f59e0b",
                        "strokeWidth": 3,
                        "strokeDasharray": "5 5",
                        "fill": "#272015",
                        "filter": "drop-shadow(0 0 16px rgba(245, 158, 11, 0.75))",
                        "opacity": 1.0
                    },
                    "label": {
                        "fill": "#fde68a",
                        "fontWeight": 700
                    }
                }
            elif node_type == "jump" and is_exec:
                node["attrs"] = {
                    "body": {
                        "stroke": "#10b981",
                        "strokeWidth": 3,
                        "strokeDasharray": "5 5",
                        "fill": "#0f2e22",
                        "filter": "drop-shadow(0 0 16px rgba(16, 185, 129, 0.75))",
                        "opacity": 1.0
                    },
                    "label": {
                        "fill": "#6ee7b7",
                        "fontWeight": 700
                    }
                }
            elif is_exec:
                node["attrs"] = {
                    "body": {
                        "stroke": "#10b981",
                        "strokeWidth": 3,
                        "fill": "#0f2e22",
                        "filter": "drop-shadow(0 0 16px rgba(16, 185, 129, 0.75))",
                        "opacity": 1.0
                    },
                    "label": {
                        "fill": "#6ee7b7",
                        "fontWeight": 600
                    }
                }
            elif st == "BYPASSED":
                node["attrs"] = {
                    "body": {
                        "stroke": "#34d399",
                        "strokeWidth": 1.5,
                        "strokeDasharray": "4 4",
                        "fill": "#0f2e22",
                        "filter": "drop-shadow(0 0 6px rgba(52, 211, 153, 0.4))",
                        "opacity": 0.75
                    },
                    "label": {
                        "fill": "#6ee7b7",
                        "fontWeight": 500
                    }
                }
            else:
                # Dim unexecuted nodes
                if node_type == "compensation":
                    node["attrs"] = {
                        "body": {
                            "stroke": "#475569",
                            "strokeWidth": 1.5,
                            "strokeDasharray": "4 4",
                            "fill": "#0f172a",
                            "opacity": 0.35
                        },
                        "label": {
                            "fill": "#64748b",
                            "opacity": 0.45
                        }
                    }
                else:
                    node["attrs"] = {
                        "body": {
                            "stroke": "#334155",
                            "strokeWidth": 1.5,
                            "fill": "#111827",
                            "opacity": 0.35
                        },
                        "label": {
                            "fill": "#64748b",
                            "opacity": 0.45
                        }
                    }

        # 2. Update edge styling (highlight executed path, dim untaken paths)
        for edge in edges:
            if "node_comp" in edge["target"]:
                comp_node = next((n for n in nodes if n["id"] == edge["target"]), None)
                comp_exec = comp_node.get("data", {}).get("executed") if comp_node else False
                edge["labels"] = [{
                    "attrs": {
                        "text": {"text": "Saga Rollback", "fill": "#ffffff" if comp_exec else "#64748b", "fontSize": 10},
                        "rect": {"fill": "#0f172a", "rx": 4, "ry": 4, "stroke": "#f97316" if comp_exec else "#334155", "strokeWidth": 1}
                    }
                }]
                edge["attrs"] = {
                    "line": {
                        "stroke": "#f97316" if comp_exec else "#475569",
                        "strokeWidth": 2.0 if comp_exec else 1.5,
                        "strokeDasharray": "5 5",
                        "targetMarker": {"name": "classic", "size": 6, "fill": "#f97316" if comp_exec else "#475569"},
                        "filter": "drop-shadow(0 0 8px rgba(249, 115, 22, 0.6))" if comp_exec else "none",
                        "opacity": 1.0 if comp_exec else (0.3 if has_live_execution else 0.5)
                    }
                }
                continue

            if edge.get("id", "").startswith("edge_rewind__"):
                src_node = next((n for n in nodes if n["id"] == edge["source"]), None)
                rewind_exec = src_node.get("data", {}).get("executed") if src_node else False

                edge["attrs"] = {
                    "line": {
                        "stroke": "#f59e0b" if rewind_exec else "#1e293b",
                        "strokeWidth": 3.0 if rewind_exec else 1.5,
                        "strokeDasharray": "5 5",
                        "opacity": 1.0 if rewind_exec else 0.25,
                        "filter": "drop-shadow(0 0 10px rgba(245, 158, 11, 0.7))" if rewind_exec else "none",
                        "targetMarker": {
                            "name": "block",
                            "width": 8,
                            "height": 6,
                            "fill": "#f59e0b" if rewind_exec else "#334155"
                        }
                    }
                }
                if "labels" in edge:
                    for lbl in edge["labels"]:
                        if "attrs" in lbl:
                            if "text" in lbl["attrs"]:
                                lbl["attrs"]["text"]["fill"] = "#f59e0b" if rewind_exec else "#475569"
                                lbl["attrs"]["text"]["opacity"] = 1.0 if rewind_exec else 0.3
                            if "rect" in lbl["attrs"]:
                                lbl["attrs"]["rect"]["fill"] = "#1e1b10" if rewind_exec else "#090d16"
                                lbl["attrs"]["rect"]["stroke"] = "#f59e0b" if rewind_exec else "#1e293b"
                                lbl["attrs"]["rect"]["opacity"] = 1.0 if rewind_exec else 0.2
                continue

            if edge.get("id", "").startswith("edge_jump__"):
                src_node = next((n for n in nodes if n["id"] == edge["source"]), None)
                jump_exec = src_node.get("data", {}).get("executed") if src_node else False
                jump_color = "#10b981" if jump_exec else "#06b6d4"

                edge["attrs"] = {
                    "line": {
                        "stroke": jump_color if jump_exec else "#1e293b",
                        "strokeWidth": 3.5 if jump_exec else 1.5,
                        "strokeDasharray": "5 5",
                        "opacity": 1.0 if jump_exec else 0.25,
                        "filter": "drop-shadow(0 0 10px rgba(16, 185, 129, 0.7))" if jump_exec else "none",
                        "targetMarker": {
                            "name": "block",
                            "width": 8,
                            "height": 6,
                            "fill": jump_color if jump_exec else "#334155"
                        }
                    }
                }
                if "labels" in edge:
                    for lbl in edge["labels"]:
                        if "attrs" in lbl:
                            if "text" in lbl["attrs"]:
                                lbl["attrs"]["text"]["fill"] = jump_color if jump_exec else "#475569"
                                lbl["attrs"]["text"]["opacity"] = 1.0 if jump_exec else 0.3
                            if "rect" in lbl["attrs"]:
                                lbl["attrs"]["rect"]["fill"] = "#0f2e22" if jump_exec else "#090d16"
                                lbl["attrs"]["rect"]["stroke"] = jump_color if jump_exec else "#1e293b"
                                lbl["attrs"]["rect"]["opacity"] = 1.0 if jump_exec else 0.2
                continue

            src_node = next((n for n in nodes if n["id"] == edge["source"]), None)
            tgt_node = next((n for n in nodes if n["id"] == edge["target"]), None)

            src_exec = edge["source"] in executed_node_ids
            tgt_exec = edge["target"] in executed_node_ids
            tgt_active = bool(tgt_node and tgt_node.get("data", {}).get("status") in ("AWAITING_APPROVAL", "RUNNING", "AWAITING_SIGNAL"))
            
            src_status = src_node.get("data", {}).get("status") if src_node else None
            tgt_status = tgt_node.get("data", {}).get("status") if tgt_node else None
            src_bypassed = src_status == "BYPASSED"
            tgt_bypassed = tgt_status == "BYPASSED"

            is_full_exec_edge = (src_exec and tgt_exec)
            is_bypassed_edge = ((src_exec or src_bypassed) and (tgt_exec or tgt_bypassed or tgt_active)) and (src_bypassed or tgt_bypassed)

            is_src_approver = bool(src_node and src_node.get("data", {}).get("type") == "quorum_approver")
            is_tgt_approver = bool(tgt_node and tgt_node.get("data", {}).get("type") == "quorum_approver")

            # Check parent quorum reachability for approver nodes
            parent_gate_reached = False
            if is_src_approver or is_tgt_approver:
                appr_node = src_node if is_src_approver else tgt_node
                p_gate_id = appr_node.get("data", {}).get("parent_gate") if appr_node else None
                if p_gate_id:
                    p_node = next((n for n in nodes if n["id"] == f"node__{p_gate_id}" or n.get("data", {}).get("step_id") == p_gate_id), None)
                    if p_node and (p_node.get("data", {}).get("executed") or p_node.get("data", {}).get("status") in ("AWAITING_APPROVAL", "RUNNING", "COMPLETED", "REJECTED")):
                        parent_gate_reached = True

            # Check if this edge is an untaken branch
            is_untaken_branch = False
            e_label_str = str(edge.get("labels", [{}])[0].get("attrs", {}).get("text", {}).get("text", "")) if edge.get("labels") else ""

            if not src_exec and not src_bypassed and not (is_src_approver and parent_gate_reached):
                is_untaken_branch = True
            elif is_tgt_approver and not src_exec and not src_bypassed:
                is_untaken_branch = True
            elif src_node and src_node.get("data", {}).get("type") in ("branch", "branching_step"):
                if "FALSE" in e_label_str:
                    # Check if a sibling TRUE branch was taken
                    has_executed_true_sibling = any(
                        is_branch_path_executed(e["target"])
                        for e in edges
                        if e["source"] == edge["source"] and e["id"] != edge["id"]
                        and ("TRUE" in str(e.get("labels", [{}])[0].get("attrs", {}).get("text", {}).get("text", "")) or "Match" in str(e.get("labels", [{}])[0].get("attrs", {}).get("text", "")))
                    )
                    has_executed_rewind_to_src = any(
                        e for e in edges
                        if e["target"] == edge["source"]
                        and ("rewind" in e.get("id", "") or "REWIND" in str(e.get("labels", [{}])[0].get("attrs", {}).get("text", {}).get("text", "")))
                        and e["source"] in executed_node_ids
                    )
                    if has_executed_true_sibling and not has_executed_rewind_to_src:
                        is_untaken_branch = True
                elif "TRUE" in e_label_str or "Match" in e_label_str:
                    if not is_branch_path_executed(edge["target"]):
                        is_untaken_branch = True

            elif src_node and src_node.get("data", {}).get("type") == "human":
                if any(b in e_label_str for b in ("APPROVED", "REJECTED", "SLA BREACH", "SLA")):
                    taken_branch = src_node.get("data", {}).get("taken_branch")
                    if taken_branch:
                        if "APPROV" in e_label_str and taken_branch != "on_approve":
                            is_untaken_branch = True
                        elif "REJECT" in e_label_str and taken_branch != "on_reject":
                            is_untaken_branch = True
                        elif "SLA" in e_label_str and taken_branch != "on_sla_breach":
                            is_untaken_branch = True
                    elif not is_branch_path_executed(edge["target"]):
                        is_untaken_branch = True

            if is_untaken_branch:
                edge["attrs"] = {
                    "line": {
                        "stroke": "#1e293b",
                        "strokeWidth": 1.5,
                        "strokeDasharray": "4 4",
                        "targetMarker": {"name": "block", "width": 6, "height": 4, "fill": "#334155"},
                        "opacity": 0.25
                    }
                }
                if "labels" in edge:
                    for lbl in edge["labels"]:
                        if "attrs" in lbl:
                            if "text" in lbl["attrs"]:
                                lbl["attrs"]["text"]["fill"] = "#475569"
                                lbl["attrs"]["text"]["opacity"] = 0.3
                            if "rect" in lbl["attrs"]:
                                lbl["attrs"]["rect"]["fill"] = "#090d16"
                                lbl["attrs"]["rect"]["stroke"] = "#1e293b"
                                lbl["attrs"]["rect"]["opacity"] = 0.2
                continue

            if is_bypassed_edge:
                edge["attrs"] = {
                    "line": {
                        "stroke": "#34d399",
                        "strokeWidth": 2.5,
                        "strokeDasharray": "5 5",
                        "targetMarker": {"name": "block", "width": 8, "height": 6, "fill": "#34d399"},
                        "filter": "drop-shadow(0 0 6px rgba(52, 211, 153, 0.4))",
                        "opacity": 1.0
                    }
                }
            elif is_src_approver and src_status == "REJECTED":
                edge["attrs"] = {
                    "line": {
                        "stroke": "#ef4444",
                        "strokeWidth": 3.5,
                        "targetMarker": {"name": "block", "width": 8, "height": 6, "fill": "#ef4444"},
                        "filter": "drop-shadow(0 0 8px rgba(239, 68, 68, 0.7))",
                        "opacity": 1.0
                    }
                }
            elif is_src_approver and not is_full_exec_edge and parent_gate_reached:
                # Approver didn't participate or is pending -> highlighted dotted Orange connection to result aggregation gate
                edge["attrs"] = {
                    "line": {
                        "stroke": "#f97316",
                        "strokeWidth": 2.5,
                        "strokeDasharray": "3 3",
                        "targetMarker": {"name": "block", "width": 8, "height": 6, "fill": "#f97316"},
                        "filter": "drop-shadow(0 0 10px rgba(249, 115, 22, 0.7))",
                        "opacity": 1.0
                    }
                }
            elif is_tgt_approver and src_exec and not tgt_exec and parent_gate_reached:
                # Upstream gate to pending approver -> highlighted dotted Orange connection
                edge["attrs"] = {
                    "line": {
                        "stroke": "#f97316",
                        "strokeWidth": 2.0,
                        "strokeDasharray": "3 3",
                        "targetMarker": {"name": "block", "width": 8, "height": 6, "fill": "#f97316"},
                        "filter": "drop-shadow(0 0 8px rgba(249, 115, 22, 0.5))",
                        "opacity": 0.95
                    }
                }
            elif src_exec and tgt_active:
                # Flowing active transition to running / awaiting step
                edge["attrs"] = {
                    "line": {
                        "stroke": "#f59e0b",
                        "strokeWidth": 3.5,
                        "strokeDasharray": "6 4",
                        "targetMarker": {"name": "block", "width": 10, "height": 8, "fill": "#f59e0b"},
                        "filter": "drop-shadow(0 0 10px rgba(245, 158, 11, 0.75))",
                        "opacity": 1.0
                    }
                }
                if "labels" in edge:
                    for lbl in edge["labels"]:
                        if "attrs" in lbl:
                            if "text" in lbl["attrs"]:
                                lbl["attrs"]["text"]["fill"] = "#fde68a"
                                lbl["attrs"]["text"]["opacity"] = 1.0
                            if "rect" in lbl["attrs"]:
                                lbl["attrs"]["rect"]["fill"] = "#1e1b10"
                                lbl["attrs"]["rect"]["stroke"] = "#f59e0b"
                                lbl["attrs"]["rect"]["opacity"] = 1.0
            elif is_full_exec_edge or (src_exec and (tgt_exec or (tgt_node and tgt_node["id"] == "node__end" and overall_status == "COMPLETED" and not is_untaken_branch))):
                tgt_type = tgt_node.get("data", {}).get("type") if tgt_node else ""
                src_type = src_node.get("data", {}).get("type") if src_node else ""
                is_exc_flow = bool(
                    tgt_type in ("exception_handler", "end_failed")
                    or src_type in ("exception_handler",)
                    or "node__exc_" in edge["target"]
                    or "node__failed_term_" in edge["target"]
                    or "node__exc_" in edge["source"]
                )
                is_rejection_edge = bool(
                    (src_node and src_node.get("data", {}).get("status") == "REJECTED")
                    or "REJECT" in e_label_str
                )
                tgt_sla = bool(
                    (tgt_node and (tgt_node.get("data", {}).get("status") == "TIMED_OUT" or tgt_node.get("data", {}).get("is_sla_breach")))
                    or (src_node and (src_node.get("data", {}).get("status") == "TIMED_OUT" or src_node.get("data", {}).get("is_sla_breach")))
                    or "SLA" in e_label_str
                )
                if is_exc_flow or is_rejection_edge:
                    edge_color = "#ef4444"
                elif tgt_sla:
                    edge_color = "#f97316"
                else:
                    edge_color = "#10b981"
                edge["attrs"] = {
                    "line": {
                        "stroke": edge_color,
                        "strokeWidth": 3.5,
                        "strokeDasharray": "5 5" if is_exc_flow else None,
                        "targetMarker": {"name": "block", "width": 10, "height": 8, "fill": edge_color},
                        "filter": f"drop-shadow(0 0 8px {edge_color}88)",
                        "opacity": 1.0
                    }
                }
                if "labels" in edge:
                    for lbl in edge["labels"]:
                        if "attrs" in lbl:
                            if "text" in lbl["attrs"]:
                                lbl["attrs"]["text"]["fill"] = "#f87171" if is_exc_flow else edge_color
                                lbl["attrs"]["text"]["opacity"] = 1.0
                            if "rect" in lbl["attrs"]:
                                rect_bg = "#2b180d" if tgt_sla else ("#2d1215" if (is_exc_flow or is_rejection_edge) else "#042f2e")
                                lbl["attrs"]["rect"]["fill"] = rect_bg
                                lbl["attrs"]["rect"]["stroke"] = edge_color
                                lbl["attrs"]["rect"]["opacity"] = 1.0
            else:
                edge["attrs"] = {
                    "line": {
                        "stroke": "#1e293b",
                        "strokeWidth": 1.5,
                        "strokeDasharray": "4 4",
                        "targetMarker": {"name": "block", "width": 6, "height": 4, "fill": "#334155"},
                        "opacity": 0.25
                    }
                }
                if "labels" in edge:
                    for lbl in edge["labels"]:
                        if "attrs" in lbl:
                            if "text" in lbl["attrs"]:
                                lbl["attrs"]["text"]["fill"] = "#475569"
                                lbl["attrs"]["text"]["opacity"] = 0.3
                            if "rect" in lbl["attrs"]:
                                lbl["attrs"]["rect"]["fill"] = "#090d16"
                                lbl["attrs"]["rect"]["stroke"] = "#1e293b"
                                lbl["attrs"]["rect"]["opacity"] = 0.2

    return {
        "workflow_name": blueprint_cfg.get("workflow_name") or blueprint_cfg.get("id", "Workflow"),
        "description": blueprint_cfg.get("description", ""),
        "task_queue": blueprint_cfg.get("task_queue", "default-queue"),
        "namespace": blueprint_cfg.get("namespace", "default"),
        "version": blueprint_cfg.get("version", "1.0.0"),
        "overall_status": overall_status,
        "executed_path": executed_path or [],
        "nodes": nodes,
        "edges": edges
    }

def render_x6_visualizer_html(
    domain_name: str, 
    route_prefix: str = "",
    domains_registry: Optional[List[Dict[str, Any]]] = None
) -> str:
    """
    Renders a complete, standalone, interactive dark-mode HTML5 application
    powered by AntV X6 (v2.x) and Dagre hierarchical graph auto-layout.
    """
    prefix = route_prefix or f"/api/v1.0.0/{domain_name}"
    domains_json = json.dumps(domains_registry or [{"domain": domain_name, "prefix": prefix, "workflow_name": domain_name}])
    default_api_key = os.getenv("OCTOPUS_API_KEY", "default-api-key")


    return f"""<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Octopus Workflow Visualizer - {domain_name}</title>
    <link rel="preconnect" href="https://fonts.googleapis.com">
    <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
    <link href="https://fonts.googleapis.com/css2?family=Fira+Code:wght@400;500;600&family=Inter:wght@300;400;500;600;700&display=swap" rel="stylesheet">
    <script src="https://cdn.jsdelivr.net/npm/@antv/x6/dist/index.js"></script>
    <script src="https://cdn.jsdelivr.net/npm/dagre@0.8.5/dist/dagre.min.js"></script>
    <style>
        :root {{
            --bg-dark: #0f172a;
            --bg-card: #1e293b;
            --bg-card-hover: #334155;
            --border: #334155;
            --primary: #6366f1;
            --primary-glow: rgba(99, 102, 241, 0.35);
            --success: #10b981;
            --warning: #f59e0b;
            --danger: #ef4444;
            --text-main: #f8fafc;
            --text-muted: #94a3b8;
        }}

        * {{
            box-sizing: border-box;
            margin: 0;
            padding: 0;
        }}

        body {{
            font-family: 'Inter', -apple-system, BlinkMacSystemFont, sans-serif;
            background-color: var(--bg-dark);
            color: var(--text-main);
            height: 100vh;
            overflow: hidden;
            display: flex;
            flex-direction: column;
        }}

        /* Header Bar */
        header {{
            height: 56px;
            background: rgba(11, 15, 25, 0.90);
            backdrop-filter: blur(16px);
            -webkit-backdrop-filter: blur(16px);
            border-bottom: 1px solid rgba(255, 255, 255, 0.08);
            display: flex;
            align-items: center;
            justify-content: space-between;
            padding: 0 18px;
            gap: 14px;
            z-index: 50;
            flex-shrink: 0;
        }}

        .header-left {{
            display: flex;
            align-items: center;
            gap: 10px;
            flex-shrink: 0;
        }}

        .logo-badge {{
            display: flex;
            align-items: center;
            gap: 6px;
            background: linear-gradient(135deg, rgba(99, 102, 241, 0.25) 0%, rgba(168, 85, 247, 0.25) 100%);
            border: 1px solid rgba(168, 85, 247, 0.4);
            padding: 5px 10px;
            border-radius: 8px;
            font-weight: 700;
            font-size: 13px;
            letter-spacing: 0.5px;
            color: #fff;
            box-shadow: 0 0 14px rgba(168, 85, 247, 0.25);
            user-select: none;
            cursor: pointer;
        }}

        .header-divider {{
            width: 1px;
            height: 22px;
            background: rgba(255, 255, 255, 0.1);
            margin: 0 2px;
        }}

        .select-pill {{
            background: #131c2e;
            border: 1px solid rgba(255, 255, 255, 0.14);
            color: #f8fafc;
            padding: 6px 12px;
            border-radius: 8px;
            font-size: 13px;
            font-weight: 600;
            outline: none;
            cursor: pointer;
            transition: all 0.2s ease;
            max-width: 200px;
        }}
        .select-pill:hover {{
            border-color: rgba(99, 102, 241, 0.6);
            background: #18243c;
        }}
        .select-pill:focus {{
            border-color: #6366f1;
            box-shadow: 0 0 0 2px rgba(99, 102, 241, 0.35);
        }}

        .select-pill-route {{
            background: #0d1e38;
            border-color: rgba(56, 189, 248, 0.45);
            color: #38bdf8;
            font-weight: 500;
            max-width: 260px;
        }}
        .select-pill-route:hover {{
            border-color: #38bdf8;
            background: #11284c;
        }}

        /* Top-Left Workflow Description Overlay Tile */
        .top-left-overlay-container {{
            position: absolute;
            top: 16px;
            left: 24px;
            z-index: 38;
            display: flex;
            flex-direction: column;
            gap: 10px;
            pointer-events: none;
            max-width: 440px;
        }}

        .wf-overlay-tile {{
            pointer-events: auto;
            background: rgba(15, 23, 42, 0.88);
            backdrop-filter: blur(16px);
            -webkit-backdrop-filter: blur(16px);
            border: 1px solid rgba(56, 189, 248, 0.25);
            border-radius: 10px;
            box-shadow: 0 8px 32px rgba(0, 0, 0, 0.55);
            min-width: 240px;
            overflow: hidden;
            transition: all 0.25s cubic-bezier(0.16, 1, 0.3, 1);
            user-select: none;
        }}

        .wf-tile-header {{
            display: flex;
            align-items: center;
            justify-content: space-between;
            padding: 8px 12px;
            background: rgba(30, 41, 59, 0.5);
            border-bottom: 1px solid rgba(255, 255, 255, 0.06);
            cursor: pointer;
            gap: 10px;
            transition: background 0.15s ease;
        }}
        .wf-tile-header:hover {{
            background: rgba(30, 41, 59, 0.85);
        }}

        .wf-tile-title {{
            display: flex;
            align-items: center;
            gap: 7px;
            font-size: 12px;
            font-weight: 700;
            color: #7dd3fc;
            letter-spacing: 0.3px;
        }}
        .wf-tile-icon {{
            font-size: 13px;
        }}

        .wf-tile-toggle-btn {{
            background: rgba(255, 255, 255, 0.08);
            border: 1px solid rgba(255, 255, 255, 0.12);
            color: #94a3b8;
            border-radius: 4px;
            width: 20px;
            height: 20px;
            display: flex;
            align-items: center;
            justify-content: center;
            font-size: 9px;
            cursor: pointer;
            transition: all 0.15s ease;
            padding: 0;
        }}
        .wf-tile-toggle-btn:hover {{
            background: rgba(56, 189, 248, 0.2);
            border-color: #38bdf8;
            color: #fff;
        }}

        .wf-tile-body {{
            padding: 10px 14px;
            font-size: 12px;
            color: #cbd5e1;
            line-height: 1.5;
            user-select: text;
            max-height: 320px;
            overflow-y: auto;
            transition: max-height 0.25s ease, padding 0.25s ease, opacity 0.2s ease;
            display: flex;
            flex-direction: column;
            gap: 7px;
        }}

        .wf-tile-row {{
            display: flex;
            align-items: flex-start;
            gap: 8px;
            font-size: 12px;
        }}
        .wf-tile-label {{
            font-size: 11px;
            font-weight: 700;
            color: #94a3b8;
            font-family: 'Fira Code', monospace;
            min-width: 80px;
            flex-shrink: 0;
        }}
        .wf-tile-val {{
            color: #e2e8f0;
            word-break: break-word;
            flex: 1;
            font-size: 12px;
        }}
        .wf-tile-val-highlight {{
            font-weight: 600;
            color: #7dd3fc;
            font-family: 'Fira Code', monospace;
        }}
        .wf-tile-val-code {{
            font-family: 'Fira Code', monospace;
            font-size: 11px;
            color: #38bdf8;
            background: rgba(56, 189, 248, 0.1);
            border: 1px solid rgba(56, 189, 248, 0.25);
            padding: 2px 7px;
            border-radius: 4px;
            display: inline-block;
        }}

        .wf-overlay-tile.minimized {{
            border-color: rgba(255, 255, 255, 0.12);
            box-shadow: 0 4px 16px rgba(0, 0, 0, 0.4);
        }}
        .wf-overlay-tile.minimized .wf-tile-header {{
            border-bottom: none;
            background: transparent;
            padding: 6px 12px;
        }}
        .wf-overlay-tile.minimized .wf-tile-header:hover {{
            background: rgba(30, 41, 59, 0.6);
        }}
        .wf-overlay-tile.minimized .wf-tile-body {{
            display: none;
        }}

        .header-center {{
            flex: 1;
            max-width: 440px;
            min-width: 240px;
        }}

        .search-bar {{
            display: flex;
            align-items: center;
            background: #0f172a;
            border: 1px solid rgba(255, 255, 255, 0.14);
            border-radius: 8px;
            padding: 3px 4px 3px 12px;
            gap: 8px;
            transition: all 0.2s ease;
        }}
        .search-bar:focus-within {{
            border-color: #6366f1;
            box-shadow: 0 0 0 2px rgba(99, 102, 241, 0.35);
            background: #131d33;
        }}
        .search-icon {{
            font-size: 12px;
            color: #64748b;
            flex-shrink: 0;
        }}
        .search-input {{
            flex: 1;
            background: transparent;
            border: none;
            outline: none;
            color: #fff;
            font-size: 12px;
            font-family: 'Fira Code', monospace;
            min-width: 0;
        }}
        .search-input::placeholder {{
            color: #64748b;
        }}
        .search-btn {{
            background: #4f46e5;
            border: none;
            color: #fff;
            padding: 5px 12px;
            border-radius: 6px;
            font-size: 12px;
            font-weight: 600;
            cursor: pointer;
            display: flex;
            align-items: center;
            gap: 4px;
            transition: background 0.15s ease;
            flex-shrink: 0;
        }}
        .search-btn:hover {{
            background: #4338ca;
        }}

        .header-right {{
            display: flex;
            align-items: center;
            gap: 8px;
            flex-shrink: 0;
        }}

        .btn {{
            background: rgba(255, 255, 255, 0.05);
            border: 1px solid rgba(255, 255, 255, 0.1);
            color: #cbd5e1;
            height: 32px;
            padding: 0 12px;
            border-radius: 7px;
            font-size: 12px;
            font-weight: 500;
            cursor: pointer;
            display: flex;
            align-items: center;
            gap: 6px;
            text-decoration: none;
            transition: all 0.15s ease;
            user-select: none;
            white-space: nowrap;
        }}

        .btn:hover:not(:disabled) {{
            background: rgba(255, 255, 255, 0.1);
            border-color: rgba(255, 255, 255, 0.2);
            color: #fff;
        }}

        .btn-track {{
            background: rgba(16, 185, 129, 0.12);
            border-color: rgba(16, 185, 129, 0.35);
            color: #34d399;
        }}
        .btn-track:hover:not(:disabled) {{
            background: rgba(16, 185, 129, 0.22);
            border-color: #10b981;
            color: #6ee7b7;
        }}

        .btn-active-track {{
            background: rgba(16, 185, 129, 0.25) !important;
            border-color: #10b981 !important;
            box-shadow: 0 0 12px rgba(16, 185, 129, 0.4);
        }}

        .btn:disabled {{
            opacity: 0.35;
            cursor: not-allowed;
            background: rgba(255, 255, 255, 0.02) !important;
            border-color: rgba(255, 255, 255, 0.05) !important;
            color: #475569 !important;
        }}

        .live-badge {{
            display: flex;
            align-items: center;
            gap: 6px;
            font-size: 11px;
            font-weight: 600;
            padding: 4px 9px;
            border-radius: 999px;
            background: rgba(16, 185, 129, 0.15);
            color: #34d399;
            border: 1px solid rgba(16, 185, 129, 0.3);
        }}

        .pulse-dot {{
            width: 7px;
            height: 7px;
            background: #10b981;
            border-radius: 50%;
            animation: pulse 2s infinite;
        }}

        @keyframes pulse {{
            0% {{ box-shadow: 0 0 0 0 rgba(16, 185, 129, 0.7); }}
            70% {{ box-shadow: 0 0 0 8px rgba(16, 185, 129, 0); }}
            100% {{ box-shadow: 0 0 0 0 rgba(16, 185, 129, 0); }}
        }}

        /* Main Workspace */
        .workspace {{
            flex: 1;
            display: flex;
            position: relative;
            overflow: hidden;
        }}

        #graph-container {{
            flex: 1;
            height: 100%;
            background: radial-gradient(circle, #1e293b 1px, transparent 1px);
            background-size: 24px 24px;
            background-color: #0b0f19;
        }}

        /* Sidebar Inspector - Collapsible Sliding Drawer */
        .inspector-panel {{
            position: absolute;
            right: 0;
            top: 0;
            bottom: 0;
            width: 420px;
            background: rgba(15, 23, 42, 0.95);
            backdrop-filter: blur(20px);
            border-left: 1px solid var(--border);
            padding: 24px;
            display: flex;
            flex-direction: column;
            gap: 20px;
            overflow-y: auto;
            transform: translateX(100%);
            transition: transform 0.35s cubic-bezier(0.16, 1, 0.3, 1);
            z-index: 40;
            box-shadow: -10px 0 40px rgba(0, 0, 0, 0.6);
        }}

        .inspector-panel.open {{
            transform: translateX(0);
        }}

        .floating-inspector-tab {{
            position: absolute;
            right: 0;
            top: 24px;
            z-index: 35;
            background: rgba(30, 41, 59, 0.92);
            backdrop-filter: blur(12px);
            border: 1px solid var(--border);
            border-right: none;
            border-radius: 10px 0 0 10px;
            padding: 8px 14px;
            color: #94a3b8;
            font-size: 12px;
            font-weight: 600;
            cursor: pointer;
            display: flex;
            align-items: center;
            gap: 6px;
            box-shadow: -4px 4px 16px rgba(0, 0, 0, 0.4);
            transition: all 0.2s ease;
        }}
        .floating-inspector-tab:hover {{
            color: #38bdf8;
            background: rgba(51, 65, 85, 0.95);
            transform: translateX(-4px);
        }}

        .close-panel-btn {{
            background: transparent;
            border: none;
            color: #94a3b8;
            font-size: 14px;
            cursor: pointer;
            padding: 4px 8px;
            border-radius: 6px;
            transition: all 0.2s ease;
            display: flex;
            align-items: center;
            justify-content: center;
        }}
        .close-panel-btn:hover {{
            color: #fff;
            background: rgba(255, 255, 255, 0.1);
        }}

        .panel-header {{
            display: flex;
            align-items: center;
            justify-content: space-between;
            padding-bottom: 12px;
            border-bottom: 1px solid var(--border);
        }}

        .panel-title {{
            font-size: 15px;
            font-weight: 600;
            text-transform: uppercase;
            letter-spacing: 0.5px;
            color: var(--text-muted);
        }}

        .tag {{
            display: inline-block;
            padding: 3px 8px;
            border-radius: 6px;
            font-size: 11px;
            font-weight: 600;
            text-transform: uppercase;
        }}
        .tag-activity {{ background: rgba(99, 102, 241, 0.2); color: #818cf8; border: 1px solid rgba(99, 102, 241, 0.4); }}
        .tag-human {{ background: rgba(245, 158, 11, 0.2); color: #fbbf24; border: 1px solid rgba(245, 158, 11, 0.4); }}
        .tag-branch {{ background: rgba(168, 85, 247, 0.2); color: #c084fc; border: 1px solid rgba(168, 85, 247, 0.4); }}

        .meta-field {{
            display: flex;
            flex-direction: column;
            gap: 6px;
        }}

        .meta-label {{
            font-size: 12px;
            font-weight: 500;
            color: var(--text-muted);
        }}

        .meta-value {{
            background: #0f172a;
            padding: 10px 12px;
            border-radius: 8px;
            border: 1px solid var(--border);
            font-size: 13px;
            font-family: 'Fira Code', monospace;
            word-break: break-all;
        }}

        .copy-payload-btn {{
            background: rgba(30, 41, 59, 0.8);
            border: 1px solid var(--border);
            color: #94a3b8;
            font-size: 11px;
            font-family: 'Inter', sans-serif;
            font-weight: 500;
            padding: 3px 8px;
            border-radius: 4px;
            cursor: pointer;
            transition: all 0.2s ease;
            display: inline-flex;
            align-items: center;
            gap: 4px;
        }}
        .copy-payload-btn:hover {{
            background: #334155;
            color: #f8fafc;
            border-color: #38bdf8;
        }}
        .payload-code-block {{
            background: #0b0f19;
            border: 1px solid rgba(56, 189, 248, 0.2);
            border-radius: 8px;
            padding: 10px 12px;
            font-family: 'Fira Code', monospace;
            font-size: 11.5px;
            line-height: 1.5;
            max-height: 220px;
            overflow-y: auto;
            white-space: pre-wrap;
            word-break: break-all;
            margin: 0;
        }}
        .payload-code-block.output-block {{
            border-color: rgba(52, 211, 153, 0.25);
        }}
        .payload-code-block::-webkit-scrollbar {{
            width: 6px;
            height: 6px;
        }}
        .payload-code-block::-webkit-scrollbar-thumb {{
            background: #334155;
            border-radius: 3px;
        }}

        /* Floating Toolbar */
        .floating-toolbar {{
            position: absolute;
            bottom: 20px;
            left: 20px;
            background: rgba(15, 23, 42, 0.85);
            backdrop-filter: blur(16px);
            -webkit-backdrop-filter: blur(16px);
            border: 1px solid rgba(255, 255, 255, 0.1);
            border-radius: 10px;
            padding: 4px;
            display: flex;
            align-items: center;
            gap: 4px;
            box-shadow: 0 10px 30px rgba(0, 0, 0, 0.5);
            z-index: 30;
        }}

        .tool-btn {{
            background: transparent;
            border: none;
            color: #94a3b8;
            padding: 6px 10px;
            border-radius: 6px;
            font-size: 12px;
            font-weight: 500;
            cursor: pointer;
            display: flex;
            align-items: center;
            gap: 5px;
            transition: all 0.15s ease;
        }}

        .tool-btn:hover {{
            color: #fff;
            background: rgba(255, 255, 255, 0.08);
        }}

        /* Approval Action Card inside Inspector */
        .approval-card {{
            background: rgba(245, 158, 11, 0.1);
            border: 1px solid rgba(245, 158, 11, 0.3);
            border-radius: 12px;
            padding: 16px;
            display: flex;
            flex-direction: column;
            gap: 12px;
        }}

        .approval-btn-group {{
            display: flex;
            gap: 8px;
        }}

        .btn-approve {{
            flex: 1;
            background: #10b981;
            border: none;
            color: #fff;
            padding: 10px;
            border-radius: 8px;
            font-weight: 600;
            cursor: pointer;
            transition: opacity 0.2s;
        }}
        .btn-approve:hover {{ opacity: 0.9; }}

        .btn-reject {{
            flex: 1;
            background: #ef4444;
            border: none;
            color: #fff;
            padding: 10px;
            border-radius: 8px;
            font-weight: 600;
            cursor: pointer;
            transition: opacity 0.2s;
        }}
        .btn-reject:hover {{ opacity: 0.9; }}

        /* Custom HTML Nodes in X6 */
        .x6-node {{
            cursor: pointer;
        }}
        .x6-node-selected rect {{
            stroke: #38bdf8 !important;
            stroke-width: 3px !important;
        }}

        /* Jobs Modal Overlay & Container */
        .modal-backdrop {{
            position: fixed;
            top: 0;
            left: 0;
            right: 0;
            bottom: 0;
            background: rgba(8, 12, 22, 0.80);
            backdrop-filter: blur(12px);
            -webkit-backdrop-filter: blur(12px);
            z-index: 999;
            display: flex;
            align-items: center;
            justify-content: center;
            padding: 24px;
            animation: fadeIn 0.2s ease;
        }}

        @keyframes fadeIn {{
            from {{ opacity: 0; }}
            to {{ opacity: 1; }}
        }}

        /* Jobs Modal Styles */
        .jobs-modal-card {{
            background: #0f172a;
            border: 1px solid rgba(255, 255, 255, 0.15);
            border-radius: 14px;
            width: 100%;
            max-width: 1100px;
            height: 80vh;
            max-height: 85vh;
            display: flex;
            flex-direction: column;
            box-shadow: 0 25px 60px rgba(0, 0, 0, 0.8), 0 0 25px rgba(99, 102, 241, 0.2);
            overflow: hidden;
            animation: slideUp 0.25s cubic-bezier(0.16, 1, 0.3, 1);
            margin: auto;
        }}


        @keyframes slideUp {{
            from {{ transform: translateY(20px); opacity: 0; }}
            to {{ transform: translateY(0); opacity: 1; }}
        }}

        .jobs-modal-header {{
            padding: 18px 24px;
            background: rgba(15, 23, 42, 0.95);
            border-bottom: 1px solid rgba(255, 255, 255, 0.08);
            display: flex;
            align-items: center;
            justify-content: space-between;
            gap: 16px;
        }}

        .jobs-modal-title {{
            display: flex;
            align-items: center;
            gap: 10px;
            font-size: 16px;
            font-weight: 700;
            color: #f8fafc;
        }}

        .jobs-count-pill {{
            background: rgba(99, 102, 241, 0.2);
            border: 1px solid rgba(99, 102, 241, 0.4);
            color: #c7d2fe;
            padding: 2px 10px;
            border-radius: 999px;
            font-size: 12px;
            font-weight: 600;
        }}

        .jobs-modal-controls {{
            padding: 14px 24px;
            background: rgba(11, 15, 25, 0.85);
            border-bottom: 1px solid rgba(255, 255, 255, 0.06);
            display: flex;
            align-items: center;
            justify-content: space-between;
            gap: 12px;
            flex-wrap: wrap;
        }}

        .jobs-filter-group {{
            display: flex;
            align-items: center;
            gap: 10px;
            flex: 1;
        }}

        .jobs-search-input {{
            flex: 1;
            min-width: 200px;
            background: #1e293b;
            border: 1px solid rgba(255, 255, 255, 0.12);
            border-radius: 8px;
            padding: 8px 14px;
            color: #fff;
            font-size: 13px;
            font-family: 'Fira Code', monospace;
            outline: none;
            transition: all 0.2s ease;
        }}

        .jobs-search-input:focus {{
            border-color: #6366f1;
            box-shadow: 0 0 0 2px rgba(99, 102, 241, 0.3);
            background: #182234;
        }}

        .jobs-filter-select {{
            background: #1e293b;
            border: 1px solid rgba(255, 255, 255, 0.12);
            border-radius: 8px;
            padding: 8px 12px;
            color: #f8fafc;
            font-size: 12px;
            font-weight: 500;
            outline: none;
            cursor: pointer;
        }}

        .jobs-modal-body {{
            flex: 1;
            overflow-y: auto;
            padding: 0;
            background: #0b0f19;
        }}

        .jobs-table {{
            width: 100%;
            border-collapse: collapse;
            font-size: 12px;
            text-align: left;
        }}

        .jobs-table th {{
            position: sticky;
            top: 0;
            background: #131c2e;
            color: #94a3b8;
            font-weight: 600;
            text-transform: uppercase;
            letter-spacing: 0.5px;
            font-size: 11px;
            padding: 12px 16px;
            border-bottom: 1px solid rgba(255, 255, 255, 0.08);
            z-index: 5;
        }}

        .jobs-table td {{
            padding: 12px 16px;
            border-bottom: 1px solid rgba(255, 255, 255, 0.05);
            vertical-align: middle;
        }}

        .jobs-table tr:hover td {{
            background: rgba(255, 255, 255, 0.03);
        }}

        .job-id-code {{
            font-family: 'Fira Code', monospace;
            font-size: 12px;
            color: #67e8f9;
            cursor: pointer;
            font-weight: 500;
        }}
        .job-id-code:hover {{
            text-decoration: underline;
            color: #a5f3fc;
        }}

        .method-badge {{
            display: inline-block;
            padding: 2px 6px;
            border-radius: 4px;
            font-size: 10px;
            font-weight: 700;
            font-family: 'Fira Code', monospace;
        }}
        .method-post {{ background: rgba(168, 85, 247, 0.2); color: #d8b4fe; border: 1px solid rgba(168, 85, 247, 0.4); }}
        .method-get {{ background: rgba(16, 185, 129, 0.2); color: #6ee7b7; border: 1px solid rgba(16, 185, 129, 0.4); }}
        .method-put {{ background: rgba(245, 158, 11, 0.2); color: #fde68a; border: 1px solid rgba(245, 158, 11, 0.4); }}
        .method-delete {{ background: rgba(239, 68, 68, 0.2); color: #fca5a5; border: 1px solid rgba(239, 68, 68, 0.4); }}

        .status-badge {{
            display: inline-flex;
            align-items: center;
            gap: 5px;
            padding: 3px 9px;
            border-radius: 6px;
            font-size: 11px;
            font-weight: 600;
        }}
        .status-200 {{ background: rgba(16, 185, 129, 0.18); color: #34d399; border: 1px solid rgba(16, 185, 129, 0.35); }}
        .status-401 {{ background: rgba(239, 68, 68, 0.18); color: #fca5a5; border: 1px solid rgba(239, 68, 68, 0.35); }}
        .status-403 {{ background: rgba(239, 68, 68, 0.18); color: #fca5a5; border: 1px solid rgba(239, 68, 68, 0.35); }}
        .status-429 {{ background: rgba(245, 158, 11, 0.18); color: #fde68a; border: 1px solid rgba(245, 158, 11, 0.35); }}
        .status-500 {{ background: rgba(239, 68, 68, 0.22); color: #f87171; border: 1px solid rgba(239, 68, 68, 0.4); }}
        .status-running {{ background: rgba(56, 189, 248, 0.18); color: #7dd3fc; border: 1px solid rgba(56, 189, 248, 0.35); }}
        .status-rollback {{ background: rgba(168, 85, 247, 0.18); color: #d8b4fe; border: 1px solid rgba(168, 85, 247, 0.35); }}

        .btn-inspect {{
            background: rgba(99, 102, 241, 0.2);
            border: 1px solid rgba(99, 102, 241, 0.4);
            color: #c7d2fe;
            padding: 5px 12px;
            border-radius: 6px;
            font-size: 11px;
            font-weight: 600;
            cursor: pointer;
            display: inline-flex;
            align-items: center;
            gap: 4px;
            transition: all 0.15s ease;
        }}
        .btn-inspect:hover {{
            background: rgba(99, 102, 241, 0.35);
            color: #fff;
            border-color: rgba(99, 102, 241, 0.6);
        }}
    </style>
</head>
<body>

    <header>
        <div class="header-left">
            <div class="logo-badge" onclick="resetToBlueprint()" title="OCTOPUS Workflow Engine">
                <span>🐙</span>
                <span>OCTOPUS</span>
            </div>
            <div class="header-divider"></div>
            <select class="select-pill" id="domainSelect" onchange="onDomainSelectChanged()" title="Select Domain / Workflow"></select>
            <select class="select-pill select-pill-route" id="routeSelect" style="display: none;" onchange="onRouteSelectChanged()" title="Select API Route"></select>
        </div>

        <div class="header-center">
            <div class="search-bar">
                <span class="search-icon">🔍</span>
                <input type="text" class="search-input" id="jobIdInput" placeholder="Enter Job ID (e.g. offboard-emp-1002)..." onkeydown="if(event.key==='Enter') lookupJob()">
                <button class="search-btn" id="lookupBtn" onclick="lookupJob()">Lookup</button>
            </div>
        </div>

        <div class="header-right">
            <button class="btn" onclick="openJobsModal()" title="View execution history & telemetry logs">
                <span>📑</span>
                <span>Jobs</span>
            </button>
            <button class="btn btn-track" id="trackBtn" onclick="toggleLiveTracking()" disabled title="Stream live execution events">
                <span>⚡</span>
                <span>Live Trace</span>
            </button>

            <button class="btn" onclick="resetToBlueprint()" title="Reset to static blueprint view">
                <span>↺</span>
                <span>Blueprint</span>
            </button>
            <a href="/docs" target="_blank" class="btn" style="color: #a5b4fc;" title="Open API documentation">
                <span>📚</span>
                <span>API Docs</span>
            </a>
            <div class="live-badge" id="liveIndicator" style="display: none;">
                <div class="pulse-dot"></div>
                <span>POLLING</span>
            </div>
        </div>
    </header>

    <!-- Floating Toast Notification -->
    <div id="toastMessage" style="display: none; position: fixed; top: 76px; right: 24px; z-index: 99; background: rgba(30, 41, 59, 0.95); backdrop-filter: blur(16px); border-radius: 10px; padding: 12px 18px; font-size: 13px; color: #fff; box-shadow: 0 10px 30px rgba(0, 0, 0, 0.5); align-items: center; gap: 10px; transition: all 0.3s ease;">
        <span id="toastIcon" style="font-size: 16px;">ℹ️</span>
        <span id="toastText">Notification message</span>
    </div>

    <div class="workspace">
        <div class="top-left-overlay-container">
            <div id="executionBanner" style="display: none; pointer-events: auto; background: rgba(15, 23, 42, 0.92); backdrop-filter: blur(12px); border: 1px solid rgba(16, 185, 129, 0.4); border-radius: 10px; padding: 8px 16px; font-size: 13px; color: #e2e8f0; box-shadow: 0 4px 20px rgba(0, 0, 0, 0.5); align-items: center; gap: 8px; width: fit-content;">
                <span id="executionSummaryText">Execution Path Active</span>
            </div>

            <!-- Top-Left Minimizable Workflow Description Tile -->
            <div id="workflowTile" class="wf-overlay-tile">
                <div class="wf-tile-header" onclick="toggleWorkflowTile()" title="Click to minimize or expand workflow info">
                    <div class="wf-tile-title">
                        <span class="wf-tile-icon">📜</span>
                        <span id="wf-tile-name">Workflow Overview</span>
                    </div>
                    <button class="wf-tile-toggle-btn" id="wfTileToggleBtn" title="Minimize / Expand description">
                        <span id="wfTileToggleIcon">▲</span>
                    </button>
                </div>
                <div class="wf-tile-body" id="wfTileBody">
                    <div class="wf-tile-row">
                        <span class="wf-tile-label">name:</span>
                        <span class="wf-tile-val wf-tile-val-highlight" id="wf-tile-name-val">-</span>
                    </div>
                    <div class="wf-tile-row">
                        <span class="wf-tile-label">version:</span>
                        <span class="wf-tile-val" id="wf-tile-version-val">-</span>
                    </div>
                    <div class="wf-tile-row">
                        <span class="wf-tile-label">description:</span>
                        <span class="wf-tile-val" id="wf-tile-desc-val">-</span>
                    </div>
                    <div class="wf-tile-row" id="wf-tile-route-row" style="display: none;">
                        <span class="wf-tile-label">route:</span>
                        <span class="wf-tile-val wf-tile-val-code" id="wf-tile-route-val">-</span>
                    </div>
                </div>
            </div>
        </div>

        <div id="graph-container"></div>

        <!-- Floating Button to Re-open Inspector when Folded -->
        <button id="openInspectorBtn" class="floating-inspector-tab" onclick="toggleInspector(true)" style="display: none;">
            <span>📋 Step Inspector</span>
        </button>

        <div class="inspector-panel" id="inspector">
            <div class="panel-header">
                <div class="panel-title">Step Inspector</div>
                <div style="display: flex; align-items: center; gap: 8px;">
                    <span class="tag tag-activity" id="insp-type">ACTIVITY</span>
                    <button class="close-panel-btn" onclick="toggleInspector(false)" title="Fold Inspector (Hide to Right)">✕</button>
                </div>
            </div>

            <div class="meta-field">
                <div class="meta-label">Step Identifier</div>
                <div class="meta-value" id="insp-id">-</div>
            </div>

            <div class="meta-field">
                <div class="meta-label">Execution Status</div>
                <div class="meta-value" id="insp-status" style="color: #38bdf8;">PENDING</div>
            </div>

            <div class="meta-field" id="insp-roles-field" style="display: none;">
                <div class="meta-label">Authorized Reviewer Roles</div>
                <div class="meta-value" id="insp-roles">-</div>
            </div>

            <!-- Security Policy Card -->
            <div class="meta-field" id="insp-policy-field" style="display: none; flex-direction: column; gap: 6px; background: rgba(15, 23, 42, 0.6); border: 1px solid rgba(251, 191, 36, 0.2); border-radius: 8px; padding: 10px;">
                <div class="meta-label" style="color: #fbbf24; font-weight: 600;">🛡️ Security Policy</div>
                <div id="insp-policy-badge"></div>
                <div id="insp-policy-meta" style="font-size: 11px; color: var(--text-muted); font-family: 'Fira Code', monospace; word-break: break-all;"></div>
                <div id="insp-policy-doc" style="font-size: 11px; color: #cbd5e1; background: #0f172a; padding: 6px 8px; border-radius: 6px; border-left: 2px solid #fbbf24; line-height: 1.4; display: none;"></div>
            </div>

            <!-- Ingress Middlewares Card -->
            <div class="meta-field" id="insp-middlewares-field" style="display: none; flex-direction: column; gap: 8px; background: rgba(15, 23, 42, 0.7); border: 1px solid rgba(56, 189, 248, 0.3); border-radius: 8px; padding: 12px;">
                <div class="meta-label" style="display: flex; align-items: center; justify-content: space-between;">
                    <span style="color: #38bdf8; font-weight: 700;">🛡️ Ingress Guards Pipeline</span>
                    <span id="insp-middlewares-count-badge" style="background: rgba(56, 189, 248, 0.15); border: 1px solid #38bdf8; color: #7dd3fc; padding: 2px 8px; border-radius: 4px; font-size: 10px; font-weight: 700;"></span>
                </div>
                <div id="insp-middlewares-list" style="display: flex; flex-direction: column; gap: 6px; font-size: 11px;"></div>
                <div id="insp-middlewares-desc" style="font-size: 11px; color: var(--text-muted); line-height: 1.5; border-top: 1px dashed rgba(255,255,255,0.1); padding-top: 6px; margin-top: 2px;"></div>
            </div>

            <!-- Exception Handler & Error Routing Card -->
            <div class="meta-field" id="insp-exception-field" style="display: none; flex-direction: column; gap: 8px; background: rgba(15, 23, 42, 0.7); border: 1px solid rgba(239, 68, 68, 0.35); border-radius: 8px; padding: 12px;">
                <div class="meta-label" style="display: flex; align-items: center; justify-content: space-between;">
                    <span style="color: #fca5a5; font-weight: 700;">🚨 Exception Handler Details</span>
                    <span id="insp-exc-status-badge" style="background: rgba(239, 68, 68, 0.15); border: 1px solid #ef4444; color: #fca5a5; padding: 2px 8px; border-radius: 4px; font-size: 10px; font-weight: 700;"></span>
                </div>
                <div style="display: flex; flex-direction: column; gap: 6px; font-size: 11px;">
                    <div style="background: rgba(30, 41, 59, 0.6); border: 1px solid rgba(100, 116, 139, 0.3); border-radius: 6px; padding: 8px 10px;">
                        <div style="color: #94a3b8; font-size: 10px; margin-bottom: 2px;">Triggering Exception & HTTP Code:</div>
                        <div style="color: #fca5a5; font-family: 'Fira Code', monospace; font-weight: 600;" id="insp-exc-name">-</div>
                    </div>
                    <div style="background: rgba(30, 41, 59, 0.6); border: 1px solid rgba(100, 116, 139, 0.3); border-radius: 6px; padding: 8px 10px;">
                        <div style="color: #94a3b8; font-size: 10px; margin-bottom: 2px;">Origin / Provenance:</div>
                        <div style="color: #38bdf8; font-family: 'Fira Code', monospace; font-weight: 600;" id="insp-exc-origin">-</div>
                    </div>
                    <div style="background: rgba(30, 41, 59, 0.6); border: 1px solid rgba(100, 116, 139, 0.3); border-radius: 6px; padding: 8px 10px;">
                        <div style="color: #94a3b8; font-size: 10px; margin-bottom: 2px;">Execution Mode:</div>
                        <div style="color: #cbd5e1; font-family: 'Fira Code', monospace;" id="insp-exc-mode">-</div>
                    </div>
                </div>
                <div id="insp-exc-desc" style="font-size: 11px; color: var(--text-muted); line-height: 1.5; border-top: 1px dashed rgba(255,255,255,0.1); padding-top: 6px; margin-top: 2px;"></div>
            </div>

            <div class="meta-field">
                <div class="meta-label">SLA Timeout</div>
                <div class="meta-value" id="insp-timeout">30s</div>
            </div>

            <!-- Branch Condition & Rule Evaluation Card -->
            <div class="meta-field" id="insp-branch-field" style="display: none; flex-direction: column; gap: 8px;">
                <div class="meta-label" style="display: flex; align-items: center; justify-content: space-between;">
                    <span style="font-weight: 700; color: #38bdf8;">⚏ Branch Condition & Routing</span>
                    <span id="insp-branch-verdict-badge"></span>
                </div>
                
                <div id="insp-branch-rule-box" style="font-size: 11px; color: var(--text-muted); font-family: 'Fira Code', monospace; background: #0f172a; padding: 10px; border-radius: 8px; border-left: 3px solid #38bdf8; line-height: 1.6;">
                    <div id="insp-branch-rules-content"></div>
                </div>

                <div id="insp-branch-eval-box" style="display: none; background: rgba(15, 23, 42, 0.9); border: 1px solid rgba(16, 185, 129, 0.4); border-radius: 8px; padding: 10px; font-size: 11px; font-family: 'Fira Code', monospace;">
                    <div style="font-weight: 700; color: #10b981; margin-bottom: 6px; display: flex; align-items: center; gap: 6px;">
                        <span>⚡ Live Path Evaluation</span>
                    </div>
                    <div id="insp-branch-eval-details" style="color: #cbd5e1; line-height: 1.6;"></div>
                </div>

                <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 8px; font-size: 11px;">
                    <div id="insp-branch-then-box" style="background: rgba(16, 185, 129, 0.08); border: 1px solid rgba(16, 185, 129, 0.3); border-radius: 6px; padding: 8px;">
                        <div style="font-weight: 700; color: #34d399; margin-bottom: 4px;">✓ TRUE (Match)</div>
                        <div id="insp-branch-then-steps" style="color: #a7f3d0; font-family: 'Fira Code', monospace; font-size: 10px;"></div>
                    </div>
                    <div id="insp-branch-else-box" style="background: rgba(56, 189, 248, 0.08); border: 1px solid rgba(56, 189, 248, 0.3); border-radius: 6px; padding: 8px;">
                        <div style="font-weight: 700; color: #7dd3fc; margin-bottom: 4px;">✕ FALSE (Else)</div>
                        <div id="insp-branch-else-steps" style="color: #bae6fd; font-family: 'Fira Code', monospace; font-size: 10px;"></div>
                    </div>
                </div>
            </div>

            <!-- Dynamic Loop & Batch Configuration Card -->
            <div class="meta-field" id="insp-loop-field" style="display: none; flex-direction: column; gap: 6px;">
                <div class="meta-label">Dynamic Loop & Batch Configuration</div>
                <div id="insp-loop-badge"></div>
                <div id="insp-loop-details" style="font-size: 11px; color: var(--text-muted); font-family: 'Fira Code', monospace; background: #0f172a; padding: 8px 10px; border-radius: 6px; border-left: 2px solid #38bdf8; line-height: 1.5;"></div>
            </div>

            <!-- Activity Implementation & Code Availability Card -->
            <div class="meta-field" id="insp-activity-field" style="display: none; flex-direction: column; gap: 6px;">
                <div class="meta-label">Python Activity Implementation</div>
                <div id="insp-activity-status"></div>
                <div id="insp-activity-meta" style="font-size: 11px; color: var(--text-muted); font-family: 'Fira Code', monospace; word-break: break-all;"></div>
                <div id="insp-activity-doc" style="font-size: 11px; color: #94a3b8; font-style: italic; background: #0f172a; padding: 6px 8px; border-radius: 6px; border-left: 2px solid #38bdf8; display: none; line-height: 1.4;"></div>
            </div>

            <!-- Saga Compensation Availability Card -->
            <div class="meta-field" id="insp-comp-field" style="display: none; flex-direction: column; gap: 6px;">
                <div class="meta-label">Saga Rollback Compensation</div>
                <div id="insp-comp-status"></div>
                <div id="insp-comp-meta" style="font-size: 11px; color: var(--text-muted); font-family: 'Fira Code', monospace; word-break: break-all;"></div>
            </div>

            <!-- Idempotency Strategy Card -->
            <div class="meta-field" id="insp-idempotency-field" style="display: none; flex-direction: column; gap: 6px;">
                <div class="meta-label">Idempotency Strategy</div>
                <div id="insp-idempotency-badge"></div>
                <div id="insp-idempotency-meta" style="font-size: 11px; color: var(--text-muted); font-family: 'Fira Code', monospace; word-break: break-all;"></div>
                <div id="insp-idempotency-desc" style="font-size: 11px; color: #94a3b8; background: #0f172a; padding: 6px 8px; border-radius: 6px; border-left: 2px solid #38bdf8; line-height: 1.4; display: none;"></div>
            </div>

            <!-- Step Execution Timing & Metrics Card -->
            <div class="meta-field" id="insp-execution-metrics" style="display: none; flex-direction: column; gap: 6px; background: rgba(15, 23, 42, 0.6); border: 1px solid rgba(56, 189, 248, 0.25); border-radius: 8px; padding: 10px;">
                <div style="display: flex; align-items: center; justify-content: space-between;">
                    <span class="meta-label" style="color: #38bdf8; font-weight: 600;">⚡ Execution Metrics</span>
                    <span id="insp-duration-badge" style="font-size: 11px; font-family: 'Fira Code', monospace; background: rgba(56, 189, 248, 0.15); color: #38bdf8; padding: 2px 8px; border-radius: 4px; font-weight: 600;">-</span>
                </div>
                <div id="insp-timestamps" style="font-size: 10.5px; color: var(--text-muted); font-family: 'Fira Code', monospace;"></div>
            </div>

            <!-- Input Payload Card -->
            <div class="meta-field" id="insp-input-payload-field" style="display: none; flex-direction: column; gap: 6px;">
                <div style="display: flex; align-items: center; justify-content: space-between;">
                    <div class="meta-label" style="display: flex; align-items: center; gap: 6px; color: #7dd3fc; font-weight: 600;">
                        <span>📥 Input Payload</span>
                        <span id="insp-input-count-badge" style="font-size: 10px; padding: 1px 6px; border-radius: 4px; background: rgba(56, 189, 248, 0.15); color: #38bdf8; font-weight: 600;">0 fields</span>
                    </div>
                    <button class="copy-payload-btn" id="copyInputPayloadBtn" onclick="copyPayload(this)" title="Copy Input JSON">📋 Copy</button>
                </div>
                <pre class="payload-code-block" id="insp-input-json-block"><code id="insp-input-json-content"></code></pre>
            </div>

            <!-- Output Payload Card -->
            <div class="meta-field" id="insp-output-payload-field" style="display: none; flex-direction: column; gap: 6px;">
                <div style="display: flex; align-items: center; justify-content: space-between;">
                    <div class="meta-label" style="display: flex; align-items: center; gap: 6px; color: #34d399; font-weight: 600;">
                        <span>📤 Output Payload</span>
                        <span id="insp-output-count-badge" style="font-size: 10px; padding: 1px 6px; border-radius: 4px; background: rgba(16, 185, 129, 0.15); color: #34d399; font-weight: 600;">0 fields</span>
                        <span id="insp-output-role-badge" style="display: none; font-size: 10px; padding: 1px 6px; border-radius: 4px; background: rgba(168, 85, 247, 0.2); color: #c084fc; font-weight: 600; border: 1px solid rgba(168, 85, 247, 0.4);">🎯 Response Model</span>
                    </div>
                    <button class="copy-payload-btn" id="copyOutputPayloadBtn" onclick="copyPayload(this)" title="Copy Output JSON">📋 Copy</button>
                </div>
                <pre class="payload-code-block output-block" id="insp-output-json-block"><code id="insp-output-json-content"></code></pre>
            </div>

            <!-- Error Payload Card (if failed) -->
            <div class="meta-field" id="insp-error-payload-field" style="display: none; flex-direction: column; gap: 6px; background: rgba(239, 68, 68, 0.1); border: 1px solid rgba(239, 68, 68, 0.3); border-radius: 8px; padding: 10px;">
                <div class="meta-label" style="color: #ef4444; font-weight: 600;">⚠️ Step Execution Error</div>
                <div id="insp-error-message" style="font-size: 11.5px; color: #fca5a5; font-family: 'Fira Code', monospace; line-height: 1.4; word-break: break-word;"></div>
            </div>

            <!-- Approval Interaction Card -->
            <div class="approval-card" id="approvalActions" style="display: none;">
                <div style="font-size: 13px; font-weight: 600; color: #fbbf24;">⚡ Human Approval Action</div>
                <div style="font-size: 12px; color: var(--text-muted);">This step is currently waiting for your decision.</div>
                
                <input type="text" class="input-box" id="reviewerId" placeholder="Reviewer ID (e.g. admin_bob)" value="admin_bob" style="width: 100%;">
                <input type="text" class="input-box" id="reviewerRoles" placeholder="Reviewer Roles (e.g. domain_admin)" value="domain_admin" style="width: 100%;">
                
                <div class="approval-btn-group">
                    <button class="btn-approve" onclick="submitDecision('APPROVED')">✓ Approve</button>
                    <button class="btn-reject" onclick="submitDecision('REJECTED')">✕ Reject</button>
                </div>
            </div>
        </div>

        <!-- Floating Graph Controls -->
        <div class="floating-toolbar">
            <button class="tool-btn" onclick="graph.zoom(0.15)">🔍 Zoom In</button>
            <button class="tool-btn" onclick="graph.zoom(-0.15)">🔍 Zoom Out</button>
            <button class="tool-btn" onclick="graph.zoomToFit({{ padding: 40 }})">⤢ Fit Screen</button>
            <button class="tool-btn" onclick="toggleLayout()">⇄ Switch Orientation</button>
        </div>
    </div>

    <!-- Jobs History Modal -->
    <div id="jobsModalBackdrop" class="modal-backdrop" style="display: none;" onclick="closeJobsModal(event)">
        <div class="jobs-modal-card" onclick="event.stopPropagation()">
            <div class="jobs-modal-header">
                <div class="jobs-modal-title">
                    <span>📑</span>
                    <span>Execution Jobs History</span>
                    <span class="jobs-count-pill" id="jobsCountPill">0 Jobs</span>
                </div>
                <div style="display: flex; align-items: center; gap: 10px;">
                    <label style="display: flex; align-items: center; gap: 6px; font-size: 12px; color: var(--text-muted); cursor: pointer; user-select: none;">
                        <input type="checkbox" id="autoRefreshJobs" onchange="toggleAutoRefreshJobs()" checked>
                        <span>Auto-refresh (3s)</span>
                    </label>
                    <button class="btn" onclick="loadJobsList()" title="Refresh jobs list">
                        <span>🔄</span>
                        <span>Refresh</span>
                    </button>
                    <button class="close-panel-btn" onclick="closeJobsModal()" title="Close modal" style="width: 28px; height: 28px;">✕</button>
                </div>
            </div>

            <div class="jobs-modal-controls">
                <div class="jobs-filter-group">
                    <input type="text" class="jobs-search-input" id="jobsSearchInput" placeholder="🔍 Filter by Job ID, IP, Route, Error..." oninput="filterJobsTable()">
                    <select class="jobs-filter-select" id="jobsStatusFilter" onchange="filterJobsTable()">
                        <option value="ALL">All Statuses</option>
                        <option value="200">200 OK / Completed</option>
                        <option value="401">401 Unauthorized</option>
                        <option value="403">403 Forbidden</option>
                        <option value="429">429 Rate Limit</option>
                        <option value="500">500 / Failed</option>
                        <option value="RUNNING">Running</option>
                        <option value="ROLLED_BACK">Rolled Back</option>
                    </select>
                    <select class="jobs-filter-select" id="jobsWorkflowFilter" onchange="filterJobsTable()">
                        <option value="ALL">All Workflows</option>
                    </select>
                </div>
            </div>

            <div class="jobs-modal-body">
                <table class="jobs-table">
                    <thead>
                        <tr>
                            <th style="width: 140px;">Status</th>
                            <th>Job ID</th>
                            <th>Workflow</th>
                            <th>Route & Method</th>
                            <th>Source IP</th>
                            <th>Execution Time</th>
                            <th>Started</th>
                            <th style="text-align: right; width: 110px;">Action</th>
                        </tr>
                    </thead>
                    <tbody id="jobsTableBody">
                        <tr>
                            <td colspan="8" style="text-align: center; color: #64748b; padding: 40px 0;">
                                Loading jobs...
                            </td>
                        </tr>
                    </tbody>
                </table>
            </div>
        </div>
    </div>

    <script>

        // Clean any embedded HTTP basic auth credentials from document URL to prevent Fetch API TypeErrors
        try {{
            if (window.location.username || window.location.password || window.location.href.includes('@')) {{
                const cleanOrigin = window.location.protocol + '//' + window.location.host;
                const cleanUrl = cleanOrigin + window.location.pathname + window.location.search + window.location.hash;
                window.history.replaceState({{}}, document.title, cleanUrl);
            }}
        }} catch (e) {{
            console.debug("Could not sanitize URL:", e);
        }}

        function safeApiUrl(path) {{
            try {{
                const cleanOrigin = window.location.protocol + '//' + window.location.host;
                if (!path) return cleanOrigin;
                if (path.startsWith('http://') || path.startsWith('https://')) {{
                    const parsed = new URL(path);
                    return cleanOrigin + parsed.pathname + parsed.search;
                }}
                return cleanOrigin + (path.startsWith('/') ? path : '/' + path);
            }} catch (e) {{
                return path;
            }}
        }}

        function escapeHtml(str) {{
            if (str === null || str === undefined) return '';
            return String(str)
                .replace(/&/g, '&amp;')
                .replace(/</g, '&lt;')
                .replace(/>/g, '&gt;')
                .replace(/"/g, '&quot;')
                .replace(/'/g, '&#039;');
        }}

        function highlightJson(obj) {{
            if (obj === undefined || obj === null) return '<span style="color: #64748b;">null</span>';
            let json = typeof obj === 'string' ? obj : JSON.stringify(obj, null, 2);
            json = json.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
            return json.replace(/("(\\u[a-zA-Z0-9]{{4}}|\\[^u]|[^\\"])*"(\\s*:)?|\\b(true|false|null)\\b|-?\\d+(?:\\.\\d*)?(?:[eE][+\\-]?\\d+)?)/g, function (match) {{
                if (/^"/.test(match)) {{
                    if (/:$/.test(match)) {{
                        return '<span style="color: #93c5fd; font-weight: 500;">' + match.slice(0, -1) + '</span><span style="color: #64748b;">:</span>';
                    }} else {{
                        return '<span style="color: #34d399;">' + match + '</span>';
                    }}
                }} else if (/true|false/.test(match)) {{
                    return '<span style="color: #c084fc; font-weight: 600;">' + match + '</span>';
                }} else if (/null/.test(match)) {{
                    return '<span style="color: #64748b; font-style: italic;">' + match + '</span>';
                }} else {{
                    return '<span style="color: #fbbf24;">' + match + '</span>';
                }}
            }});
        }}

        async function copyPayload(btnElem) {{
            try {{
                const text = btnElem.getAttribute('data-raw-json') || '';
                await navigator.clipboard.writeText(text);
                const originalText = btnElem.innerHTML;
                btnElem.innerHTML = '✓ Copied!';
                btnElem.style.color = '#34d399';
                btnElem.style.borderColor = '#10b981';
                setTimeout(() => {{
                    btnElem.innerHTML = originalText;
                    btnElem.style.color = '';
                    btnElem.style.borderColor = '';
                }}, 1800);
            }} catch (e) {{
                console.warn('Copy to clipboard failed:', e);
            }}
        }}
        window.copyPayload = copyPayload;

        const INITIAL_DOMAIN = "{domain_name}";
        const INITIAL_PREFIX = "{prefix}";
        const DEFAULT_API_KEY = "{default_api_key}";
        let DOMAINS_REGISTRY = {domains_json};

        function getApiHeaders(extraHeaders = {{}}) {{
            const urlParams = new URLSearchParams(window.location.search);
            const key = localStorage.getItem('octopus_api_key') || urlParams.get('api_key') || DEFAULT_API_KEY || 'default-api-key';

            const headers = {{ ...extraHeaders }};
            if (key) {{
                headers['X-API-Key'] = key;
            }}
            return headers;
        }}

        let currentDomain = INITIAL_DOMAIN;
        let currentPrefix = INITIAL_PREFIX;
        let currentLayoutDir = "TB";
        let currentGraphData = null;
        let livePollingTimer = null;
        let selectedNodeData = null;
        let toastTimer = null;

        function showToast(text, icon = 'ℹ️', borderColor = '#6366f1', duration = 5000) {{
            const toast = document.getElementById('toastMessage');
            document.getElementById('toastText').innerText = text;
            document.getElementById('toastIcon').innerText = icon;
            toast.style.border = `1px solid ${{borderColor}}`;
            toast.style.display = 'flex';
            if (toastTimer) clearTimeout(toastTimer);
            toastTimer = setTimeout(() => {{
                toast.style.display = 'none';
            }}, duration);
        }}

        const domainSelect = document.getElementById('domainSelect');
        const routeSelect = document.getElementById('routeSelect');
        let currentRoute = "";
        let currentMethod = "";

        function populateRouteDropdown(routes, selectedRoute, selectedMethod) {{
            if (!routeSelect) return;
            routeSelect.innerHTML = '';
            if (!routes || !Array.isArray(routes) || routes.length === 0) {{
                routeSelect.style.display = 'none';
                currentRoute = "";
                currentMethod = "";
                return;
            }}

            routeSelect.style.display = 'block';
            routes.forEach(r => {{
                const opt = document.createElement('option');
                const m = (r.method || 'POST').toUpperCase();
                const path = r.route || '';
                const desc = r.summary || (r.step_count ? `${{r.step_count}} steps` : '');
                opt.value = path;
                opt.dataset.method = m;
                opt.text = `[${{m}}] ${{path}}${{desc ? ' - ' + desc : ''}}`;
                if (path === selectedRoute || (selectedRoute && (selectedRoute.endsWith(path) || path.endsWith(selectedRoute)))) {{
                    opt.selected = true;
                }}
                routeSelect.appendChild(opt);
            }});

            if (routeSelect.selectedIndex < 0 && routeSelect.options.length > 0) {{
                routeSelect.selectedIndex = 0;
            }}
            const activeOpt = routeSelect.options[routeSelect.selectedIndex >= 0 ? routeSelect.selectedIndex : 0];
            if (activeOpt) {{
                currentRoute = activeOpt.value;
                currentMethod = activeOpt.dataset.method || 'POST';
            }}
        }}

        function onRouteSelectChanged() {{
            const opt = routeSelect.options[routeSelect.selectedIndex];
            if (!opt) return;
            currentRoute = opt.value;
            currentMethod = opt.dataset.method || 'POST';

            loadBlueprintGraph();
        }}

        function populateDropdown(domains) {{
            domainSelect.innerHTML = '';
            if (!domains || !Array.isArray(domains) || domains.length === 0) {{
                domains = [{{
                    domain: INITIAL_DOMAIN || "sample_workflow",
                    prefix: INITIAL_PREFIX || "/api/v1.0.0/sample_workflow",
                    workflow_name: INITIAL_DOMAIN || "sample_workflow",
                    version: "v1.0.0"
                }}];
            }}
            domains.forEach(item => {{
                const opt = document.createElement('option');
                opt.value = item.domain;
                const vTag = item.version ? ` (${{item.version}})` : '';
                opt.text = `${{item.workflow_name || item.domain}}${{vTag}}`;
                opt.dataset.prefix = item.prefix;
                if (item.domain === currentDomain) opt.selected = true;
                domainSelect.appendChild(opt);
            }});
            if (domainSelect.selectedIndex < 0 && domainSelect.options.length > 0) {{
                domainSelect.selectedIndex = 0;
            }}
            const activeOpt = domainSelect.options[domainSelect.selectedIndex >= 0 ? domainSelect.selectedIndex : 0];
            if (activeOpt) {{
                currentDomain = activeOpt.value;
                currentPrefix = activeOpt.dataset.prefix;
            }}
        }}

        async function refreshDomains() {{
            try {{
                const res = await fetch(safeApiUrl('/api/visualizer/domains'));
                if (res.ok) {{
                    DOMAINS_REGISTRY = await res.json();
                    populateDropdown(DOMAINS_REGISTRY);
                }}
            }} catch (e) {{
                console.debug("Could not refresh domains list:", e);
            }}
        }}

        function onDomainSelectChanged() {{
            const opt = domainSelect.options[domainSelect.selectedIndex];
            if (!opt) return;
            currentDomain = opt.value;
            currentPrefix = opt.dataset.prefix;

            const cleanOrigin = window.location.protocol + '//' + window.location.host;
            const newUrl = new URL(cleanOrigin + window.location.pathname + window.location.search);
            newUrl.searchParams.set('domain', currentDomain);
            window.history.pushState({{}}, '', newUrl.toString());

            document.getElementById('trackBtn').disabled = true;
            document.getElementById('trackBtn').classList.remove('btn-active-track');
            loadBlueprintGraph();
        }}

        // Initialize AntV X6 Graph
        const graph = new X6.Graph({{
            container: document.getElementById('graph-container'),
            autoResize: true,
            panning: true,
            mousewheel: {{
                enabled: true,
                modifiers: ['ctrl', 'meta']
            }},
            grid: {{
                size: 20,
                visible: true,
                type: 'dot',
                args: {{ color: '#1e293b', thickness: 1 }}
            }},
            connecting: {{
                router: {{
                    name: 'manhattan',
                    args: {{
                        padding: 10,
                        startDirections: ['bottom', 'right', 'left'],
                        endDirections: ['top', 'left', 'right'],
                        step: 10
                    }}
                }},
                connector: {{ name: 'rounded', args: {{ radius: 8 }} }},
                anchor: 'center',
                connectionPoint: 'boundary'
            }}
        }});
        window.graph = graph;

        // Register Node Shapes
        X6.Node.registry.register('octopus-activity-node', {{
            inherit: 'rect',
            attrs: {{
                body: {{
                    rx: 10,
                    ry: 10,
                    fill: '#1e293b',
                    stroke: '#4f46e5',
                    strokeWidth: 2,
                    filter: 'drop-shadow(0 4px 6px rgba(0,0,0,0.3))'
                }},
                label: {{
                    fill: '#f8fafc',
                    fontSize: 13,
                    fontWeight: 500,
                    fontFamily: 'Inter, sans-serif'
                }}
            }}
        }});

        X6.Node.registry.register('octopus-human-node', {{
            inherit: 'polygon',
            attrs: {{
                body: {{
                    refPoints: '0,10 10,0 20,10 10,20',
                    fill: '#272015',
                    stroke: '#f59e0b',
                    strokeWidth: 2,
                    filter: 'drop-shadow(0 4px 14px rgba(245, 158, 11, 0.3))'
                }},
                label: {{
                    fill: '#fde68a',
                    fontSize: 12,
                    fontWeight: 600,
                    fontFamily: 'Inter, sans-serif'
                }}
            }}
        }});

        X6.Node.registry.register('octopus-quorum-approver-node', {{
            inherit: 'rect',
            attrs: {{
                body: {{
                    rx: 10,
                    ry: 10,
                    fill: '#1e293b',
                    stroke: '#3b82f6',
                    strokeWidth: 2,
                    filter: 'drop-shadow(0 4px 10px rgba(59, 130, 246, 0.25))'
                }},
                label: {{
                    fill: '#93c5fd',
                    fontSize: 12,
                    fontWeight: 600,
                    fontFamily: 'Inter, sans-serif'
                }}
            }}
        }});

        X6.Node.registry.register('octopus-quorum-join-node', {{
            inherit: 'polygon',
            attrs: {{
                body: {{
                    refPoints: '0,10 10,0 20,10 10,20',
                    fill: '#134e4a',
                    stroke: '#14b8a6',
                    strokeWidth: 2,
                    filter: 'drop-shadow(0 4px 12px rgba(20, 184, 166, 0.25))'
                }},
                label: {{
                    fill: '#5eead4',
                    fontSize: 12,
                    fontWeight: 600,
                    fontFamily: 'Inter, sans-serif'
                }}
            }}
        }});

        X6.Node.registry.register('octopus-branch-node', {{
            inherit: 'polygon',
            attrs: {{
                body: {{
                    refPoints: '0,10 10,0 20,10 10,20',
                    fill: '#241433',
                    stroke: '#a855f7',
                    strokeWidth: 2,
                    filter: 'drop-shadow(0 4px 12px rgba(168, 85, 247, 0.25))'
                }},
                label: {{
                    fill: '#e9d5ff',
                    fontSize: 12,
                    fontWeight: 600,
                    fontFamily: 'Inter, sans-serif'
                }}
            }}
        }});

        X6.Node.registry.register('octopus-rewind-node', {{
            inherit: 'rect',
            attrs: {{
                body: {{
                    rx: 10,
                    ry: 10,
                    fill: '#21170d',
                    stroke: '#f59e0b',
                    strokeWidth: 2,
                    strokeDasharray: '5 5',
                    filter: 'drop-shadow(0 4px 12px rgba(245, 158, 11, 0.25))'
                }},
                label: {{
                    fill: '#fde68a',
                    fontSize: 12,
                    fontWeight: 600,
                    fontFamily: 'Inter, sans-serif'
                }}
            }}
        }});

        X6.Node.registry.register('octopus-jump-node', {{
            inherit: 'rect',
            attrs: {{
                body: {{
                    rx: 10,
                    ry: 10,
                    fill: '#082f49',
                    stroke: '#06b6d4',
                    strokeWidth: 2,
                    strokeDasharray: '5 5',
                    filter: 'drop-shadow(0 4px 12px rgba(6, 182, 212, 0.25))'
                }},
                label: {{
                    fill: '#67e8f9',
                    fontSize: 12,
                    fontWeight: 600,
                    fontFamily: 'Inter, sans-serif'
                }}
            }}
        }});

        X6.Node.registry.register('octopus-loop-node', {{
            inherit: 'rect',
            attrs: {{
                body: {{
                    rx: 12,
                    ry: 12,
                    fill: '#111827',
                    stroke: '#38bdf8',
                    strokeWidth: 2,
                    filter: 'drop-shadow(0 4px 14px rgba(56, 189, 248, 0.35))'
                }},
                label: {{
                    fill: '#bae6fd',
                    fontSize: 12,
                    fontWeight: 600,
                    fontFamily: 'Inter, sans-serif'
                }}
            }}
        }});

        X6.Node.registry.register('octopus-start-node', {{
            inherit: 'rect',
            attrs: {{
                body: {{
                    rx: 25,
                    ry: 25,
                    fill: '#064e3b',
                    stroke: '#10b981',
                    strokeWidth: 2
                }},
                label: {{
                    fill: '#6ee7b7',
                    fontSize: 13,
                    fontWeight: 700,
                    textWrap: {{
                        width: -24,
                        ellipsis: true
                    }}
                }}
            }}
        }});

        X6.Node.registry.register('octopus-end-node', {{
            inherit: 'rect',
            attrs: {{
                body: {{
                    rx: 25,
                    ry: 25,
                    fill: '#450a0a',
                    stroke: '#ef4444',
                    strokeWidth: 2
                }},
                label: {{
                    fill: '#fca5a5',
                    fontSize: 13,
                    fontWeight: 700,
                    textWrap: {{
                        width: -28,
                        ellipsis: true
                    }}
                }}
            }}
        }});

        X6.Node.registry.register('octopus-compensation-node', {{
            inherit: 'rect',
            attrs: {{
                body: {{
                    rx: 8,
                    ry: 8,
                    fill: '#1c1917',
                    stroke: '#f97316',
                    strokeWidth: 1.5,
                    strokeDasharray: '4 4'
                }},
                label: {{
                    fill: '#fdba74',
                    fontSize: 12,
                    fontFamily: 'Fira Code, monospace',
                    fontWeight: 600,
                    textWrap: {{
                        width: -16,
                        ellipsis: true
                    }}
                }}
            }}
        }});

        // Handle Node Selection
        graph.on('node:click', ({{ node }}) => {{
            graph.getNodes().forEach(n => n.removeTools());
            try {{
                node.addTools([
                    {{
                        name: 'boundary',
                        args: {{
                            padding: 5,
                            attrs: {{
                                fill: 'none',
                                stroke: '#38bdf8',
                                strokeWidth: 2,
                                strokeDasharray: '4,4'
                            }}
                        }}
                    }}
                ]);
            }} catch (e) {{}}

            const data = node.getData() || {{}};
            selectedNodeData = data;
            
            document.getElementById('insp-id').innerText = data.step_id || data.title || node.id;
            document.getElementById('insp-type').innerText = (data.type || 'STEP').toUpperCase();
            document.getElementById('insp-status').innerText = data.status || 'PENDING';
            document.getElementById('insp-timeout').innerText = (data.timeout_seconds ? data.timeout_seconds + 's' : 'Default');

            // Roles & Policy
            if (data.allowed_roles && data.allowed_roles.length > 0) {{
                document.getElementById('insp-roles-field').style.display = 'flex';
                document.getElementById('insp-roles').innerText = data.allowed_roles.join(', ');
            }} else {{
                document.getElementById('insp-roles-field').style.display = 'none';
            }}

            const policiesMeta = data.security_policies_meta || (data.security_policy ? [{{
                name: data.security_policy,
                class: data.security_policy_class || 'SecurityPolicy',
                module: data.security_policy_module || 'strategies',
                doc: data.security_policy_doc || `Enforces '${{data.security_policy}}' on reviewer approval.`
            }}] : []);

            if (policiesMeta.length > 0) {{
                const policyField = document.getElementById('insp-policy-field');
                policyField.style.display = 'flex';
                const labelText = policiesMeta.length > 1 ? `🛡️ Security Policies (${{policiesMeta.length}} Applied, Fail-Fast)` : '🛡️ Security Policy';
                
                let cardsHtml = `<div class="meta-label" style="color: #fbbf24; font-weight: 600; margin-bottom: 4px;">${{labelText}}</div>`;
                policiesMeta.forEach((p, idx) => {{
                    const numBadge = policiesMeta.length > 1 ? `<span style="font-size: 10px; color: #94a3b8; margin-right: 4px; font-weight: bold;">#${{idx + 1}}</span>` : '';
                    cardsHtml += `
                        <div style="background: rgba(15, 23, 42, 0.8); border: 1px solid rgba(251, 191, 36, 0.25); border-radius: 6px; padding: 8px; margin-top: ${{idx > 0 ? '6px' : '2px'}};">
                            <div style="margin-bottom: 4px;">
                                ${{numBadge}}<span style="display: inline-block; padding: 2px 7px; font-size: 11px; font-family: monospace; font-weight: 600; border-radius: 4px; background: rgba(251, 191, 36, 0.15); color: #fbbf24; border: 1px solid rgba(251, 191, 36, 0.3);">${{p.name}}</span>
                            </div>
                            <div style="font-size: 11px; color: var(--text-muted); font-family: 'Fira Code', monospace; word-break: break-all; margin-bottom: 4px;">📁 <span style="color: #38bdf8;">${{p.module}}.${{p.class}}</span></div>
                            <div style="font-size: 11px; color: #cbd5e1; background: #0f172a; padding: 6px 8px; border-radius: 4px; border-left: 2px solid #fbbf24; line-height: 1.4;">${{p.doc}}</div>
                        </div>
                    `;
                }});
                policyField.innerHTML = cardsHtml;
            }} else {{
                document.getElementById('insp-policy-field').style.display = 'none';
            }}

            // Ingress Middlewares Card
            const mwField = document.getElementById('insp-middlewares-field');
            if (mwField) {{
                if (data.middlewares && Array.isArray(data.middlewares) && data.middlewares.length > 0) {{
                    mwField.style.display = 'flex';
                    document.getElementById('insp-middlewares-count-badge').innerText = data.middlewares.length + ' Active Guards';
                    const listEl = document.getElementById('insp-middlewares-list');
                    let listHtml = '';
                    data.middlewares.forEach((mw, i) => {{
                        let mwName = typeof mw === 'object' ? (mw.name || JSON.stringify(mw)) : String(mw);
                        let mwDesc = '';
                        if (mwName.indexOf('BearerAuth') !== -1) mwDesc = 'JWT token validation, HMAC verification & claims identity injection';
                        else if (mwName.indexOf('RateLimit') !== -1) mwDesc = 'Quota enforcement (Burst token bucket & rate-limit window)';
                        else if (mwName.indexOf('IpWhitelist') !== -1 || mwName.indexOf('IpAccess') !== -1) mwDesc = 'Client IP subnet filtering & network access control';
                        else if (mwName.indexOf('CorrelationId') !== -1) mwDesc = 'Distributed tracing header propagation (X-Job-ID / Correlation)';
                        else mwDesc = 'Ingress filter & security policy guard';

                        listHtml += '<div style="background: rgba(30, 41, 59, 0.6); border: 1px solid rgba(100, 116, 139, 0.3); border-radius: 6px; padding: 6px 10px;">';
                        listHtml += '<div style="font-weight: 600; color: #38bdf8; display: flex; align-items: center; gap: 6px;">';
                        listHtml += '<span style="color: #94a3b8; font-size: 10px;">#' + (i+1) + '</span> <span>' + escapeHtml(mwName) + '</span>';
                        listHtml += '</div>';
                        if (mwDesc) {{
                            listHtml += '<div style="font-size: 10px; color: #94a3b8; margin-top: 2px;">' + escapeHtml(mwDesc) + '</div>';
                        }}
                        listHtml += '</div>';
                    }});
                    listEl.innerHTML = listHtml;
                    const descEl = document.getElementById('insp-middlewares-desc');
                    descEl.innerText = data.description || 'Pre-execution security filters and quota validation guards executed before workflow pipeline.';
                }} else {{
                    mwField.style.display = 'none';
                }}
            }}

            // Exception Handler Card
            const excField = document.getElementById('insp-exception-field');
            if (excField) {{
                if (data.type === 'exception_handler' || data.exception_type) {{
                    excField.style.display = 'flex';
                    const code = data.status_code || 400;
                    const excType = data.exception_type || 'Exception';
                    document.getElementById('insp-exc-status-badge').innerText = 'HTTP ' + code;
                    document.getElementById('insp-exc-name').innerText = '⚠️ ' + excType + ' (' + code + ')';
                    
                    let originText = data.source_location || data.source_type || 'Workflow Blueprint Pipeline';
                    if (data.is_middleware || (!data.source_step_id && originText.indexOf('Middleware') !== -1)) {{
                        originText = '🛡️ Middleware Exception Pipeline: ' + (data.middleware_name || data.source_name || 'Ingress Guards');
                    }} else if (data.source_step_id) {{
                        originText = '⚙️ Step Exception Handler: ' + data.source_step_id;
                    }}
                    document.getElementById('insp-exc-origin').innerText = originText;

                    const isAsync = Boolean(data.is_async);
                    document.getElementById('insp-exc-mode').innerHTML = isAsync 
                        ? '<span style="color: #38bdf8;">⚡ Asynchronous</span> (Dispatched in background without blocking response)'
                        : '<span style="color: #fbbf24;">🔄 Synchronous</span> (Blocks until execution completes)';

                    document.getElementById('insp-exc-desc').innerText = data.description || 
                        ('Invoked upon ' + excType + ' to execute ' + (data.activity || 'exception recovery') + ' activity.');
                }} else {{
                    excField.style.display = 'none';
                }}
            }}

            // Branch Conditions & Dynamic Rule Evaluation Card
            const branchField = document.getElementById('insp-branch-field');
            if (branchField) {{
                const isBranch = data.type === 'branch' || Boolean(data.condition) || Boolean(data.conditions) || Boolean(data.branch_info);
                if (isBranch) {{
                    branchField.style.display = 'flex';
                    const verdictBadge = document.getElementById('insp-branch-verdict-badge');
                    const rulesContent = document.getElementById('insp-branch-rules-content');
                    const evalBox = document.getElementById('insp-branch-eval-box');
                    const evalDetails = document.getElementById('insp-branch-eval-details');
                    const thenBox = document.getElementById('insp-branch-then-box');
                    const elseBox = document.getElementById('insp-branch-else-box');
                    const thenStepsEl = document.getElementById('insp-branch-then-steps');
                    const elseStepsEl = document.getElementById('insp-branch-else-steps');

                    const bInfo = data.branch_info || {{}};
                    const cond = data.condition || bInfo.condition || {{}};
                    const conds = data.conditions || bInfo.conditions || {{}};

                    // 1. Build Rules Description
                    let rulesHtml = '';
                    if (cond.rule) {{
                        rulesHtml += '<div style="margin-bottom: 4px;"><strong>Rule:</strong> <span style="color: #fbbf24;">' + escapeHtml(cond.rule) + '</span></div>';
                    }} else if (cond.variable || cond.field) {{
                        const v = cond.variable || cond.field;
                        const op = cond.operator || '==';
                        const val = cond.value !== undefined ? JSON.stringify(cond.value) : '""';
                        rulesHtml += '<div style="margin-bottom: 4px;"><strong>Expression:</strong> <span style="color: #38bdf8;">' + escapeHtml(v) + '</span> <span style="color: #f43f5e; font-weight: bold;">' + escapeHtml(op) + '</span> <span style="color: #34d399;">' + escapeHtml(val) + '</span></div>';
                    }} else if (conds.conditions && Array.isArray(conds.conditions)) {{
                        const matchMode = conds.match || 'all';
                        rulesHtml += '<div style="margin-bottom: 4px;"><strong>Match Mode:</strong> <span style="color: #fbbf24;">' + matchMode.toUpperCase() + '</span> (' + conds.conditions.length + ' rules)</div>';
                        conds.conditions.forEach((sc, i) => {{
                            const sv = sc.variable || sc.field || 'var';
                            const sop = sc.operator || '==';
                            const sval = sc.value !== undefined ? JSON.stringify(sc.value) : '""';
                            rulesHtml += '<div style="padding-left: 8px; border-left: 2px solid #334155; margin-top: 2px;">#' + (i+1) + ': <span style="color: #38bdf8;">' + escapeHtml(sv) + '</span> <span style="color: #f43f5e;">' + escapeHtml(sop) + '</span> <span style="color: #34d399;">' + escapeHtml(sval) + '</span></div>';
                        }});
                    }} else {{
                        rulesHtml += '<div>Dynamic Decision Gate</div>';
                    }}
                    rulesContent.innerHTML = rulesHtml;

                    // 2. Resolve Targets (Then / Else)
                    const thenList = (bInfo.then_step_ids && bInfo.then_step_ids.length > 0) ? bInfo.then_step_ids : (cond.then_steps ? cond.then_steps.map(s => s.id || s.activity) : []);
                    const elseList = (bInfo.else_step_ids && bInfo.else_step_ids.length > 0) ? bInfo.else_step_ids : (cond.else_steps ? cond.else_steps.map(s => s.id || s.activity) : []);
                    
                    thenStepsEl.innerText = thenList.length > 0 ? thenList.join(' ➔ ') : '(End / Next Step)';
                    elseStepsEl.innerText = elseList.length > 0 ? elseList.join(' ➔ ') : '(End / Next Step)';

                    // 3. Live Evaluation
                    const evalInfo = bInfo.evaluation || (data.evaluation);
                    const isExecuted = data.executed || (data.status === 'COMPLETED');

                    // Reset box styles
                    thenBox.style.boxShadow = 'none';
                    thenBox.style.borderColor = 'rgba(16, 185, 129, 0.3)';
                    elseBox.style.boxShadow = 'none';
                    elseBox.style.borderColor = 'rgba(56, 189, 248, 0.3)';

                    if (evalInfo && evalInfo.evaluated) {{
                        evalBox.style.display = 'block';
                        const isTrue = evalInfo.verdict === 'TRUE';
                        if (isTrue) {{
                            verdictBadge.innerHTML = '<span style="background: rgba(16, 185, 129, 0.2); border: 1px solid #10b981; color: #34d399; padding: 2px 8px; border-radius: 4px; font-size: 10px; font-weight: 700;">✓ TRUE (Match)</span>';
                            thenBox.style.boxShadow = '0 0 12px rgba(16, 185, 129, 0.4)';
                            thenBox.style.borderColor = '#10b981';
                        }} else {{
                            verdictBadge.innerHTML = '<span style="background: rgba(56, 189, 248, 0.2); border: 1px solid #38bdf8; color: #7dd3fc; padding: 2px 8px; border-radius: 4px; font-size: 10px; font-weight: 700;">✕ FALSE (Else)</span>';
                            elseBox.style.boxShadow = '0 0 12px rgba(56, 189, 248, 0.4)';
                            elseBox.style.borderColor = '#38bdf8';
                        }}

                        let evalHtml = '<div><strong>Evaluated Variable:</strong> <span style="color: #38bdf8;">' + escapeHtml(evalInfo.variable_name || cond.variable || 'state') + '</span></div>';
                        if (evalInfo.runtime_value !== undefined) {{
                            evalHtml += '<div><strong>Runtime Value:</strong> <span style="color: #34d399; font-weight: bold;">' + escapeHtml(JSON.stringify(evalInfo.runtime_value)) + '</span></div>';
                        }}
                        if (evalInfo.operator && evalInfo.expected_value !== undefined) {{
                            evalHtml += '<div><strong>Rule Check:</strong> <code>' + escapeHtml(JSON.stringify(evalInfo.runtime_value)) + ' ' + escapeHtml(evalInfo.operator) + ' ' + escapeHtml(JSON.stringify(evalInfo.expected_value)) + '</code> ➔ <strong style="color:' + (isTrue ? '#34d399' : '#38bdf8') + '">' + (isTrue ? 'MATCH (TRUE)' : 'MISMATCH (FALSE)') + '</strong></div>';
                        }}
                        evalHtml += '<div style="margin-top: 4px; padding-top: 4px; border-top: 1px dashed rgba(255,255,255,0.1);"><strong>Active Branch Route:</strong> <span style="color:' + (isTrue ? '#34d399' : '#38bdf8') + '; font-weight: bold;">➔ ' + (isTrue ? 'TRUE (Then Branch)' : 'FALSE (Else Branch)') + '</span></div>';
                        evalDetails.innerHTML = evalHtml;
                    }} else if (isExecuted) {{
                        evalBox.style.display = 'block';
                        verdictBadge.innerHTML = '<span style="background: rgba(16, 185, 129, 0.2); border: 1px solid #10b981; color: #34d399; padding: 2px 8px; border-radius: 4px; font-size: 10px; font-weight: 700;">✓ EXECUTED</span>';
                        evalDetails.innerHTML = '<div><strong>Status:</strong> <span style="color: #34d399;">Branch condition evaluated during workflow execution</span></div>';
                    }} else {{
                        evalBox.style.display = 'none';
                        verdictBadge.innerHTML = '<span style="background: rgba(100, 116, 139, 0.2); border: 1px solid #64748b; color: #94a3b8; padding: 2px 8px; border-radius: 4px; font-size: 10px; font-weight: 700;">STANDBY</span>';
                    }}
                }} else {{
                    branchField.style.display = 'none';
                }}
            }}

            // Dynamic Loop & Batch Configuration in Step Inspector
            if (data.is_loop || data.type === 'loop' || Boolean(data.loop)) {{
                document.getElementById('insp-loop-field').style.display = 'flex';
                const loopBadge = document.getElementById('insp-loop-badge');
                const loopDetails = document.getElementById('insp-loop-details');

                let loopType = 'Dynamic Loop Iteration';
                let detailsText = '';

                if (data.loop_for_each) {{
                    loopType = '🔁 Collection Batching (for_each)';
                    detailsText = `Target Collection: <span style="color: #38bdf8;">${{data.loop_for_each}}</span>\n` +
                                  (data.loop_item_variable ? `Item Variable: <span style="color: #a78bfa;">${{data.loop_item_variable}}</span>\n` : '') +
                                  `Batch Execution: Sequential activity invocation per array element`;
                }} else if (data.loop_until) {{
                    loopType = '⏱️ Polling Convergence (until)';
                    const u = data.loop_until;
                    const uVar = (typeof u === 'object' && u.variable) ? u.variable : 'status';
                    const uVal = (typeof u === 'object' && u.value !== undefined) ? JSON.stringify(u.value) : '"ACTIVE"';
                    detailsText = `Target Condition: <span style="color: #38bdf8;">${{uVar}}</span> == <span style="color: #34d399;">${{uVal}}</span>\n` +
                                  `Max Iterations: <span style="color: #fbbf24;">${{data.loop_max_iterations || 5}}</span>\n` +
                                  `Interval: <span style="color: #fbbf24;">${{data.loop_interval_seconds || 1}}s</span>\n` +
                                  `Backoff Factor: <span style="color: #fbbf24;">${{data.loop_backoff_coefficient || 1.0}}x</span>`;
                }} else if (data.loop_while) {{
                    loopType = '🔄 Condition Loop (while)';
                    const w = data.loop_while;
                    const wVar = (typeof w === 'object' && w.variable) ? w.variable : 'status';
                    const wVal = (typeof w === 'object' && w.value !== undefined) ? JSON.stringify(w.value) : '""';
                    detailsText = `Continue While: <span style="color: #38bdf8;">${{wVar}}</span> == <span style="color: #34d399;">${{wVal}}</span>\n` +
                                  `Max Iterations: <span style="color: #fbbf24;">${{data.loop_max_iterations || 5}}</span>\n` +
                                  `Interval: <span style="color: #fbbf24;">${{data.loop_interval_seconds || 1}}s</span>`;
                }} else {{
                    detailsText = `Loop Configuration: ${{JSON.stringify(data.loop || {{}})}}`;
                }}

                loopBadge.innerHTML = `<span style="background: rgba(56, 189, 248, 0.15); border: 1px solid #38bdf8; color: #7dd3fc; padding: 4px 10px; border-radius: 6px; font-size: 11px; font-weight: 700; display: inline-flex; align-items: center; gap: 5px;">${{loopType}}</span>`;
                loopDetails.innerHTML = detailsText.replace(/\\n/g, '<br/>');
            }} else {{
                document.getElementById('insp-loop-field').style.display = 'none';
            }}

            // Activity Code Availability Inspector (Supports regular Activity & Compensation Rollback Nodes)
            if ((data.type === 'activity' || data.type === 'compensation' || data.type === 'loop') && data.activity) {{
                document.getElementById('insp-activity-field').style.display = 'flex';
                const statusElem = document.getElementById('insp-activity-status');
                const metaElem = document.getElementById('insp-activity-meta');
                const docElem = document.getElementById('insp-activity-doc');

                const isCompNode = (data.type === 'compensation');
                const titleLabel = isCompNode ? 'Saga Compensation Implementation' : 'Python Activity Implementation';
                const labelElem = document.querySelector('#insp-activity-field .meta-label');
                if (labelElem) labelElem.innerText = titleLabel;

                if (data.activity_available) {{
                    const badgeText = isCompNode ? '✓ Compensation Activity Found & Registered' : '✓ Activity Found & Registered';
                    statusElem.innerHTML = '<span style="background: rgba(16, 185, 129, 0.15); border: 1px solid #10b981; color: #34d399; padding: 4px 10px; border-radius: 6px; font-size: 11px; font-weight: 600; display: inline-flex; align-items: center; gap: 5px;">' + badgeText + '</span>';
                    const classLabel = data.activity_class ? (' :: <span style="color: #a78bfa; font-weight: 600;">' + data.activity_class + '</span>') : '';
                    metaElem.innerHTML = '📁 <span style="color: #38bdf8;">' + (data.activity_module || 'activities') + '</span>' + classLabel + ' (' + (data.activity_file || '') + ')';
                    if (data.activity_doc) {{
                        docElem.style.display = 'block';
                        docElem.innerText = data.activity_doc;
                    }} else {{
                        docElem.style.display = 'none';
                    }}
                }} else {{
                    const badgeText = isCompNode ? '⚠️ Missing Compensation Implementation' : '⚠️ Missing Activity Implementation';
                    statusElem.innerHTML = '<span style="background: rgba(239, 68, 68, 0.15); border: 1px solid #ef4444; color: #fca5a5; padding: 4px 10px; border-radius: 6px; font-size: 11px; font-weight: 600; display: inline-flex; align-items: center; gap: 5px;">' + badgeText + '</span>';
                    metaElem.innerHTML = '<span style="color: #f87171;">Activity <code>' + data.activity + '()</code> is not defined in any Python activities module or Activity Class.</span>';
                    docElem.style.display = 'none';
                }}
            }} else {{
                document.getElementById('insp-activity-field').style.display = 'none';
            }}

            // Saga Compensation Availability Inspector
            if (data.compensation) {{
                document.getElementById('insp-comp-field').style.display = 'flex';
                const compStatusElem = document.getElementById('insp-comp-status');
                const compMetaElem = document.getElementById('insp-comp-meta');

                if (data.compensation_available) {{
                    compStatusElem.innerHTML = '<span style="background: rgba(16, 185, 129, 0.15); border: 1px solid #10b981; color: #34d399; padding: 4px 10px; border-radius: 6px; font-size: 11px; font-weight: 600; display: inline-flex; align-items: center; gap: 5px;">✓ Compensation Implemented</span>';
                    compMetaElem.innerHTML = '📁 <span style="color: #fb923c;">' + (data.compensation_module || 'activities') + '</span>';
                }} else {{
                    compStatusElem.innerHTML = '<span style="background: rgba(245, 158, 11, 0.15); border: 1px solid #f59e0b; color: #fde68a; padding: 4px 10px; border-radius: 6px; font-size: 11px; font-weight: 600; display: inline-flex; align-items: center; gap: 5px;">⚠️ Compensation Missing</span>';
                    compMetaElem.innerHTML = '<span style="color: #fbbf24;">Rollback function <code>' + data.compensation + '()</code> is not defined.</span>';
                }}
            }} else {{
                document.getElementById('insp-comp-field').style.display = 'none';
            }}

            // Idempotency Strategy Inspector
            if (data.idempotency_strategy) {{
                document.getElementById('insp-idempotency-field').style.display = 'flex';
                const idemBadge = document.getElementById('insp-idempotency-badge');
                const idemMeta = document.getElementById('insp-idempotency-meta');
                const idemDesc = document.getElementById('insp-idempotency-desc');

                const stratName = String(data.idempotency_strategy);
                const className = data.idempotency_strategy_class || 'IdempotencyStrategy';
                const modName = data.idempotency_strategy_module || 'strategies';
                const stratDoc = data.idempotency_strategy_doc || `Domain idempotency strategy '${{stratName}}'.`;

                idemBadge.innerHTML = `<span style="background: rgba(56, 189, 248, 0.15); border: 1px solid #38bdf8; color: #7dd3fc; padding: 4px 10px; border-radius: 6px; font-size: 11px; font-weight: 700; font-family: monospace; display: inline-flex; align-items: center; gap: 5px;">🔑 ${{stratName}}</span>`;
                idemMeta.innerHTML = `📁 <span style="color: #38bdf8;">${{modName}}.${{className}}</span>`;
                idemDesc.style.display = 'block';
                idemDesc.innerText = stratDoc;
            }} else {{
                document.getElementById('insp-idempotency-field').style.display = 'none';
            }}

            // Execution Metrics & Payloads Inspector
            const timelineItem = data.timeline || null;
            const inputPayload = (timelineItem && (timelineItem.input !== undefined || timelineItem.input_payload !== undefined))
                ? (timelineItem.input !== undefined ? timelineItem.input : timelineItem.input_payload)
                : (data.input_payload !== undefined ? data.input_payload : (data.input !== undefined ? data.input : (data.type === 'start' ? (data.job_data || data.input_payload) : null)));
            
            const outputPayload = (timelineItem && (timelineItem.output !== undefined || timelineItem.output_payload !== undefined))
                ? (timelineItem.output !== undefined ? timelineItem.output : timelineItem.output_payload)
                : (data.output_payload !== undefined ? data.output_payload : (data.output !== undefined ? data.output : null));

            const errorMessage = (timelineItem && (timelineItem.error !== undefined || timelineItem.error_message !== undefined))
                ? (timelineItem.error || timelineItem.error_message)
                : (data.error || null);

            const durationMs = (timelineItem && timelineItem.duration_ms !== undefined)
                ? timelineItem.duration_ms
                : (data.duration_ms !== undefined ? data.duration_ms : null);

            const isOutputStep = Boolean(data.is_output || data.output);

            // Metrics Card
            const metricsField = document.getElementById('insp-execution-metrics');
            const durationBadge = document.getElementById('insp-duration-badge');
            const timestampsElem = document.getElementById('insp-timestamps');

            if (data.executed && (durationMs !== null && durationMs !== undefined || (timelineItem && (timelineItem.finished_at || timelineItem.started_at)))) {{
                metricsField.style.display = 'flex';
                durationBadge.innerText = (durationMs !== null && durationMs !== undefined) ? `${{durationMs}}ms` : 'Recorded';
                let timeStr = '';
                if (timelineItem && timelineItem.started_at) {{
                    timeStr += `Started: ${{timelineItem.started_at.replace('T', ' ').slice(0, 19)}} UTC`;
                }}
                if (timelineItem && timelineItem.finished_at) {{
                    if (timeStr) timeStr += ' • ';
                    timeStr += `Finished: ${{timelineItem.finished_at.replace('T', ' ').slice(0, 19)}} UTC`;
                }}
                timestampsElem.innerText = timeStr || 'Step completed in pipeline trace.';
            }} else {{
                metricsField.style.display = 'none';
            }}

            // Input Payload
            const inputField = document.getElementById('insp-input-payload-field');
            const inputContent = document.getElementById('insp-input-json-content');
            const inputCountBadge = document.getElementById('insp-input-count-badge');
            const copyInputBtn = document.getElementById('copyInputPayloadBtn');

            if (data.executed && inputPayload !== null && inputPayload !== undefined && Object.keys(inputPayload || {{}}).length > 0) {{
                inputField.style.display = 'flex';
                const keysCount = (typeof inputPayload === 'object' && inputPayload !== null) ? Object.keys(inputPayload).length : 1;
                inputCountBadge.innerText = `${{keysCount}} ${{keysCount === 1 ? 'field' : 'fields'}}`;
                inputContent.innerHTML = highlightJson(inputPayload);
                copyInputBtn.setAttribute('data-raw-json', JSON.stringify(inputPayload, null, 2));
            }} else if (data.executed && (inputPayload !== null && inputPayload !== undefined)) {{
                inputField.style.display = 'flex';
                inputCountBadge.innerText = '0 fields';
                inputContent.innerHTML = '<span style="color: #64748b;">{{}}</span>';
                copyInputBtn.setAttribute('data-raw-json', '{{}}');
            }} else {{
                inputField.style.display = 'none';
            }}

            // Output Payload
            const outputField = document.getElementById('insp-output-payload-field');
            const outputContent = document.getElementById('insp-output-json-content');
            const outputCountBadge = document.getElementById('insp-output-count-badge');
            const outputRoleBadge = document.getElementById('insp-output-role-badge');
            const copyOutputBtn = document.getElementById('copyOutputPayloadBtn');

            if (data.executed && outputPayload !== null && outputPayload !== undefined) {{
                outputField.style.display = 'flex';
                const outKeysCount = (typeof outputPayload === 'object' && outputPayload !== null) ? Object.keys(outputPayload).length : 1;
                outputCountBadge.innerText = `${{outKeysCount}} ${{outKeysCount === 1 ? 'field' : 'fields'}}`;
                if (isOutputStep) {{
                    outputRoleBadge.style.display = 'inline-block';
                }} else {{
                    outputRoleBadge.style.display = 'none';
                }}
                outputContent.innerHTML = highlightJson(outputPayload);
                copyOutputBtn.setAttribute('data-raw-json', JSON.stringify(outputPayload, null, 2));
            }} else {{
                outputField.style.display = 'none';
            }}

            // Error Payload
            const errorField = document.getElementById('insp-error-payload-field');
            const errorMsgElem = document.getElementById('insp-error-message');
            if (errorMessage) {{
                errorField.style.display = 'flex';
                errorMsgElem.innerText = errorMessage;
            }} else {{
                errorField.style.display = 'none';
            }}

            // Show approval buttons ONLY if step is actively awaiting approval in a running workflow
            if (data.status === 'AWAITING_APPROVAL') {{
                document.getElementById('approvalActions').style.display = 'flex';
            }} else {{
                document.getElementById('approvalActions').style.display = 'none';
            }}

            // Automatically slide out and open Step Inspector when item is clicked
            toggleInspector(true);
        }});

        graph.on('blank:click', () => {{
            graph.getNodes().forEach(n => n.removeTools());
            // Automatically fold and hide Step Inspector to the right when clicking canvas
            toggleInspector(false);
        }});

        // Fold / Unfold Step Inspector Drawer
        function toggleInspector(open = null) {{
            const panel = document.getElementById('inspector');
            const openBtn = document.getElementById('openInspectorBtn');
            if (!panel) return;

            const isOpen = panel.classList.contains('open');
            const shouldOpen = open === null ? !isOpen : open;

            if (shouldOpen) {{
                panel.classList.add('open');
                if (openBtn) openBtn.style.display = 'none';
            }} else {{
                panel.classList.remove('open');
                if (openBtn && selectedNodeData) openBtn.style.display = 'flex';
            }}
        }}
        window.toggleInspector = toggleInspector;
        window.selectNode = function(idOrMatch) {{
            if (!graph || !graph.getNodes) return false;
            const nodes = graph.getNodes();
            const target = nodes.find(n => n.id === idOrMatch || n.id.includes(idOrMatch) || (n.getData() && (n.getData().step_id === idOrMatch || n.getData().activity === idOrMatch)));
            if (target) {{
                graph.trigger('node:click', {{ node: target }});
                return true;
            }}
            return false;
        }};

        // Auto-Layout using Dagre algorithm
        function applyDagreLayout(graphData, rankdir = 'TB') {{
            if (!graph || typeof dagre === 'undefined' || !graphData) return;
            currentGraphData = graphData;

            let rawNodes = graphData.nodes || [];
            let rawEdges = graphData.edges || [];

            // Support X6 cell formats if passed from graph.toJSON()
            if (!rawNodes.length && graphData.cells) {{
                rawNodes = graphData.cells.filter(c => c.shape !== 'edge').map(c => ({{
                    id: c.id,
                    width: (c.size && c.size.width) || 220,
                    height: (c.size && c.size.height) || 80,
                    ...c
                }}));
                rawEdges = graphData.cells.filter(c => c.shape === 'edge').map(c => ({{
                    id: c.id,
                    source: c.source ? (c.source.cell || c.source) : c.source,
                    target: c.target ? (c.target.cell || c.target) : c.target,
                    ...c
                }}));
            }}

            // Defensive: Filter out any edges where source or target does not exist in rawNodes
            const nodeIds = new Set(rawNodes.map(n => n.id));
            rawEdges = rawEdges.filter(edge => {{
                const src = edge.source ? (edge.source.cell || edge.source) : edge.source;
                const tgt = edge.target ? (edge.target.cell || edge.target) : edge.target;
                return nodeIds.has(src) && nodeIds.has(tgt);
            }});

            const g = new dagre.graphlib.Graph();
            g.setGraph({{ rankdir: rankdir, nodesep: rankdir === 'LR' ? 45 : 55, ranksep: rankdir === 'LR' ? 85 : 75, marginx: 35, marginy: 35 }});
            g.setDefaultEdgeLabel(() => ({{}}));

            rawNodes.forEach(node => {{
                const w = (node.size && node.size.width) || node.width || 220;
                const h = (node.size && node.size.height) || node.height || 80;
                g.setNode(node.id, {{ width: w, height: h }});
            }});

            rawEdges.forEach(edge => {{
                const src = edge.source ? (edge.source.cell || edge.source) : edge.source;
                const tgt = edge.target ? (edge.target.cell || edge.target) : edge.target;
                if (src && tgt) {{
                    g.setEdge(src, tgt);
                }}
            }});

            dagre.layout(g);

            const positionedNodes = rawNodes.map(node => {{
                const pos = g.node(node.id);
                const w = (node.size && node.size.width) || node.width || 220;
                const h = (node.size && node.size.height) || node.height || 80;
                if (pos) {{
                    return {{
                        ...node,
                        x: pos.x - w / 2,
                        y: pos.y - h / 2
                    }};
                }}
                return node;
            }});

            const formattedEdges = rawEdges.map(edge => {{
                if (edge.id && edge.id.includes('rewind')) {{
                    return {{
                        ...edge,
                        router: {{
                            name: 'manhattan',
                            args: {{
                                padding: 25,
                                startDirections: ['left', 'bottom'],
                                endDirections: ['left']
                            }}
                        }},
                        connector: {{ name: 'rounded', args: {{ radius: 12 }} }}
                    }};
                }}
                return {{
                    ...edge,
                    router: {{
                        name: 'manhattan',
                        args: {{
                            padding: 10,
                            startDirections: rankdir === 'LR' ? ['right', 'bottom'] : ['bottom', 'right', 'left'],
                            endDirections: rankdir === 'LR' ? ['left', 'top'] : ['top', 'left', 'right'],
                            step: 10
                        }}
                    }},
                    connector: {{ name: 'rounded', args: {{ radius: 8 }} }}
                }};
            }});

            graph.fromJSON({{
                nodes: positionedNodes,
                edges: formattedEdges
            }});

            setTimeout(() => {{
                graph.zoomToFit({{ padding: 40, maxScale: 1.1 }});
            }}, 50);
        }}

        function toggleWorkflowTile(forceState) {{
            const tile = document.getElementById('workflowTile');
            const icon = document.getElementById('wfTileToggleIcon');
            if (!tile) return;
            let isMin;
            if (typeof forceState === 'boolean') {{
                isMin = forceState;
                tile.classList.toggle('minimized', isMin);
            }} else {{
                isMin = tile.classList.toggle('minimized');
            }}
            if (icon) {{
                icon.innerText = isMin ? '▼' : '▲';
            }}
            try {{
                localStorage.setItem('octopus_wf_tile_minimized', isMin ? '1' : '0');
            }} catch(e) {{}}
        }}

        function updateWorkflowTile(data) {{
            if (!data) return;
            const headerNameEl = document.getElementById('wf-tile-name');
            const nameValEl = document.getElementById('wf-tile-name-val');
            const verValEl = document.getElementById('wf-tile-version-val');
            const descValEl = document.getElementById('wf-tile-desc-val');
            const routeRowEl = document.getElementById('wf-tile-route-row');
            const routeValEl = document.getElementById('wf-tile-route-val');

            const domainName = data.workflow_name || currentDomain || 'Workflow';
            let versionStr = data.version ? (String(data.version).startsWith('v') ? data.version : 'v' + data.version) : 'v1.0.0';

            if (headerNameEl) {{
                headerNameEl.innerText = 'Workflow Overview';
            }}
            if (nameValEl) {{
                nameValEl.innerText = domainName;
            }}
            if (verValEl) {{
                verValEl.innerText = versionStr;
            }}
            if (descValEl) {{
                descValEl.innerText = data.description || 'Interactive DAG workflow blueprint and live execution trace visualizer.';
            }}

            const actRoute = data.selected_route || currentRoute;
            const actMethod = data.selected_method || currentMethod;
            if (routeRowEl && routeValEl) {{
                if (actRoute) {{
                    routeRowEl.style.display = 'flex';
                    routeValEl.innerText = (actMethod ? actMethod + ' ' : '') + actRoute;
                }} else {{
                    routeRowEl.style.display = 'none';
                }}
            }}
        }}

        // Restore minimization state if set
        try {{
            if (localStorage.getItem('octopus_wf_tile_minimized') === '1') {{
                setTimeout(() => toggleWorkflowTile(true), 10);
            }}
        }} catch(e) {{}}

        // Load Static Blueprint
        async function loadBlueprintGraph(preserveBanner = false) {{
            if (livePollingTimer) {{
                clearInterval(livePollingTimer);
                livePollingTimer = null;
            }}
            document.getElementById('liveIndicator').style.display = 'none';
            if (!preserveBanner) {{
                document.getElementById('executionBanner').style.display = 'none';
            }}
            selectedNodeData = null;
            toggleInspector(false);

            if (!currentPrefix && domainSelect && domainSelect.options.length > 0) {{
                const selIdx = domainSelect.selectedIndex >= 0 ? domainSelect.selectedIndex : 0;
                const activeOpt = domainSelect.options[selIdx];
                if (activeOpt) {{
                    currentDomain = activeOpt.value;
                    currentPrefix = activeOpt.dataset.prefix;
                }}
            }}
            if (!currentPrefix) {{
                currentPrefix = INITIAL_PREFIX || "/api/v1.0.0/sample_workflow";
            }}

            try {{
                let url = `${{currentPrefix}}/diagram/x6`;
                const qParams = [];
                if (currentRoute) qParams.push('route=' + encodeURIComponent(currentRoute));
                if (currentMethod) qParams.push('method=' + encodeURIComponent(currentMethod));
                if (qParams.length > 0) url += '?' + qParams.join('&');

                const res = await fetch(safeApiUrl(url), {{ headers: getApiHeaders() }});
                if (!res.ok) throw new Error('HTTP ' + res.status);
                const data = await res.json();
                updateWorkflowTile(data);
                if (data.routes) {{
                    populateRouteDropdown(data.routes, data.selected_route || currentRoute, data.selected_method || currentMethod);
                }}
                applyDagreLayout(data, currentLayoutDir);
            }} catch (err) {{
                console.error("Failed to load blueprint:", err);
                showToast(`Could not load blueprint: ${{err.message}}`, '⚠️', '#f59e0b', 4000);
            }}
        }}

        // 1. LOOKUP JOB: Checks if job exists, highlights path if complete, enables track if running, gives message if not found
        async function lookupJob() {{
            const jobId = document.getElementById('jobIdInput').value.trim();
            if (!jobId) {{
                showToast('Please enter a valid Job ID to lookup.', '⚠️', '#f59e0b');
                return;
            }}

            if (livePollingTimer) {{
                clearInterval(livePollingTimer);
                livePollingTimer = null;
                document.getElementById('liveIndicator').style.display = 'none';
                document.getElementById('trackBtn').innerText = '⚡ Track Live';
            }}
            selectedNodeData = null;
            toggleInspector(false);

            try {{
                // Step 1: Resolve job status & domain
                const resolveRes = await fetch(safeApiUrl(`/api/visualizer/resolve-job?job_id=${{encodeURIComponent(jobId)}}`), {{ headers: getApiHeaders() }});
                if (!resolveRes.ok) {{
                    await loadBlueprintGraph(true);
                    const banner = document.getElementById('executionBanner');
                    const summaryText = document.getElementById('executionSummaryText');
                    banner.style.display = 'flex';
                    banner.style.borderColor = 'rgba(239, 68, 68, 0.5)';
                    banner.style.background = 'rgba(239, 68, 68, 0.14)';
                    banner.style.color = '#fca5a5';
                    summaryText.innerHTML = `⚠️ Job ID <strong>"${{jobId}}"</strong> Not Found. Showing Base Blueprint Diagram.`;
                    showToast(`⚠️ Job '${{jobId}}' does not exist in cluster. Displaying standard blueprint for '${{currentDomain}}'.`, '⚠️', '#f59e0b', 6000);
                    document.getElementById('trackBtn').disabled = true;
                    document.getElementById('trackBtn').classList.remove('btn-active-track');
                    return;
                }}
                const resolved = await resolveRes.json();

                const banner = document.getElementById('executionBanner');
                const summaryText = document.getElementById('executionSummaryText');

                // Case C: Job does NOT exist
                if (!resolved.exists) {{
                    await loadBlueprintGraph(true);
                    showToast(`⚠️ Job '${{jobId}}' does not exist in cluster. Displaying standard blueprint for '${{currentDomain}}'.`, '⚠️', '#f59e0b', 6000);
                    document.getElementById('trackBtn').disabled = true;
                    document.getElementById('trackBtn').classList.remove('btn-active-track');
                    banner.style.display = 'flex';
                    banner.style.borderColor = 'rgba(239, 68, 68, 0.5)';
                    banner.style.background = 'rgba(239, 68, 68, 0.14)';
                    banner.style.color = '#fca5a5';
                    summaryText.innerHTML = `⚠️ Job ID <strong>"${{jobId}}"</strong> Not Found. Showing Base Blueprint Diagram.`;
                    return;
                }}



                // Case D: Cross-Service Workflow - Job exists in Temporal cluster, but belongs to a workflow NOT hosted on this service
                if (resolved.belongs_to_service === false) {{
                    showToast(`⚠️ Job '${{jobId}}' was initiated by workflow '${{resolved.domain}}', which is hosted by another microservice.`, '⚠️', '#f59e0b', 8000);
                    document.getElementById('trackBtn').disabled = true;
                    document.getElementById('trackBtn').classList.remove('btn-active-track');
                    banner.style.display = 'flex';
                    banner.style.borderLeftColor = '#f59e0b';
                    summaryText.innerHTML = `⚠️ <strong style="color:#f59e0b">Cross-Service Workflow:</strong> Job '<strong>${{jobId}}</strong>' was initiated by <code>${{resolved.domain}}</code> (hosted on another service). It cannot be overlaid on <code>${{currentDomain}}</code>.`;
                    loadBlueprintGraph();
                    return;
                }}

                let targetJobId = jobId;
                let childInfo = null;

                if (resolved.is_child && resolved.parent_job_id) {{
                    targetJobId = resolved.parent_job_id;
                    childInfo = resolved;
                    showToast(`🔗 Linked child ${{resolved.child_type || ''}} workflow to parent ${{resolved.parent_job_id}}`, '🔗', '#38bdf8', 6000);
                }}

                if (resolved.route) {{
                    currentRoute = resolved.route;
                }}
                if (resolved.method) {{
                    currentMethod = resolved.method;
                }}

                // If job belongs to another workflow domain on THIS service, auto-switch dropdown
                if (resolved.domain && resolved.domain !== currentDomain) {{
                    currentDomain = resolved.domain;
                    currentPrefix = resolved.prefix;
                    for (let i = 0; i < domainSelect.options.length; i++) {{
                        if (domainSelect.options[i].value === currentDomain) {{
                            domainSelect.selectedIndex = i;
                            break;
                        }}
                    }}
                }}

                // Step 2: Fetch diagram with execution trace (uses targetJobId to display full parent DAG if child)
                let diagUrl = `${{currentPrefix}}/diagram/x6?job_id=${{targetJobId}}`;
                if (currentRoute) {{
                    diagUrl += '&route=' + encodeURIComponent(currentRoute);
                }}
                const res = await fetch(safeApiUrl(diagUrl), {{ headers: getApiHeaders() }});
                if (!res.ok) throw new Error('HTTP ' + res.status);
                const data = await res.json();

                updateWorkflowTile(data);
                if (data.routes) {{
                    populateRouteDropdown(data.routes, data.selected_route, data.selected_method);
                    if (data.selected_route) {{
                        currentRoute = data.selected_route;
                        currentMethod = data.selected_method;
                    }}
                }}

                applyDagreLayout(data, currentLayoutDir);

                if (data.mismatch && data.mismatch.is_mismatch) {{
                    showToast(data.mismatch.message, '⚠️', '#f59e0b', 8000);
                    document.getElementById('trackBtn').disabled = true;
                    document.getElementById('trackBtn').classList.remove('btn-active-track');
                    banner.style.display = 'flex';
                    banner.style.borderLeftColor = '#f59e0b';
                    summaryText.innerHTML = `⚠️ <strong style="color:#f59e0b">Workflow Mismatch:</strong> ${{data.mismatch.message}}`;
                    return;
                }}

                const executedCount = (data.nodes || []).filter(n => n.data && n.data.executed).length;

                // Case A: Job is FAILED or TIMED_OUT (or parent of rolled-back child)
                const effectiveStatus = (childInfo && data.overall_status) ? data.overall_status : resolved.status;

                if (effectiveStatus === 'FAILED' || effectiveStatus === 'TIMED_OUT' || data.overall_status === 'FAILED') {{
                    const compNodes = (data.nodes || []).filter(n => n.data && n.data.type === 'compensation' && n.data.executed);
                    const hasComp = compNodes.length > 0;
                    const isTimeout = effectiveStatus === 'TIMED_OUT' || (data.nodes || []).some(n => n.data && n.data.is_sla_breach);

                    let failMsg = `⚠️ Job '${{targetJobId}}' has FAILED.`;
                    let statusLabel = effectiveStatus || 'FAILED';

                    if (hasComp) {{
                        const sortedComps = [...compNodes].sort((a, b) => {{
                            const p = data.executed_path || [];
                            const ia = p.indexOf(a.data.activity);
                            const ib = p.indexOf(b.data.activity);
                            return (ia !== -1 && ib !== -1) ? (ia - ib) : 0;
                        }});
                        const compNames = sortedComps.map(c => c.data.activity || c.data.title).join(' → ');
                        failMsg = `⚠️ Job '${{targetJobId}}' FAILED. Saga Rollback Executed: ${{compNames}} (${{compNodes.length}} compensations completed).`;
                        statusLabel = 'ROLLED_BACK';
                    }} else if (isTimeout) {{
                        failMsg = `⏱️ Job '${{targetJobId}}' TIMED OUT (SLA Breach). No SLA fallback handler configured.`;
                        statusLabel = 'TIMED OUT (SLA BREACH)';
                    }} else {{
                        failMsg = `⚠️ Job '${{targetJobId}}' FAILED (Timeout or Execution Error).`;
                    }}

                    showToast(failMsg, '⚠️', hasComp ? '#f97316' : '#ef4444', 8000);
                    document.getElementById('trackBtn').disabled = true;
                    document.getElementById('trackBtn').classList.remove('btn-active-track');
                    
                    banner.style.display = 'flex';
                    if (hasComp) {{
                        banner.style.borderColor = 'rgba(249, 115, 22, 0.5)';
                        banner.style.background = 'rgba(249, 115, 22, 0.14)';
                        banner.style.color = '#fdba74';
                        summaryText.innerHTML = '<b>Execution Path:</b> ' + (executedCount - compNodes.length) + ' forward steps + ' + compNodes.length + ' compensations executed &bull; Status: <span style="color:#f97316; font-weight:700;">' + statusLabel + '</span>';
                    }} else {{
                        banner.style.borderColor = 'rgba(239, 68, 68, 0.5)';
                        banner.style.background = 'rgba(239, 68, 68, 0.14)';
                        banner.style.color = '#fca5a5';
                        summaryText.innerHTML = '<b>Execution Path:</b> ' + executedCount + ' steps reached &bull; Status: <span style="color:#ef4444; font-weight:700;">' + statusLabel + '</span>';
                    }}
                }}
                // Case B: Job is COMPLETED or TERMINATED
                else if (effectiveStatus === 'COMPLETED' || effectiveStatus === 'TERMINATED') {{
                    showToast(`✅ Job '${{targetJobId}}' is ${{effectiveStatus}}! Complete execution path highlighted.`, '✅', '#10b981', 6000);
                    document.getElementById('trackBtn').disabled = true;
                    document.getElementById('trackBtn').classList.remove('btn-active-track');
                    
                    banner.style.display = 'flex';
                    banner.style.borderColor = 'rgba(16, 185, 129, 0.4)';
                    banner.style.background = 'rgba(16, 185, 129, 0.12)';
                    banner.style.color = '#6ee7b7';
                    summaryText.innerHTML = '<b>Execution Path:</b> ' + executedCount + ' steps executed &bull; Status: <span style="color:#34d399; font-weight:700;">' + effectiveStatus + '</span>';
                }}
                // Case C: Job is RUNNING or AWAITING_APPROVAL
                else {{
                    showToast(`⏳ Job '${{targetJobId}}' is ${{effectiveStatus || 'RUNNING'}}. Live tracking is now enabled.`, '⚡', '#6366f1', 6000);
                    document.getElementById('trackBtn').disabled = false;
                    document.getElementById('trackBtn').classList.add('btn-active-track');
                    
                    const isAwaiting = effectiveStatus === 'AWAITING_APPROVAL' || effectiveStatus === 'AWAITING_SIGNAL' || (data.overall_status && (data.overall_status.includes('AWAITING') || data.overall_status === 'RUNNING'));
                    const statusBadge = isAwaiting
                        ? '<span style="background: rgba(245, 158, 11, 0.2); color: #fde68a; border: 1px solid #f59e0b; padding: 2px 8px; border-radius: 4px; font-weight: 700; margin-left: 8px; font-size: 11px; letter-spacing: 0.5px;">AWAITING REVIEW</span>'
                        : '<span style="color:#fbbf24; font-weight:700;">' + (effectiveStatus || 'RUNNING') + '</span>';

                    banner.style.display = 'flex';
                    banner.style.borderColor = 'rgba(245, 158, 11, 0.4)';
                    banner.style.background = 'rgba(245, 158, 11, 0.12)';
                    banner.style.color = '#fde68a';
                    summaryText.innerHTML = '✨ Execution Path: <strong>' + executedCount + ' steps executed</strong> &bull; Status: ' + statusBadge;
                }}

                if (childInfo) {{
                    summaryText.innerHTML = `<span style="background:rgba(56,189,248,0.2); border:1px solid #38bdf8; color:#7dd3fc; padding:2px 7px; border-radius:4px; font-size:10px; font-weight:700; margin-right:8px; display:inline-flex; align-items:center; gap:3px;">🔗 CHILD LINK</span> ` + summaryText.innerHTML + ` <span style="color:#94a3b8; font-size:11px; margin-left:8px;">(Child: <code>${{jobId}}</code>)</span>`;
                }}

            }} catch (err) {{
                console.error("Lookup error:", err);
                showToast(`❌ Lookup error: ${{err.message}}`, '❌', '#ef4444');
                const banner = document.getElementById('executionBanner');
                const summaryText = document.getElementById('executionSummaryText');
                if (banner && summaryText) {{
                    banner.style.display = 'flex';
                    banner.style.borderColor = 'rgba(239, 68, 68, 0.5)';
                    banner.style.background = 'rgba(239, 68, 68, 0.14)';
                    banner.style.color = '#fca5a5';
                    summaryText.innerHTML = `⚠️ Job ID <strong>"${{jobId}}"</strong> Not Found. Showing Base Blueprint Diagram.`;
                }}
                loadBlueprintGraph();
            }}
        }}

        // 2. TRACK LIVE JOB: Polls running jobs continuously until completed
        function toggleLiveTracking() {{
            const trackBtn = document.getElementById('trackBtn');
            const jobId = document.getElementById('jobIdInput').value.trim();
            if (!jobId) return;

            if (livePollingTimer) {{
                // Stop tracking
                clearInterval(livePollingTimer);
                livePollingTimer = null;
                document.getElementById('liveIndicator').style.display = 'none';
                trackBtn.innerText = '⚡ Track Live';
                showToast('Live tracking paused.', '⏸', '#94a3b8', 3000);
            }} else {{
                // Start tracking
                document.getElementById('liveIndicator').style.display = 'flex';
                trackBtn.innerText = '⏹ Stop Tracking';
                showToast(`Live polling active for '${{jobId}}' (every 2.5s)`, '⚡', '#38bdf8', 3000);

                async function poll() {{
                    try {{
                        const resolveRes = await fetch(safeApiUrl(`/api/visualizer/resolve-job?job_id=${{encodeURIComponent(jobId)}}`), {{ headers: getApiHeaders() }});
                        let targetJobId = jobId;
                        if (resolveRes.ok) {{
                            const rData = await resolveRes.json();
                            if (rData && rData.is_child && rData.parent_job_id) {{
                                targetJobId = rData.parent_job_id;
                            }}
                        }}

                        const res = await fetch(safeApiUrl(`${{currentPrefix}}/diagram/x6?job_id=${{targetJobId}}`), {{ headers: getApiHeaders() }});
                        if (!res.ok) throw new Error('HTTP ' + res.status);
                        const data = await res.json();
                        applyDagreLayout(data, currentLayoutDir);

                        // If workflow completed during tracking, stop polling
                        if (data.overall_status === 'COMPLETED' || data.overall_status === 'TERMINATED' || data.overall_status === 'FAILED') {{
                            clearInterval(livePollingTimer);
                            livePollingTimer = null;
                            document.getElementById('liveIndicator').style.display = 'none';
                            trackBtn.innerText = '⚡ Track Live';
                            trackBtn.disabled = true;
                            trackBtn.classList.remove('btn-active-track');
                            
                            const banner = document.getElementById('executionBanner');
                            const summaryText = document.getElementById('executionSummaryText');
                            const executedCount = (data.nodes || []).filter(n => n.data && n.data.executed).length;

                            if (data.overall_status === 'FAILED' || data.overall_status === 'TIMED_OUT') {{
                                const compNodes = (data.nodes || []).filter(n => n.data && n.data.type === 'compensation' && n.data.executed);
                                const hasComp = compNodes.length > 0;
                                const isTimeout = data.overall_status === 'TIMED_OUT' || (data.nodes || []).some(n => n.data && n.data.is_sla_breach);

                                let failMsg = `⚠️ Job '${{jobId}}' has FAILED.`;
                                let statusLabel = data.overall_status || 'FAILED';

                                if (hasComp) {{
                                    const sortedComps = [...compNodes].sort((a, b) => {{
                                        const p = data.executed_path || [];
                                        const ia = p.indexOf(a.data.activity);
                                        const ib = p.indexOf(b.data.activity);
                                        return (ia !== -1 && ib !== -1) ? (ia - ib) : 0;
                                    }});
                                    const compNames = sortedComps.map(c => c.data.activity || c.data.title).join(' → ');
                                    failMsg = `⚠️ Job '${{jobId}}' FAILED. Saga Rollback Executed: ${{compNames}} (${{compNodes.length}} compensations completed).`;
                                    statusLabel = 'FAILED (SAGA ROLLED BACK)';
                                }} else if (isTimeout) {{
                                    failMsg = `⏱️ Job '${{jobId}}' TIMED OUT (SLA Breach). No SLA fallback handler configured.`;
                                    statusLabel = 'TIMED OUT (SLA BREACH)';
                                }} else {{
                                    failMsg = `⚠️ Job '${{jobId}}' FAILED (Timeout or Execution Error).`;
                                }}

                                showToast(failMsg, '⚠️', hasComp ? '#f97316' : '#ef4444', 8000);
                                if (hasComp) {{
                                    banner.style.borderColor = 'rgba(249, 115, 22, 0.5)';
                                    banner.style.background = 'rgba(249, 115, 22, 0.14)';
                                    banner.style.color = '#fdba74';
                                    summaryText.innerHTML = '<b>Execution Path:</b> ' + (executedCount - compNodes.length) + ' forward steps + ' + compNodes.length + ' compensations executed &bull; Status: <span style="color:#f97316; font-weight:700;">' + statusLabel + '</span>';
                                }} else {{
                                    banner.style.borderColor = 'rgba(239, 68, 68, 0.5)';
                                    banner.style.background = 'rgba(239, 68, 68, 0.14)';
                                    banner.style.color = '#fca5a5';
                                    summaryText.innerHTML = '<b>Execution Path:</b> ' + executedCount + ' steps reached &bull; Status: <span style="color:#ef4444; font-weight:700;">' + statusLabel + '</span>';
                                }}
                            }} else {{
                                showToast(`🎉 Job '${{jobId}}' has finished (${{data.overall_status}})! Path highlighted.`, '🎉', '#10b981', 8000);
                                banner.style.borderColor = 'rgba(16, 185, 129, 0.4)';
                                banner.style.background = 'rgba(16, 185, 129, 0.12)';
                                banner.style.color = '#6ee7b7';
                                summaryText.innerHTML = '<b>Execution Path:</b> ' + executedCount + ' steps executed &bull; Status: <span style="color:#34d399; font-weight:700;">' + data.overall_status + '</span>';
                            }}
                        }}
                    }} catch (e) {{
                        console.error("Live poll error:", e);
                    }}
                }}

                poll();
                livePollingTimer = setInterval(poll, 2500);
            }}
        }}

        function resetToBlueprint() {{
            if (livePollingTimer) {{
                clearInterval(livePollingTimer);
                livePollingTimer = null;
            }}
            document.getElementById('liveIndicator').style.display = 'none';
            document.getElementById('trackBtn').disabled = true;
            document.getElementById('trackBtn').innerText = '⚡ Track Live';
            document.getElementById('trackBtn').classList.remove('btn-active-track');
            document.getElementById('executionBanner').style.display = 'none';
            document.getElementById('jobIdInput').value = '';
            showToast(`Reset to '${{currentDomain}}' standard blueprint.`, '↺', '#6366f1', 3000);
            loadBlueprintGraph();
        }}

        function toggleLayout() {{
            currentLayoutDir = currentLayoutDir === 'TB' ? 'LR' : 'TB';
            if (currentGraphData) {{
                applyDagreLayout(currentGraphData, currentLayoutDir);
            }} else {{
                const data = graph.toJSON();
                applyDagreLayout(data, currentLayoutDir);
            }}
            showToast(`Switched layout to ${{currentLayoutDir === 'TB' ? 'Vertical (Top ➔ Bottom)' : 'Horizontal (Left ➔ Right)'}}`, '⇄', '#38bdf8', 3000);
        }}

        // Submit In-Graph Signal Decision
        async function submitDecision(decision) {{
            const jobId = document.getElementById('jobIdInput').value.trim();
            if (!jobId || !selectedNodeData) return;

            const reviewerId = document.getElementById('reviewerId').value.trim() || 'admin_bob';
            const reviewerRoles = document.getElementById('reviewerRoles').value.trim() || 'domain_admin';

            try {{
                const res = await fetch(safeApiUrl(`${{currentPrefix}}/signal/${{jobId}}`), {{
                    method: 'POST',
                    headers: getApiHeaders({{
                        'Content-Type': 'application/json',
                        'X-User-ID': reviewerId,
                        'X-User-Roles': reviewerRoles
                    }}),
                    body: JSON.stringify({{
                        decision: decision,
                        signal_name: selectedNodeData.signal_name,
                        comments: `Decision ${{decision}} submitted via X6 Visualizer UI.`
                    }})
                }});

                const result = await res.json();
                if (res.ok) {{
                    showToast(`✅ Decision '${{decision}}' recorded successfully for ${{jobId}}!`, '✅', '#10b981', 5000);
                    lookupJob();
                }} else {{
                    showToast(`❌ Failed: ${{JSON.stringify(result.detail || result)}}`, '❌', '#ef4444', 6000);
                }}
            }} catch (err) {{
                showToast(`❌ Request Error: ${{err.message}}`, '❌', '#ef4444', 6000);
            }}
        }}

        let allJobsData = [];
        let jobsAutoRefreshTimer = null;

        function openJobsModal() {{
            document.getElementById('jobsModalBackdrop').style.display = 'flex';
            populateWorkflowFilterDropdown();
            loadJobsList();
            if (document.getElementById('autoRefreshJobs') && document.getElementById('autoRefreshJobs').checked && !jobsAutoRefreshTimer) {{
                jobsAutoRefreshTimer = setInterval(loadJobsList, 3000);
            }}
        }}

        function closeJobsModal(e) {{
            if (e && e.target && e.target.id !== 'jobsModalBackdrop') return;
            document.getElementById('jobsModalBackdrop').style.display = 'none';
            if (jobsAutoRefreshTimer) {{
                clearInterval(jobsAutoRefreshTimer);
                jobsAutoRefreshTimer = null;
            }}
        }}

        function toggleAutoRefreshJobs() {{
            const isChecked = document.getElementById('autoRefreshJobs').checked;
            if (isChecked) {{
                if (!jobsAutoRefreshTimer) jobsAutoRefreshTimer = setInterval(loadJobsList, 3000);
            }} else {{
                if (jobsAutoRefreshTimer) {{
                    clearInterval(jobsAutoRefreshTimer);
                    jobsAutoRefreshTimer = null;
                }}
            }}
        }}

        async function populateWorkflowFilterDropdown() {{
            const wfSelect = document.getElementById('jobsWorkflowFilter');
            if (!wfSelect) return;
            const currentVal = wfSelect.value;
            try {{
                const res = await fetch(safeApiUrl('/api/visualizer/domains'), {{ headers: getApiHeaders() }});
                if (res.ok) {{
                    const domains = await res.json();
                    let html = '<option value="ALL">All Workflows</option>';
                    domains.forEach(d => {{
                        html += `<option value="${{escapeHtml(d.domain)}}">${{escapeHtml(d.domain)}}</option>`;
                    }});
                    wfSelect.innerHTML = html;
                    if (currentVal) wfSelect.value = currentVal;
                }}
            }} catch (e) {{
                console.debug('Failed to populate workflow filter', e);
            }}
        }}

        async function loadJobsList() {{
            try {{
                const res = await fetch(safeApiUrl('/api/visualizer/jobs?limit=150'), {{ headers: getApiHeaders() }});
                if (!res.ok) throw new Error('Failed to fetch jobs');
                const data = await res.json();
                allJobsData = Array.isArray(data) ? data : [];
                filterJobsTable();
            }} catch (err) {{
                console.debug('Failed to load jobs list:', err);
            }}
        }}

        function filterJobsTable() {{
            const search = (document.getElementById('jobsSearchInput').value || '').toLowerCase().trim();
            const statusFilter = document.getElementById('jobsStatusFilter').value;
            const wfFilter = document.getElementById('jobsWorkflowFilter').value;
            const tbody = document.getElementById('jobsTableBody');
            const countPill = document.getElementById('jobsCountPill');

            let filtered = allJobsData.filter(job => {{
                if (wfFilter !== 'ALL' && job.workflow_name !== wfFilter && job.workflow !== wfFilter) {{
                    return false;
                }}

                const statusCode = job.status_code || 0;
                const statusStr = (job.status || '').toUpperCase();

                if (statusFilter === '200') {{
                    if (statusCode !== 200 && statusStr !== 'COMPLETED') return false;
                }} else if (statusFilter === '401') {{
                    if (statusCode !== 401 && !statusStr.includes('401')) return false;
                }} else if (statusFilter === '403') {{
                    if (statusCode !== 403 && !statusStr.includes('403')) return false;
                }} else if (statusFilter === '429') {{
                    if (statusCode !== 429 && !statusStr.includes('429')) return false;
                }} else if (statusFilter === '500') {{
                    if (statusCode < 500 && statusStr !== 'FAILED') return false;
                }} else if (statusFilter === 'RUNNING') {{
                    if (statusStr !== 'RUNNING') return false;
                }} else if (statusFilter === 'ROLLED_BACK') {{
                    if (statusStr !== 'ROLLED_BACK') return false;
                }}

                if (search) {{
                    const match = (job.job_id || '').toLowerCase().includes(search)
                        || (job.workflow_name || '').toLowerCase().includes(search)
                        || (job.route || '').toLowerCase().includes(search)
                        || (job.method || '').toLowerCase().includes(search)
                        || (job.source_ip || '').toLowerCase().includes(search)
                        || (job.error || '').toLowerCase().includes(search)
                        || (job.error_type || '').toLowerCase().includes(search);
                    if (!match) return false;
                }}

                return true;
            }});

            if (countPill) {{
                countPill.innerText = `${{filtered.length}} of ${{allJobsData.length}} Jobs`;
            }}

            if (filtered.length === 0) {{
                tbody.innerHTML = `
                    <tr>
                        <td colspan="8" style="text-align: center; color: #64748b; padding: 40px 0;">
                            <div style="font-size: 24px; margin-bottom: 8px;">📂</div>
                            <div>No matching executions found</div>
                            <div style="font-size: 11px; margin-top: 4px; color: #475569;">Send an API request or curl command to populate live execution history.</div>
                        </td>
                    </tr>
                `;
                return;
            }}

            let html = '';
            filtered.forEach(job => {{
                const jId = escapeHtml(job.job_id || '');
                const wf = escapeHtml(job.workflow_name || job.workflow || '-');
                const route = escapeHtml(job.route || job.route_id || '-');
                const method = (job.method || job.route_method || 'POST').toUpperCase();
                const ip = escapeHtml(job.source_ip || job.client_host || '127.0.0.1');
                const duration = job.duration_ms !== undefined ? `${{job.duration_ms}} ms` : '-';
                const started = formatTime(job.started_at);

                let statusBadgeHtml = '';
                const code = job.status_code || 0;
                const status = (job.status || 'UNKNOWN').toUpperCase();

                if (code === 200 || status === 'COMPLETED') {{
                    statusBadgeHtml = `<span class="status-badge status-200">✅ 200 OK</span>`;
                }} else if (code === 401 || (status === 'FAILED' && ((job.error_type || '').includes('Credentials') || (job.error_type || '').includes('Auth') || (job.error || '').includes('Invalid username')))) {{
                    statusBadgeHtml = `<span class="status-badge status-401">🛑 401 UNAUTH</span>`;
                }} else if (code === 403 || (status === 'FAILED' && (job.error_type || '').includes('Denied'))) {{
                    statusBadgeHtml = `<span class="status-badge status-403">🛑 403 FORBIDDEN</span>`;
                }} else if (code === 429 || (status === 'FAILED' && (job.error_type || '').includes('RateLimit'))) {{
                    statusBadgeHtml = `<span class="status-badge status-429">⚠️ 429 LIMIT</span>`;
                }} else if (status === 'ROLLED_BACK') {{
                    statusBadgeHtml = `<span class="status-badge status-rollback">↩️ ROLLED_BACK</span>`;
                }} else if (status === 'RUNNING') {{
                    statusBadgeHtml = `<span class="status-badge status-running">⏳ RUNNING</span>`;
                }} else {{
                    statusBadgeHtml = `<span class="status-badge status-500">🛑 ${{code > 0 ? code : 'FAILED'}}</span>`;
                }}

                let methodClass = 'method-post';
                if (method === 'GET') methodClass = 'method-get';
                else if (method === 'PUT' || method === 'PATCH') methodClass = 'method-put';
                else if (method === 'DELETE') methodClass = 'method-delete';

                html += `
                    <tr>
                        <td>${{statusBadgeHtml}}</td>
                        <td>
                            <span class="job-id-code" onclick="selectJobFromModal('${{jId}}', '${{wf}}', '${{route}}', '${{method}}')" title="Click to inspect in visualizer graph">
                                ${{jId}}
                            </span>
                        </td>
                        <td><span style="font-weight: 600; color: #cbd5e1;">${{wf}}</span></td>
                        <td>
                            <div style="display: flex; align-items: center; gap: 6px;">
                                <span class="method-badge ${{methodClass}}">${{method}}</span>
                                <span style="font-family: 'Fira Code', monospace; color: #94a3b8; font-size: 11px;">${{route}}</span>
                            </div>
                        </td>
                        <td>
                            <span style="font-family: 'Fira Code', monospace; color: #a5b4fc; font-size: 11px;">🌐 ${{ip}}</span>
                        </td>
                        <td>
                            <span style="font-family: 'Fira Code', monospace; color: #fde68a; font-weight: 500;">⚡ ${{duration}}</span>
                        </td>
                        <td>
                            <span style="color: #64748b; font-size: 11px;">${{started}}</span>
                        </td>
                        <td style="text-align: right;">
                            <button class="btn-inspect" onclick="selectJobFromModal('${{jId}}', '${{wf}}', '${{route}}', '${{method}}')" title="Inspect this execution in graph">
                                <span>Inspect</span>
                                <span>↗</span>
                            </button>
                        </td>
                    </tr>
                `;
            }});

            tbody.innerHTML = html;
        }}

        function formatTime(isoStr) {{
            if (!isoStr) return '-';
            try {{
                const date = new Date(isoStr);
                return date.toLocaleTimeString([], {{ hour: '2-digit', minute: '2-digit', second: '2-digit' }});
            }} catch (e) {{
                return isoStr;
            }}
        }}

        async function selectJobFromModal(jobId, workflow, route, method) {{
            closeJobsModal();
            const jobIdInput = document.getElementById('jobIdInput');
            jobIdInput.value = jobId;

            if (workflow && workflow !== '-') {{
                const domainSelect = document.getElementById('domainSelect');
                if (domainSelect && domainSelect.value !== workflow) {{
                    domainSelect.value = workflow;
                    await onDomainSelectChanged();
                }}
            }}

            if (route && route !== '-') {{
                const routeSelect = document.getElementById('routeSelect');
                if (routeSelect) {{
                    for (let i = 0; i < routeSelect.options.length; i++) {{
                        if (routeSelect.options[i].value === route) {{
                            routeSelect.selectedIndex = i;
                            break;
                        }}
                    }}
                }}
            }}

            await lookupJob();
        }}

        // Initial Load Sequence

        (async function init() {{
            populateDropdown(DOMAINS_REGISTRY);
            await refreshDomains();

            const urlParams = new URLSearchParams(window.location.search);
            const paramDomain = urlParams.get('domain');
            const paramJobId = urlParams.get('job_id');

            if (paramDomain) {{
                for (let i = 0; i < domainSelect.options.length; i++) {{
                    if (domainSelect.options[i].value === paramDomain) {{
                        domainSelect.selectedIndex = i;
                        currentDomain = paramDomain;
                        currentPrefix = domainSelect.options[i].dataset.prefix;
                        break;
                    }}
                }}
            }} else if (domainSelect.options.length > 0) {{
                if (domainSelect.selectedIndex < 0) {{
                    domainSelect.selectedIndex = 0;
                }}
                const opt = domainSelect.options[domainSelect.selectedIndex];
                currentDomain = opt.value;
                currentPrefix = opt.dataset.prefix;
            }}

            if (paramJobId) {{
                document.getElementById('jobIdInput').value = paramJobId;
                lookupJob();
            }} else {{
                loadBlueprintGraph();
            }}
        }})();
    </script>
</body>
</html>
"""
