"""
Declarative Synchronous Todo REST API powered by OCTOPUS Live Engine
"""

import os
import sys
import uuid
from datetime import datetime, timezone
from typing import Any, Dict, List, Optional
from pydantic import BaseModel, Field

from octopus_live import LiveEngine


# ==========================================
# 1. Domain Models (Pydantic)
# ==========================================

class CreateTodoRequest(BaseModel):
    title: str = Field(..., description="Title of the task")
    description: Optional[str] = Field("", description="Optional details about the task")


class UpdateTodoRequest(BaseModel):
    title: Optional[str] = Field(None, description="Updated title")
    completed: Optional[bool] = Field(None, description="Completion status")


class TodoResponse(BaseModel):
    id: str
    title: str
    description: Optional[str] = ""
    completed: bool = False
    created_at: str


class TodoListResponse(BaseModel):
    todos: List[TodoResponse]
    total_count: int


class DeleteTodoResponse(BaseModel):
    id: str
    deleted: bool
    message: str


# ==========================================
# 2. In-Memory Store & Engine Setup
# ==========================================

TODOS_DB: Dict[str, Dict[str, Any]] = {}
engine = LiveEngine()

# Register Request / Response Models
engine.register_model("CreateTodoRequest", CreateTodoRequest)
engine.register_model("UpdateTodoRequest", UpdateTodoRequest)
engine.register_model("TodoResponse", TodoResponse)
engine.register_model("TodoListResponse", TodoListResponse)
engine.register_model("DeleteTodoResponse", DeleteTodoResponse)


# ==========================================
# 3. Register Business Activities
# ==========================================

@engine.activity("ValidateTodoInput")
def validate_todo_input(payload: Dict[str, Any]) -> Dict[str, Any]:
    title = str(payload.get("title", "")).strip()
    if not title:
        raise ValueError("Todo 'title' cannot be empty.")
    return {
        "validated_title": title,
        "validated_desc": str(payload.get("description", "")).strip()
    }


@engine.activity("PersistTodo")
def persist_todo(payload: Dict[str, Any]) -> Dict[str, Any]:
    todo_id = f"todo_{uuid.uuid4().hex[:8]}"
    created_at = datetime.now(timezone.utc).isoformat()
    
    item = {
        "id": todo_id,
        "title": payload["validated_title"],
        "description": payload.get("validated_desc", ""),
        "completed": False,
        "created_at": created_at
    }
    TODOS_DB[todo_id] = item
    return {"todo_item": item}


@engine.activity("FetchTodos")
def fetch_todos(payload: Dict[str, Any]) -> Dict[str, Any]:
    filter_status = payload.get("status")
    items = list(TODOS_DB.values())
    
    if filter_status == "completed":
        items = [t for t in items if t["completed"]]
    elif filter_status == "active":
        items = [t for t in items if not t["completed"]]
        
    return {
        "items": items,
        "count": len(items)
    }


@engine.activity("FindTodoById")
def find_todo_by_id(payload: Dict[str, Any]) -> Dict[str, Any]:
    todo_id = payload.get("todo_id")
    if not todo_id or todo_id not in TODOS_DB:
        raise ValueError(f"Todo with id '{todo_id}' not found.")
    return {"todo_item": TODOS_DB[todo_id]}


@engine.activity("UpdateTodo")
def update_todo(payload: Dict[str, Any]) -> Dict[str, Any]:
    todo_id = payload.get("todo_id")
    if not todo_id or todo_id not in TODOS_DB:
        raise ValueError(f"Todo with id '{todo_id}' not found.")
    
    item = TODOS_DB[todo_id]
    if "title" in payload and payload["title"] is not None:
        new_title = str(payload["title"]).strip()
        if new_title:
            item["title"] = new_title
    if "completed" in payload and payload["completed"] is not None:
        item["completed"] = bool(payload["completed"])
        
    TODOS_DB[todo_id] = item
    return {"todo_item": item}


@engine.activity("DeleteTodo")
def delete_todo(payload: Dict[str, Any]) -> Dict[str, Any]:
    todo_id = payload.get("todo_id")
    if not todo_id or todo_id not in TODOS_DB:
        raise ValueError(f"Todo with id '{todo_id}' not found.")
    
    del TODOS_DB[todo_id]
    return {"deleted_id": todo_id}


# ==========================================
# 4. Designated Response Packaging Activities (is_output: true)
# ==========================================

@engine.activity("PackageTodoResponse")
def package_todo_response(payload: Dict[str, Any]) -> Dict[str, Any]:
    item = payload.get("todo_item", {})
    return {
        "id": item.get("id", ""),
        "title": item.get("title", ""),
        "description": item.get("description", ""),
        "completed": bool(item.get("completed", False)),
        "created_at": item.get("created_at", "")
    }


@engine.activity("PackageTodoListResponse")
def package_todo_list_response(payload: Dict[str, Any]) -> Dict[str, Any]:
    items = payload.get("items", [])
    return {
        "todos": items,
        "total_count": payload.get("count", len(items))
    }


@engine.activity("PackageDeleteResponse")
def package_delete_response(payload: Dict[str, Any]) -> Dict[str, Any]:
    return {
        "id": payload.get("deleted_id", ""),
        "deleted": True,
        "message": "Todo deleted successfully"
    }


# ==========================================
# 5. Load Blueprint and Build FastAPI App
# ==========================================

yaml_path = os.path.join(os.path.dirname(__file__), "todo_workflow.yaml")
engine.load_blueprint_file(yaml_path)

app = engine.get_fastapi_app(title="OCTOPUS Todo REST API", version="1.0.0")


if __name__ == "__main__":
    import uvicorn
    import argparse
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=int(os.getenv("PORT", "8000")))
    args = parser.parse_args()
    port = args.port
    print("\n" + "="*70)
    print(f"🚀 OCTOPUS Live Engine - Todo REST API on port {port}")
    print(f"📖 Swagger Docs:  http://localhost:{port}/docs")
    print(f"🎨 Live Visualizer: http://localhost:{port}/visualizer?domain=todo_api_service")
    print("="*70 + "\n")
    uvicorn.run(app, host="0.0.0.0", port=port)
