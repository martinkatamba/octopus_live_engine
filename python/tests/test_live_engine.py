import pytest
from pydantic import BaseModel
from typing import List
from starlette.testclient import TestClient

from octopus_live.domain.models.models import ApiEndpointConfig, MaskingConfig, StepCondition, TelemetryConfig, WorkflowBlueprint, WorkflowStep
from octopus_live.domain.ports.outbound import IActivityExecutor
from octopus_live.domain.services.masking_service import mask_payload
from octopus_live.engine import OctopusLiveEngine


class MockActivities:
    def calculate_subtotal(self, payload):
        items = payload.get("items", [])
        subtotal = sum(i.get("price", 0) * i.get("quantity", 1) for i in items)
        return {"subtotal": subtotal}

    def apply_promo_discount(self, payload):
        subtotal = payload.get("subtotal", 0.0)
        promo = payload.get("promo_code")
        discount = 15.0 if promo == "OCTOPUS15" else 0.0
        return {"discount_amount": discount, "discounted_subtotal": max(0.0, subtotal - discount)}

    def calculate_sales_tax(self, payload):
        amt = float(payload.get("discounted_subtotal", payload.get("subtotal", 0.0)))
        rate = 0.0825
        tax = round(amt * rate, 2)
        return {"sales_tax": tax, "final_total": round(amt + tax, 2)}

    def package_tax_response(self, payload):
        return {
            "subtotal": float(payload.get("subtotal", 0.0)),
            "tax_total": float(payload.get("sales_tax", 0.0)),
            "total": float(payload.get("final_total", 0.0))
        }


@pytest.mark.asyncio
async def test_live_engine_sync_execution():
    bp = WorkflowBlueprint(
        workflow_name="checkout_tax_calculator",
        steps=[
            WorkflowStep(id="step_calc_subtotal", activity="calculate_subtotal"),
            WorkflowStep(
                id="step_promo",
                activity="apply_promo_discount",
                condition=StepCondition(rule="promo_code != null")
            ),
            WorkflowStep(id="step_calc_tax", activity="calculate_sales_tax")
        ]
    )

    engine = OctopusLiveEngine()
    engine.blueprint_repo.save(bp)
    engine.register_activities(MockActivities())

    payload = {
        "job_id": "test-cart-99",
        "promo_code": "OCTOPUS15",
        "items": [
            {"id": "item-1", "price": 50.0, "quantity": 2},
            {"id": "item-2", "price": 25.0, "quantity": 1}
        ]
    }

    state = await engine.execute_sync("checkout_tax_calculator", payload)

    assert state.status.value == "COMPLETED"
    assert state.data["subtotal"] == 125.0
    assert state.data["discount_amount"] == 15.0
    assert state.data["discounted_subtotal"] == 110.0
    assert state.data["sales_tax"] == 9.08
    assert state.data["final_total"] == 119.08
    assert len(state.traces) == 3


@pytest.mark.asyncio
async def test_live_engine_visualizer_diagram():
    bp = WorkflowBlueprint(
        workflow_name="checkout_tax_calculator",
        steps=[
            WorkflowStep(id="step_1", activity="calculate_subtotal"),
            WorkflowStep(id="step_2", activity="calculate_sales_tax")
        ]
    )
    engine = OctopusLiveEngine()
    engine.blueprint_repo.save(bp)

    diagram = engine.compile_x6_diagram("checkout_tax_calculator")
    node_ids = [n["id"] for n in diagram["nodes"]]
    assert "node__start" in node_ids
    assert "node__step_1" in node_ids or "node__calculate_subtotal" in node_ids
    assert "node__end" in node_ids
    assert len(diagram["edges"]) >= 2


class AdvancedMockActivities:
    def __init__(self):
        self.async_called = False
        self.compensated = False

    def process_item(self, payload):
        item = payload.get("item", {})
        return {"processed_price": item.get("price", 0) * 1.1}

    def reserve_funds(self, payload):
        return {"funds_reserved": True, "hold_id": "HOLD-123"}

    def cancel_reservation(self, payload):
        self.compensated = True
        return {"funds_reserved": False, "reservation_cancelled": True}

    def async_audit_log(self, payload):
        self.async_called = True
        return {"audited": True}

    def failing_step(self, payload):
        raise ValueError("Simulated network timeout")


@pytest.mark.asyncio
async def test_live_engine_loop_and_async_activity():
    bp = WorkflowBlueprint(
        workflow_name="batch_processor",
        steps=[
            WorkflowStep(
                id="step_loop_items",
                activity="process_item",
                loop={"items_field": "line_items", "item_variable": "item"}
            ),
            WorkflowStep(
                id="step_async_audit",
                activity="async_audit_log",
                is_async=True
            ),
            WorkflowStep(
                id="step_settle_early",
                settle=True,
                reason="Batch processed successfully"
            ),
            WorkflowStep(
                id="step_unreachable",
                activity="calculate_sales_tax"
            )
        ]
    )

    activities = AdvancedMockActivities()
    engine = OctopusLiveEngine()
    engine.blueprint_repo.save(bp)
    engine.register_activities(activities)

    payload = {
        "job_id": "loop-test-101",
        "line_items": [
            {"name": "Widget A", "price": 10.0},
            {"name": "Widget B", "price": 20.0},
            {"name": "Widget C", "price": 30.0}
        ]
    }

    state = await engine.execute_sync("batch_processor", payload)

    assert state.status.value == "COMPLETED"
    assert "step_loop_items_results" in state.data
    assert len(state.data["step_loop_items_results"]) == 3
    # Settle stopped execution before unreachable step
    step_ids = [t.step_id for t in state.traces]
    assert "step_unreachable" not in step_ids
    assert "step_settle_early" in step_ids


@pytest.mark.asyncio
async def test_live_engine_saga_compensation_sync_mode():
    bp = WorkflowBlueprint(
        workflow_name="saga_order_flow",
        compensation_mode="sync",
        steps=[
            WorkflowStep(
                id="step_reserve",
                activity="reserve_funds",
                compensation="cancel_reservation"
            ),
            WorkflowStep(
                id="step_fail",
                activity="failing_step"
            )
        ]
    )

    activities = AdvancedMockActivities()
    engine = OctopusLiveEngine()
    engine.blueprint_repo.save(bp)
    engine.register_activities(activities)

    payload = {"job_id": "saga-fail-101", "amount": 250.0}
    state = await engine.execute_sync("saga_order_flow", payload)

    assert state.status.value == "ROLLED_BACK"
    assert activities.compensated is True
    assert state.data.get("reservation_cancelled") is True


# --- Declarative Model-Driven Multi-Route API Tests ---

class TaxItem(BaseModel):
    sku: str
    price: float
    quantity: int

class CheckoutTaxRequest(BaseModel):
    customer_country: str
    items: List[TaxItem]

class CheckoutTaxResponse(BaseModel):
    subtotal: float = 0.0
    tax_total: float = 0.0
    total: float = 0.0

class QuickTaxResponse(BaseModel):
    sales_tax: float
    final_total: float


def test_declarative_multi_route_api():
    # Define workflow with multiple API routes
    bp = WorkflowBlueprint(
        workflow_name="ecommerce_tax_service",
        version="1.0.0",
        steps=[
            WorkflowStep(id="default_subtotal", activity="calculate_subtotal"),
            WorkflowStep(id="default_tax", activity="calculate_sales_tax")
        ],
        api=[
            ApiEndpointConfig(
                route="/api/v1/checkout/tax",
                method="POST",
                input_model="CheckoutTaxRequest",
                output_model="CheckoutTaxResponse",
                status_code=200,
                summary="Calculate tax for cart",
                tags=["Checkout"],
                steps=[
                    WorkflowStep(id="ep_subtotal", activity="calculate_subtotal"),
                    WorkflowStep(id="ep_tax", activity="calculate_sales_tax"),
                    WorkflowStep(id="ep_package", activity="package_tax_response", is_output=True)
                ]
            ),
            ApiEndpointConfig(
                route="/api/v1/checkout/quick-tax",
                method="GET",
                input_model="QuickTaxRequest",
                output_model="QuickTaxResponse",
                status_code=200,
                summary="Quick tax quote",
                tags=["Checkout"],
                steps=[
                    WorkflowStep(id="quick_tax", activity="calculate_sales_tax", is_output=True)
                ]
            )
        ]
    )

    engine = OctopusLiveEngine()
    engine.blueprint_repo.save(bp)
    engine.register_activities(MockActivities())
    engine.register_model("CheckoutTaxRequest", CheckoutTaxRequest)
    engine.register_model("CheckoutTaxResponse", CheckoutTaxResponse)
    engine.register_model("QuickTaxResponse", QuickTaxResponse)

    app = engine.create_api_app(title="Test E-Commerce API")
    client = TestClient(app)

    # 1. Test POST /api/v1/checkout/tax with designated output activity
    cart_body = {
        "customer_country": "US",
        "items": [
            {"sku": "SKU-1", "price": 40.0, "quantity": 2},
            {"sku": "SKU-2", "price": 20.0, "quantity": 1}
        ]
    }
    resp = client.post("/api/v1/checkout/tax", json=cart_body, headers={"X-Job-ID": "custom-tx-001"})
    assert resp.status_code == 200
    assert resp.headers["X-Job-ID"] == "custom-tx-001"
    assert resp.headers["X-Execution-Status"] == "COMPLETED"
    assert "X-Execution-Duration-Ms" in resp.headers
    data = resp.json()
    assert data["subtotal"] == 100.0
    assert data["tax_total"] == 8.25
    assert data["total"] == 108.25

    # 2. Test GET /api/v1/checkout/quick-tax with query params
    get_resp = client.get("/api/v1/checkout/quick-tax?subtotal=200.0")
    assert get_resp.status_code == 200
    assert get_resp.headers["X-Job-ID"].startswith("job_")
    assert get_resp.headers["X-Execution-Status"] == "COMPLETED"
    get_data = get_resp.json()
    assert get_data["sales_tax"] == 16.5
    assert get_data["final_total"] == 216.5

    # 3. Test Visualizer Diagram per endpoint
    diagram_post = engine.compile_x6_diagram("ecommerce_tax_service", route="/api/v1/checkout/tax", method="POST")
    node_ids_post = [n["id"] for n in diagram_post["nodes"]]
    assert "node__ep_subtotal" in node_ids_post
    assert "node__ep_tax" in node_ids_post

    diagram_get = engine.compile_x6_diagram("ecommerce_tax_service", route="/api/v1/checkout/quick-tax", method="GET")
    node_ids_get = [n["id"] for n in diagram_get["nodes"]]
    assert "node__quick_tax" in node_ids_get


@pytest.mark.asyncio
async def test_live_engine_multi_rule_conditions():
    class TestMultiRuleActivities:
        def ExpressRoute(self, payload):
            return {"tier_assigned": "EXPRESS"}
        def StandardRoute(self, payload):
            return {"tier_assigned": "STANDARD"}

    bp = WorkflowBlueprint(
        workflow_name="multi_rule_pipeline",
        steps=[
            WorkflowStep(
                id="step_check_eligibility",
                condition=StepCondition(
                    match="all",
                    rules=[
                        {"field": "order_total", "operator": "gt", "value": 1000},
                        {"field": "customer_tier", "operator": "eq", "value": "VIP"}
                    ],
                    then_steps=[
                        WorkflowStep(id="step_express", activity="ExpressRoute")
                    ],
                    else_steps=[
                        WorkflowStep(id="step_standard", activity="StandardRoute")
                    ]
                )
            )
        ]
    )

    engine = OctopusLiveEngine()
    engine.blueprint_repo.save(bp)
    engine.register_activities(TestMultiRuleActivities())

    # Case 1: Both rules match -> ExpressRoute
    res1 = await engine.execute_sync("multi_rule_pipeline", {"order_total": 1500, "customer_tier": "VIP"})
    assert res1.data["tier_assigned"] == "EXPRESS"

    # Case 2: One rule fails (order_total <= 1000) -> StandardRoute
    res2 = await engine.execute_sync("multi_rule_pipeline", {"order_total": 500, "customer_tier": "VIP"})
    assert res2.data["tier_assigned"] == "STANDARD"

    # Case 3: Compound expression with `or`
    bp_compound = WorkflowBlueprint(
        workflow_name="compound_expr_pipeline",
        steps=[
            WorkflowStep(
                id="step_check_compound",
                condition=StepCondition(
                    rule="is_admin == true or security_level >= 5",
                    then_steps=[
                        WorkflowStep(id="step_express", activity="ExpressRoute")
                    ],
                    else_steps=[
                        WorkflowStep(id="step_standard", activity="StandardRoute")
                    ]
                )
            )
        ]
    )
    engine.blueprint_repo.save(bp_compound)
    res3 = await engine.execute_sync("compound_expr_pipeline", {"is_admin": False, "security_level": 7})
    assert res3.data["tier_assigned"] == "EXPRESS"


@pytest.mark.asyncio
async def test_live_engine_regex_conditions():
    class RegexTestActivities:
        def EmailValid(self, payload):
            return {"email_status": "VALID"}
        def EmailInvalid(self, payload):
            return {"email_status": "INVALID"}

    bp = WorkflowBlueprint(
        workflow_name="regex_pipeline",
        steps=[
            WorkflowStep(
                id="step_validate_email_pattern",
                condition=StepCondition(
                    field="user_email",
                    operator="matches",
                    value=r"^[a-zA-Z0-9_.+-]+@[a-zA-Z0-9-]+\.[a-zA-Z0-9-.]+$",
                    then_steps=[
                        WorkflowStep(id="step_valid", activity="EmailValid")
                    ],
                    else_steps=[
                        WorkflowStep(id="step_invalid", activity="EmailInvalid")
                    ]
                )
            )
        ]
    )

    engine = OctopusLiveEngine()
    engine.blueprint_repo.save(bp)
    engine.register_activities(RegexTestActivities())

    # 1. Valid email match
    res_valid = await engine.execute_sync("regex_pipeline", {"user_email": "alice@octopus.io"})
    assert res_valid.data["email_status"] == "VALID"

    # 2. Invalid email pattern
    res_invalid = await engine.execute_sync("regex_pipeline", {"user_email": "invalid-email-address"})
    assert res_invalid.data["email_status"] == "INVALID"


@pytest.mark.asyncio
async def test_jobs_list_view_and_telemetry_endpoints():
    bp = WorkflowBlueprint(
        workflow_name="checkout_tax_calculator",
        version="1.0.0",
        steps=[
            WorkflowStep(id="step_calc_subtotal", activity="calculate_subtotal"),
            WorkflowStep(id="step_calc_tax", activity="calculate_sales_tax")
        ],
        api=[
            ApiEndpointConfig(
                route="/checkout/calculate",
                method="POST",
                steps=[
                    WorkflowStep(id="step_calc_subtotal", activity="calculate_subtotal"),
                    WorkflowStep(id="step_calc_tax", activity="calculate_sales_tax")
                ]
            )
        ]
    )

    engine = OctopusLiveEngine()
    engine.blueprint_repo.save(bp)
    engine.register_activities(MockActivities())

    app = engine.create_api_app()
    client = TestClient(app)

    # 1. Execute an API request
    res = client.post(
        "/checkout/calculate",
        json={"items": [{"id": "item-1", "price": 40.0, "quantity": 2}]},
        headers={"X-Forwarded-For": "192.168.1.105"}
    )
    assert res.status_code == 200
    job_id = res.headers.get("X-Job-ID")
    assert job_id is not None

    # 2. Query /api/visualizer/jobs
    jobs_res = client.get("/api/visualizer/jobs")
    assert jobs_res.status_code == 200
    jobs = jobs_res.json()
    assert len(jobs) >= 1
    target = next((j for j in jobs if j["job_id"] == job_id), None)
    assert target is not None
    assert target["workflow_name"] == "checkout_tax_calculator"
    assert target["status"] == "COMPLETED"
    assert target["status_code"] == 200
    assert target["source_ip"] == "192.168.1.105"
    assert target["route"] == "/checkout/calculate"
    assert target["method"] == "POST"
    assert target["duration_ms"] >= 0.0

    # 3. Query /api/jobs
    api_jobs_res = client.get("/api/jobs")
    assert api_jobs_res.status_code == 200
    assert len(api_jobs_res.json()) >= 1

    # 4. Query workflow scoped /api/v1.0.0/checkout_tax_calculator/jobs
    wf_jobs_res = client.get("/api/v1.0.0/checkout_tax_calculator/jobs")
    assert wf_jobs_res.status_code == 200
    assert len(wf_jobs_res.json()) >= 1

    # 5. Visualizer HTML contains Jobs modal elements
    vis_res = client.get("/visualizer")
    assert vis_res.status_code == 200
    assert "jobsModalBackdrop" in vis_res.text
    assert "openJobsModal()" in vis_res.text
    assert "loadJobsList()" in vis_res.text


@pytest.mark.asyncio
async def test_sensitive_field_masking():
    # 1. Direct unit test of mask_payload
    raw_payload = {
        "username": "sarah_vip",
        "password": "supersecretpassword",
        "API_KEY": "sk_live_99999",
        "meta": {
            "client_ip": "127.0.0.1",
            "Secret": "nested_secret_val"
        },
        "tokens": [
            {
                "token_type": "Bearer",
                "TOKEN": "jwt.token.abc"
            }
        ]
    }

    mask_fields = ["password", "api_key", "secret", "token"]
    masked = mask_payload(raw_payload, mask_fields, "********")

    assert masked["username"] == "sarah_vip"
    assert masked["password"] == "********"
    assert masked["API_KEY"] == "********"
    assert masked["meta"]["Secret"] == "********"
    assert masked["meta"]["client_ip"] == "127.0.0.1"
    assert masked["tokens"][0]["TOKEN"] == "********"
    assert masked["tokens"][0]["token_type"] == "Bearer"

    # 2. Integration test: Live Engine execution with Blueprint masking
    engine = OctopusLiveEngine()

    received_password = None

    class AuthActivities:
        async def login_auth_activity(self, payload):
            nonlocal received_password
            received_password = payload.get("password")
            if received_password != "secret123":
                raise ValueError("invalid credentials")
            return {
                "access_token": "token_abc123xyz",
                "customer_id": "cust_001"
            }

        async def step_with_custom_mask(self, payload):
            return {"processed": True}

    engine.register_activities(AuthActivities())

    bp = WorkflowBlueprint(
        workflow_name="masked_auth_pipeline",
        version="1.0.0",
        mode="sync",
        masking=MaskingConfig(
            enabled=True,
            fields=["password", "access_token"],
            mask_value="********"
        ),
        steps=[
            WorkflowStep(id="step_login", activity="login_auth_activity"),
            WorkflowStep(id="step_custom_pin", activity="step_with_custom_mask", mask_fields=["pin_code"])
        ]
    )
    engine.blueprint_repo.save(bp)

    state = await engine.execute_sync("masked_auth_pipeline", {
        "username": "sarah_vip",
        "password": "secret123",
        "pin_code": "4321"
    })

    assert state.status.value == "COMPLETED"
    # Activity received real unmasked credentials
    assert received_password == "secret123"

    assert len(state.traces) == 2
    login_trace = state.traces[0]
    assert login_trace.input_payload["password"] == "********"
    assert login_trace.input_payload["username"] == "sarah_vip"
    assert login_trace.output_payload["access_token"] == "********"
    assert login_trace.output_payload["customer_id"] == "cust_001"

    custom_trace = state.traces[1]
    assert custom_trace.input_payload["pin_code"] == "********"

    # 3. Visualizer compile_x6_diagram test
    diagram = engine.compile_x6_diagram("masked_auth_pipeline", state.job_id)
    assert diagram is not None
    nodes = diagram.get("nodes", [])
    login_node = next((n["data"] for n in nodes if n.get("data", {}).get("step_id") == "step_login"), None)
    assert login_node is not None
    assert login_node["input_payload"]["password"] == "********"
    assert login_node["input_payload"]["username"] == "sarah_vip"


@pytest.mark.asyncio
async def test_relational_sql_state_store_dialects():
    from octopus_live.adapters.outbound.relational_store import (
        RelationalSqlStateStore, PostgresDialect, MySqlDialect, SqlServerDialect, SqliteDialect
    )
    from octopus_live.adapters.outbound.store_factory import StoreFactory
    from octopus_live.domain.models.models import ExecutionState, ExecutionStatus, StepExecutionTrace

    # 1. Dialect SQL Validation
    pg = PostgresDialect()
    assert "ON CONFLICT (job_id) DO UPDATE" in pg.get_upsert_execution_state_sql()
    assert pg.placeholder(1) == "%s"  # psycopg2 placeholders, not asyncpg's $1

    my = MySqlDialect()
    assert "ON DUPLICATE KEY UPDATE" in my.get_upsert_execution_state_sql()
    assert my.placeholder(1) == "%s"  # pymysql placeholders

    ms = SqlServerDialect()
    assert "MERGE INTO execution_states" in ms.get_upsert_execution_state_sql()

    sq = SqliteDialect()
    assert "ON CONFLICT(job_id) DO UPDATE" in sq.get_upsert_execution_state_sql()

    # 2. StoreFactory creation
    store_sqlite = StoreFactory.create("sqlite", {"db_path": ":memory:"})
    assert isinstance(store_sqlite, RelationalSqlStateStore)

    store_pg = StoreFactory.create("postgres", {"dsn": "postgresql://user:pass@localhost:5432/db"})
    assert isinstance(store_pg, RelationalSqlStateStore)
    assert store_pg.dialect.name == "postgres"

    store_my = StoreFactory.create("mysql", {"dsn": "mysql://user:pass@localhost:3306/db"})
    assert isinstance(store_my, RelationalSqlStateStore)
    assert store_my.dialect.name == "mysql"

    store_ms = StoreFactory.create("sqlserver", {"dsn": "sqlserver://sa:pass@localhost:1433"})
    assert isinstance(store_ms, RelationalSqlStateStore)
    assert store_ms.dialect.name == "sqlserver"

    # 3. Functional Execution with In-Memory Relational SQLite
    state = ExecutionState(
        job_id="job-relational-test-101",
        workflow_name="order_fulfillment",
        status=ExecutionStatus.COMPLETED,
        route="/api/v1/orders/checkout",
        method="POST",
        data={"sku": "ITEM-1", "qty": 2},
        output_data={"order_id": "ORD-12345", "total": 99.99},
        traces=[
            StepExecutionTrace(
                step_id="step_verify",
                activity_name="verify_order",
                status="COMPLETED",
                input_payload={"sku": "ITEM-1"},
                output_payload={"verified": True},
                duration_ms=1.25
            )
        ]
    )

    await store_sqlite.save_state(state)

    fetched = await store_sqlite.get_state("job-relational-test-101")
    assert fetched is not None
    assert fetched.job_id == "job-relational-test-101"
    assert fetched.workflow_name == "order_fulfillment"
    assert fetched.status == ExecutionStatus.COMPLETED
    assert fetched.output_data["order_id"] == "ORD-12345"
    assert len(fetched.traces) == 1
    assert fetched.traces[0].activity_name == "verify_order"
    assert fetched.traces[0].duration_ms == 1.25

    listed = await store_sqlite.list_states("order_fulfillment", 10)
    assert len(listed) == 1
    assert listed[0].job_id == "job-relational-test-101"







def test_declared_route_returns_masked_fields_to_the_caller(tmp_path):
    """D5: a declared route that issues a masked field (a login returning
    access_token) responds with the real value; the job itself stays masked."""
    bp_file = tmp_path / "tokens.yaml"
    bp_file.write_text(
        "workflow_name: tokens\nversion: \"1.0.0\"\nmasking:\n  fields: [access_token]\n"
        "api:\n  - route: /api/v1.0.0/tokens/login\n    method: POST\n    steps:\n"
        "      - id: issue\n        activity: Issue\n        is_output: true\n"
    )
    engine = OctopusLiveEngine()
    engine.register_activity("Issue", lambda payload: {"access_token": "tok-123"})
    engine.load_blueprint_file(str(bp_file))
    client = TestClient(engine.create_api_app())

    res = client.post("/api/v1.0.0/tokens/login", json={})
    assert res.status_code == 200, res.text
    assert res.json()["access_token"] == "tok-123"

    import asyncio, json
    stored = asyncio.run(engine.get_execution_state(res.headers["X-Job-ID"]))
    assert "tok-123" not in json.dumps(stored, default=vars)
