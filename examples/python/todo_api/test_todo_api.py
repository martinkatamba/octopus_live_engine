import os
import sys
import pytest
from fastapi.testclient import TestClient

# Add python source and example dir to sys.path
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "../../../python/src")))
sys.path.insert(0, os.path.abspath(os.path.dirname(__file__)))

try:
    from octopus_live_engine.examples.python.todo_api.server import app, engine, TODOS_DB
except ImportError:
    from server import app, engine, TODOS_DB


@pytest.fixture(autouse=True)
def clean_db():
    TODOS_DB.clear()
    yield
    TODOS_DB.clear()


def test_todo_api_lifecycle():
    client = TestClient(app)

    # 1. Health check
    res_health = client.get("/health")
    assert res_health.status_code == 200
    assert res_health.json()["status"] == "UP"

    # 2. CREATE TODO: POST /api/v1/todos
    post_payload = {
        "title": "Buy groceries",
        "description": "Milk, eggs, and sourdough bread"
    }
    res_create = client.post(
        "/api/v1/todos",
        json=post_payload,
        headers={"X-Job-ID": "job_todo_create_001"}
    )
    assert res_create.status_code == 201
    assert res_create.headers["X-Job-ID"] == "job_todo_create_001"
    assert res_create.headers["X-Execution-Status"] == "COMPLETED"
    
    created_todo = res_create.json()
    todo_id = created_todo["id"]
    assert todo_id.startswith("todo_")
    assert created_todo["title"] == "Buy groceries"
    assert created_todo["description"] == "Milk, eggs, and sourdough bread"
    assert created_todo["completed"] is False
    assert "created_at" in created_todo
    # Verify internal state keys were filtered out by is_output and output_model
    assert "validated_title" not in created_todo

    # 3. LIST TODOS: GET /api/v1/todos
    res_list = client.get("/api/v1/todos", headers={"X-Job-ID": "job_todo_list_001"})
    assert res_list.status_code == 200
    list_body = res_list.json()
    assert list_body["total_count"] == 1
    assert len(list_body["todos"]) == 1
    assert list_body["todos"][0]["id"] == todo_id

    # 4. GET SINGLE TODO: GET /api/v1/todos/{todo_id}
    res_single = client.get(f"/api/v1/todos/{todo_id}", headers={"X-Job-ID": "job_todo_single_001"})
    assert res_single.status_code == 200
    single_body = res_single.json()
    assert single_body["id"] == todo_id
    assert single_body["title"] == "Buy groceries"

    # 5. UPDATE TODO: PUT /api/v1/todos/{todo_id}
    update_payload = {
        "title": "Buy organic groceries",
        "completed": True
    }
    res_update = client.put(
        f"/api/v1/todos/{todo_id}",
        json=update_payload,
        headers={"X-Job-ID": "job_todo_update_001"}
    )
    assert res_update.status_code == 200
    updated_body = res_update.json()
    assert updated_body["id"] == todo_id
    assert updated_body["title"] == "Buy organic groceries"
    assert updated_body["completed"] is True

    # 6. DELETE TODO: DELETE /api/v1/todos/{todo_id}
    res_delete = client.delete(f"/api/v1/todos/{todo_id}", headers={"X-Job-ID": "job_todo_delete_001"})
    assert res_delete.status_code == 200
    del_body = res_delete.json()
    assert del_body["id"] == todo_id
    assert del_body["deleted"] is True
    assert del_body["message"] == "Todo deleted successfully"

    # 7. Verify empty list after delete
    res_list_after = client.get("/api/v1/todos")
    assert res_list_after.status_code == 200
    assert res_list_after.json()["total_count"] == 0

    # 8. Visualizer Diagram Compilation for POST route with job tracing
    diagram = engine.get_visualizer_use_case().compile_x6_diagram(
        "todo_api_service",
        job_id="job_todo_create_001",
        route="/api/v1/todos",
        method="POST"
    )
    assert diagram is not None
    assert diagram["workflow_name"] == "todo_api_service"
    assert diagram["overall_status"] == "COMPLETED"
    assert "nodes" in diagram
    assert len(diagram["nodes"]) == 5  # START, step_validate, step_persist, step_package, END
    
    # Check that is_output is reflected on step_package_todo_response
    package_node = next(n for n in diagram["nodes"] if n["id"] == "node__step_package_todo_response")
    assert package_node["data"]["is_output"] is True
    assert package_node["data"]["status"] == "COMPLETED"
    assert package_node["data"]["executed"] is True
