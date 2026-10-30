import os
from typing import Any, Dict, List, Optional
import yaml
from ...domain.models.models import (
    ApiEndpointConfig,
    ErrorHandlingConfig,
    GlobalExceptionHandlerConfig,
    MaskingConfig,
    MiddlewareConfig,
    MiddlewareExceptionConfig,
    StepCondition,
    TelemetryConfig,
    WorkflowBlueprint,
    WorkflowStep,
)
from ...domain.ports.outbound import IBlueprintRepository
from ...domain.services.condition_evaluator import validate_blueprint_conditions


class YamlBlueprintRepository(IBlueprintRepository):
    """Outbound adapter loading declarative YAML blueprints from filesystem."""

    def __init__(self, config_dir: str = "config"):
        self.config_dir = config_dir
        self._blueprints: Dict[str, WorkflowBlueprint] = {}
        self.load_directory(config_dir)

    def load_directory(self, config_dir: str) -> None:
        if not os.path.exists(config_dir) or not os.path.isdir(config_dir):
            return

        for fname in os.listdir(config_dir):
            if fname.endswith(".yaml") or fname.endswith(".yml"):
                fpath = os.path.join(config_dir, fname)
                try:
                    bp = self._parse_file(fpath)
                    if bp:
                        self.save(bp)
                except Exception as ex:
                    print(f"Failed to parse blueprint {fname}: {ex}")

    def load_file(self, file_path: str) -> Optional[WorkflowBlueprint]:
        if not os.path.exists(file_path) or not os.path.isfile(file_path):
            return None
        bp = self._parse_file(file_path)
        if bp:
            self.save(bp)
        return bp

    def get(self, workflow_name: str) -> Optional[WorkflowBlueprint]:
        return self._blueprints.get(workflow_name)

    def list_all(self) -> List[WorkflowBlueprint]:
        return list(self._blueprints.values())

    def save(self, blueprint: WorkflowBlueprint) -> None:
        # A rule that does not parse is refused here, never evaluated (D2).
        validate_blueprint_conditions(blueprint)
        self._blueprints[blueprint.workflow_name] = blueprint

    def _parse_file(self, fpath: str) -> Optional[WorkflowBlueprint]:
        with open(fpath, "r", encoding="utf-8") as f:
            raw = yaml.safe_load(f)
        if not raw or not isinstance(raw, dict):
            return None

        wf_name = raw.get("workflow_name") or os.path.splitext(os.path.basename(fpath))[0]
        steps = []
        for s in raw.get("steps", []):
            step = self._parse_step(s)
            steps.append(step)

        allowed_roles = raw.get("allowed_initiator_roles") or raw.get("allowed_roles") or []
        if isinstance(allowed_roles, str):
            allowed_roles = [allowed_roles]

        # Parse declared middlewares
        middlewares: List[MiddlewareConfig] = []
        for m in raw.get("middlewares", []):
            if isinstance(m, str):
                middlewares.append(MiddlewareConfig(name=m))
            elif isinstance(m, dict):
                raw_type = m.get("type", "pre-request")
                name = m.get("name") or (raw_type if raw_type not in ("pre-request", "post-request") else "") or m.get("class") or ""
                mw_type = raw_type if raw_type in ("pre-request", "post-request") else "pre-request"
                activity = m.get("activity")
                is_async = bool(m.get("async", m.get("is_async", False)))
                excludes = m.get("excludes") or m.get("exclude") or m.get("exclude_routes") or []
                includes = m.get("includes") or m.get("include") or m.get("include_routes") or []
                raw_exceptions = m.get("exceptions") or m.get("exception_handlers") or m.get("exception_mappings") or []
                parsed_exceptions = self._parse_middleware_exceptions(raw_exceptions)
                cfg = dict(m.get("config") or m.get("params") or {})
                # Also include top-level custom arguments declared on the middleware entry
                reserved_keys = {
                    "name", "type", "class", "activity", "async", "is_async",
                    "excludes", "exclude", "exclude_routes",
                    "includes", "include", "include_routes", "exceptions",
                    "exception_handlers", "exception_mappings", "config", "params"
                }
                for k, v in m.items():
                    if k not in reserved_keys and k not in cfg:
                        cfg[k] = v
                if isinstance(excludes, str):
                    excludes = [excludes]
                if isinstance(includes, str):
                    includes = [includes]
                middlewares.append(MiddlewareConfig(
                    name=name,
                    type=mw_type,
                    activity=activity,
                    is_async=is_async,
                    excludes=list(excludes),
                    includes=list(includes),
                    exceptions=parsed_exceptions,
                    config=cfg
                ))

        # Parse API endpoints (supports list, single object, or structured {base_prefix, version, endpoints})
        api_endpoints: List[ApiEndpointConfig] = []
        raw_api = raw.get("api")
        wf_version = str(raw.get("version", "1.0.0"))
        if not wf_version.startswith("v") and not wf_version.startswith("/"):
            wf_ver_str = f"v{wf_version}"
        else:
            wf_ver_str = wf_version

        if isinstance(raw_api, dict):
            if "endpoints" in raw_api or "routes" in raw_api:
                base_pfx = raw_api.get("base_prefix") or raw_api.get("base_url") or "/api"
                api_ver = str(raw_api.get("version") or wf_ver_str)
                if not api_ver.startswith("v") and not api_ver.startswith("/"):
                    api_ver = f"v{api_ver}"
                computed_prefix = f"{base_pfx.rstrip('/')}/{api_ver.strip('/')}/{wf_name.strip('/')}"

                ep_list = raw_api.get("endpoints") or raw_api.get("routes") or []
                if isinstance(ep_list, list):
                    for ep in ep_list:
                        if isinstance(ep, dict):
                            parsed_ep = self._parse_api_endpoint(ep)
                            if parsed_ep.route and not parsed_ep.route.startswith("/api/") and not parsed_ep.route.startswith(computed_prefix):
                                parsed_ep.route = f"{computed_prefix}/{parsed_ep.route.lstrip('/')}"
                            api_endpoints.append(parsed_ep)
            else:
                api_endpoints.append(self._parse_api_endpoint(raw_api))
        elif isinstance(raw_api, list):
            for ep in raw_api:
                if isinstance(ep, dict):
                    parsed_ep = self._parse_api_endpoint(ep)
                    api_endpoints.append(parsed_ep)

        # Fallback top-level steps to first API endpoint steps if not explicitly defined at root
        if not steps and api_endpoints and api_endpoints[0].steps:
            steps = list(api_endpoints[0].steps)

        # Parse Masking, Telemetry, and Redaction
        masking_cfg = None
        raw_masking = raw.get("masking")
        if raw_masking:
            if isinstance(raw_masking, list):
                masking_cfg = MaskingConfig(enabled=True, fields=raw_masking, mask_value="********")
            elif isinstance(raw_masking, dict):
                masking_cfg = MaskingConfig(
                    enabled=bool(raw_masking.get("enabled", True)),
                    fields=raw_masking.get("fields") or raw_masking.get("mask_fields") or [],
                    mask_value=raw_masking.get("mask_value") or raw_masking.get("mask") or "********"
                )

        telemetry_cfg = None
        raw_telemetry = raw.get("telemetry")
        if raw_telemetry and isinstance(raw_telemetry, dict):
            t_mask_fields = raw_telemetry.get("mask_fields") or []
            t_masking = None
            if "masking" in raw_telemetry:
                tm = raw_telemetry["masking"]
                if isinstance(tm, list):
                    t_masking = MaskingConfig(enabled=True, fields=tm, mask_value="********")
                elif isinstance(tm, dict):
                    t_masking = MaskingConfig(
                        enabled=bool(tm.get("enabled", True)),
                        fields=tm.get("fields") or tm.get("mask_fields") or [],
                        mask_value=tm.get("mask_value") or tm.get("mask") or "********"
                    )
            telemetry_cfg = TelemetryConfig(
                mask_fields=t_mask_fields,
                masking=t_masking
            )

        redaction_cfg = None
        raw_redaction = raw.get("redaction")
        if raw_redaction:
            if isinstance(raw_redaction, list):
                redaction_cfg = MaskingConfig(enabled=True, fields=raw_redaction, mask_value="********")
            elif isinstance(raw_redaction, dict):
                redaction_cfg = MaskingConfig(
                    enabled=bool(raw_redaction.get("enabled", True)),
                    fields=raw_redaction.get("fields") or raw_redaction.get("mask_fields") or [],
                    mask_value=raw_redaction.get("mask_value") or raw_redaction.get("mask") or "********"
                )

        # Parse error_handling and global_exception_handler
        error_handling_cfg = None
        global_exc_cfg = None
        raw_eh = raw.get("error_handling") or raw.get("errors") or raw.get("exceptions")
        if isinstance(raw_eh, dict):
            raw_gh = raw_eh.get("global_handler") or raw_eh.get("global_exception_handler") or raw_eh.get("global") or raw_eh
            if isinstance(raw_gh, dict) and ("activity" in raw_gh or "code" in raw_gh or "message" in raw_gh):
                global_exc_cfg = GlobalExceptionHandlerConfig(
                    activity=raw_gh.get("activity"),
                    code=int(raw_gh.get("code") or raw_gh.get("status_code") or 500),
                    message=raw_gh.get("message"),
                    is_async=bool(raw_gh.get("async", raw_gh.get("is_async", False))),
                    include_stack_trace=bool(raw_gh.get("include_stack_trace", raw_gh.get("stack_trace", True))),
                    headers=dict(raw_gh.get("headers") or {})
                )
                error_handling_cfg = ErrorHandlingConfig(global_handler=global_exc_cfg)
        elif raw.get("global_exception_handler") and isinstance(raw.get("global_exception_handler"), dict):
            raw_gh = raw["global_exception_handler"]
            global_exc_cfg = GlobalExceptionHandlerConfig(
                activity=raw_gh.get("activity"),
                code=int(raw_gh.get("code") or raw_gh.get("status_code") or 500),
                message=raw_gh.get("message"),
                is_async=bool(raw_gh.get("async", raw_gh.get("is_async", False))),
                include_stack_trace=bool(raw_gh.get("include_stack_trace", raw_gh.get("stack_trace", True))),
                headers=dict(raw_gh.get("headers") or {})
            )
            error_handling_cfg = ErrorHandlingConfig(global_handler=global_exc_cfg)

        return WorkflowBlueprint(
            workflow_name=wf_name,
            version=str(raw.get("version", "1.0.0")),
            namespace=raw.get("namespace", "default"),
            description=raw.get("description", ""),
            mode=raw.get("mode", "sync"),
            compensation_mode=raw.get("compensation_mode", "sync"),
            allowed_initiator_roles=list(allowed_roles),
            middlewares=middlewares,
            steps=steps,
            api=api_endpoints,
            persistence=raw.get("persistence") or {},
            metadata=raw,
            masking=masking_cfg,
            telemetry=telemetry_cfg,
            redaction=redaction_cfg,
            error_handling=error_handling_cfg,
            global_exception_handler=global_exc_cfg
        )

    def _parse_middleware_exceptions(self, raw_exc: Any) -> List[MiddlewareExceptionConfig]:
        configs: List[MiddlewareExceptionConfig] = []
        if not raw_exc:
            return configs

        if isinstance(raw_exc, dict):
            for exc_name, val in raw_exc.items():
                if isinstance(val, dict):
                    is_async = bool(val.get("async") or val.get("is_async") or val.get("fire_and_forget") or val.get("background"))
                    configs.append(MiddlewareExceptionConfig(
                        exception=exc_name,
                        code=int(val.get("code") or val.get("status_code") or val.get("Code") or 400),
                        message=val.get("message") or val.get("detail") or val.get("msg") or val.get("Message"),
                        activity=val.get("activity") or val.get("handler_activity") or val.get("handler") or val.get("Activity"),
                        is_async=is_async,
                        headers=val.get("headers") or {}
                    ))
                elif isinstance(val, (int, str)):
                    code_val = int(val) if str(val).isdigit() else 400
                    msg_val = str(val) if not str(val).isdigit() else None
                    configs.append(MiddlewareExceptionConfig(
                        exception=exc_name,
                        code=code_val,
                        message=msg_val
                    ))
        elif isinstance(raw_exc, list):
            for item in raw_exc:
                if isinstance(item, dict):
                    if "exception" in item or "name" in item:
                        exc_name = item.get("exception") or item.get("name") or ""
                        is_async = bool(item.get("async") or item.get("is_async") or item.get("fire_and_forget") or item.get("background"))
                        configs.append(MiddlewareExceptionConfig(
                            exception=exc_name,
                            code=int(item.get("code") or item.get("status_code") or item.get("Code") or 400),
                            message=item.get("message") or item.get("detail") or item.get("msg") or item.get("Message"),
                            activity=item.get("activity") or item.get("handler_activity") or item.get("handler") or item.get("Activity"),
                            is_async=is_async,
                            headers=item.get("headers") or {}
                        ))
                    else:
                        for exc_name, val in item.items():
                            if isinstance(val, dict):
                                is_async = bool(val.get("async") or val.get("is_async") or val.get("fire_and_forget") or val.get("background"))
                                configs.append(MiddlewareExceptionConfig(
                                    exception=exc_name,
                                    code=int(val.get("code") or val.get("status_code") or val.get("Code") or 400),
                                    message=val.get("message") or val.get("detail") or val.get("msg") or val.get("Message"),
                                    activity=val.get("activity") or val.get("handler_activity") or val.get("handler") or val.get("Activity"),
                                    is_async=is_async,
                                    headers=val.get("headers") or {}
                                ))
                            elif isinstance(val, (int, str)):
                                code_val = int(val) if str(val).isdigit() else 400
                                msg_val = str(val) if not str(val).isdigit() else None
                                configs.append(MiddlewareExceptionConfig(
                                    exception=exc_name,
                                    code=code_val,
                                    message=msg_val
                                ))
                elif isinstance(item, str):
                    configs.append(MiddlewareExceptionConfig(exception=item))
        return configs

    def _parse_api_endpoint(self, raw_ep: dict) -> ApiEndpointConfig:
        ep_steps: List[WorkflowStep] = []
        for s in raw_ep.get("steps", []):
            ep_steps.append(self._parse_step(s))

        tags = raw_ep.get("tags") or []
        if isinstance(tags, str):
            tags = [tags]

        return ApiEndpointConfig(
            route=raw_ep.get("route") or raw_ep.get("path", ""),
            method=str(raw_ep.get("method", "POST")).upper(),
            input_model=raw_ep.get("input_model") or raw_ep.get("request_model"),
            output_model=raw_ep.get("output_model") or raw_ep.get("response_model"),
            status_code=int(raw_ep.get("status_code", 200)),
            summary=raw_ep.get("summary"),
            description=raw_ep.get("description"),
            tags=list(tags),
            steps=ep_steps
        )

    def _parse_step(self, raw_step: dict) -> WorkflowStep:
        cond = None
        if "condition" in raw_step or "conditions" in raw_step or "branching_step" in raw_step:
            c = raw_step.get("condition") or raw_step.get("conditions") or {}
            then_raw = raw_step.get("then_steps") or raw_step.get("then") or (c.get("then_steps") if isinstance(c, dict) else []) or (c.get("then") if isinstance(c, dict) else []) or []
            else_raw = raw_step.get("else_steps") or raw_step.get("else") or (c.get("else_steps") if isinstance(c, dict) else []) or (c.get("else") if isinstance(c, dict) else []) or []
            then_steps = [self._parse_step(ts) for ts in then_raw]
            else_steps = [self._parse_step(es) for es in else_raw]

            rule = None
            field = None
            op = None
            val = None
            rules_list = []
            match_policy = "all"
            rule_type = None

            if isinstance(c, dict):
                rule = c.get("rule")
                field = c.get("field") or c.get("variable")
                op = c.get("operator")
                val = c.get("value")
                match_policy = c.get("match") or c.get("rule_type") or c.get("logic") or "all"
                rule_type = c.get("rule_type") or match_policy

                raw_rules = c.get("rules") or c.get("conditions")
                if isinstance(raw_rules, list):
                    rules_list = raw_rules
                    if not field and len(raw_rules) > 0:
                        first_sub = raw_rules[0]
                        if isinstance(first_sub, dict):
                            field = first_sub.get("variable") or first_sub.get("field")
                            op = first_sub.get("operator")
                            val = first_sub.get("value")
                            rule = first_sub.get("rule")

            cond = StepCondition(
                rule=rule,
                operator=op,
                field=field,
                value=val,
                rules=rules_list,
                match=match_policy,
                rule_type=rule_type,
                then_steps=then_steps,
                else_steps=else_steps
            )

        is_async = bool(raw_step.get("async") or raw_step.get("is_async") or raw_step.get("fire_and_forget"))
        compensate = bool(raw_step.get("compensate") or raw_step.get("rollback") or raw_step.get("trigger_compensation"))
        terminate = bool(raw_step.get("terminate") or raw_step.get("settle"))
        raw_exceptions = raw_step.get("exceptions") or raw_step.get("exception_handlers") or []
        parsed_exceptions = self._parse_middleware_exceptions(raw_exceptions) if raw_exceptions else []

        is_output = bool(raw_step.get("is_output") or raw_step.get("output") or raw_step.get("is_output_activity"))

        step_mask_fields = raw_step.get("mask_fields") or raw_step.get("mask") or []
        if isinstance(step_mask_fields, str):
            step_mask_fields = [step_mask_fields]

        step_masking = None
        raw_step_masking = raw_step.get("masking")
        if raw_step_masking:
            if isinstance(raw_step_masking, list):
                step_masking = MaskingConfig(enabled=True, fields=raw_step_masking, mask_value="********")
            elif isinstance(raw_step_masking, dict):
                step_masking = MaskingConfig(
                    enabled=bool(raw_step_masking.get("enabled", True)),
                    fields=raw_step_masking.get("fields") or raw_step_masking.get("mask_fields") or [],
                    mask_value=raw_step_masking.get("mask_value") or raw_step_masking.get("mask") or "********"
                )

        return WorkflowStep(
            id=raw_step.get("id", ""),
            activity=raw_step.get("activity"),
            description=raw_step.get("description"),
            timeout_seconds=raw_step.get("timeout_seconds", 30),
            delay_seconds=float(raw_step.get("delay_seconds") or 0.0),
            compensation=raw_step.get("compensation"),
            condition=cond,
            loop=raw_step.get("loop"),
            for_each=raw_step.get("for_each"),
            retry_policy=raw_step.get("retry_policy"),
            jump_to=raw_step.get("jump_to"),
            jump_to_step_index=raw_step.get("jump_to_step_index"),
            rewind_to_step_index=raw_step.get("rewind_to_step_index"),
            is_async=is_async,
            compensate=compensate,
            rollback=compensate,
            trigger_compensation=compensate,
            terminate=terminate,
            settle=terminate,
            reason=raw_step.get("reason"),
            dynamic_branching=raw_step.get("dynamic_branching"),
            is_output=is_output,
            pivot=bool(raw_step.get("pivot")),
            exceptions=parsed_exceptions,
            mask_fields=list(step_mask_fields),
            masking=step_masking
        )
