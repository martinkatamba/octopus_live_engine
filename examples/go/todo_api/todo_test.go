package main

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/octopus-workflow/octopus_live_engine/go/pkg/octopus_live"
)

func setupTodoEngine() *octopus_live.OctopusLiveEngine {
	gin.SetMode(gin.TestMode)
	engine := octopus_live.NewLiveEngine()

	// 1. Register Models
	engine.RegisterModel("CreateTodoRequest", CreateTodoRequest{})
	engine.RegisterModel("UpdateTodoRequest", UpdateTodoRequest{})
	engine.RegisterModel("TodoResponse", TodoResponse{})
	engine.RegisterModel("TodoListResponse", TodoListResponse{})
	engine.RegisterModel("DeleteTodoResponse", DeleteTodoResponse{})

	// 2. Register Activities
	engine.RegisterActivity("ValidateTodoInput", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		title, _ := input["title"].(string)
		title = strings.TrimSpace(title)
		if title == "" {
			return nil, fmt.Errorf("todo 'title' cannot be empty")
		}
		desc, _ := input["description"].(string)
		return map[string]interface{}{
			"validated_title": title,
			"validated_desc":  desc,
		}, nil
	})

	engine.RegisterActivity("PersistTodo", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		todosMu.Lock()
		defer todosMu.Unlock()

		todoID := fmt.Sprintf("todo_%d", time.Now().UnixNano()%100000000)
		createdAt := time.Now().UTC().Format(time.RFC3339)
		item := map[string]interface{}{
			"id":          todoID,
			"title":       input["validated_title"],
			"description": input["validated_desc"],
			"completed":   false,
			"created_at":  createdAt,
		}
		todosDB[todoID] = item
		return map[string]interface{}{"todo_item": item}, nil
	})

	engine.RegisterActivity("FetchTodos", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		todosMu.RLock()
		defer todosMu.RUnlock()

		items := make([]map[string]interface{}, 0, len(todosDB))
		for _, item := range todosDB {
			items = append(items, item)
		}
		return map[string]interface{}{
			"items": items,
			"count": len(items),
		}, nil
	})

	engine.RegisterActivity("FindTodoById", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		todosMu.RLock()
		defer todosMu.RUnlock()

		id, _ := input["todo_id"].(string)
		item, exists := todosDB[id]
		if !exists {
			return nil, fmt.Errorf("todo with id '%s' not found", id)
		}
		return map[string]interface{}{"todo_item": item}, nil
	})

	engine.RegisterActivity("UpdateTodo", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		todosMu.Lock()
		defer todosMu.Unlock()

		id, _ := input["todo_id"].(string)
		item, exists := todosDB[id]
		if !exists {
			return nil, fmt.Errorf("todo with id '%s' not found", id)
		}

		if t, ok := input["title"].(string); ok && strings.TrimSpace(t) != "" {
			item["title"] = strings.TrimSpace(t)
		}
		if c, ok := input["completed"].(bool); ok {
			item["completed"] = c
		}
		todosDB[id] = item
		return map[string]interface{}{"todo_item": item}, nil
	})

	engine.RegisterActivity("DeleteTodo", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		todosMu.Lock()
		defer todosMu.Unlock()

		id, _ := input["todo_id"].(string)
		if _, exists := todosDB[id]; !exists {
			return nil, fmt.Errorf("todo with id '%s' not found", id)
		}
		delete(todosDB, id)
		return map[string]interface{}{"deleted_id": id}, nil
	})

	engine.RegisterActivity("PackageTodoResponse", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		item, _ := input["todo_item"].(map[string]interface{})
		if item == nil {
			item = input
		}
		return map[string]interface{}{
			"id":          item["id"],
			"title":       item["title"],
			"description": item["description"],
			"completed":   item["completed"],
			"created_at":  item["created_at"],
		}, nil
	})

	engine.RegisterActivity("PackageTodoListResponse", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		items, _ := input["items"].([]map[string]interface{})
		count, _ := input["count"].(int)
		return map[string]interface{}{
			"todos":       items,
			"total_count": count,
		}, nil
	})

	engine.RegisterActivity("PackageDeleteResponse", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		return map[string]interface{}{
			"id":      input["deleted_id"],
			"deleted": true,
			"message": "Todo deleted successfully",
		}, nil
	})

	engine.LoadBlueprintFile("todo_workflow.yaml")
	return engine
}

func TestGoTodoLifecycle(t *testing.T) {
	todosMu.Lock()
	todosDB = make(map[string]map[string]interface{})
	todosMu.Unlock()

	engine := setupTodoEngine()
	router := engine.GetGinEngine()

	// 1. Create Todo (POST /api/v1/todos)
	createBody, _ := json.Marshal(map[string]interface{}{
		"title":       "Write Go Tests",
		"description": "Complete unit testing suite for Go examples",
	})
	createReq, _ := http.NewRequest("POST", "/api/v1/todos", bytes.NewBuffer(createBody))
	createReq.Header.Set("Content-Type", "application/json")
	createReq.Header.Set("X-Job-ID", "tx-go-create-001")

	wCreate := httptest.NewRecorder()
	router.ServeHTTP(wCreate, createReq)

	if wCreate.Code != 201 {
		t.Fatalf("Expected 201 Created, got %d: %s", wCreate.Code, wCreate.Body.String())
	}

	var created TodoResponse
	_ = json.Unmarshal(wCreate.Body.Bytes(), &created)
	if created.Title != "Write Go Tests" {
		t.Errorf("Expected title 'Write Go Tests', got %s", created.Title)
	}
	if created.Completed {
		t.Errorf("Expected completed=false")
	}

	// 2. List Todos (GET /api/v1/todos)
	listReq, _ := http.NewRequest("GET", "/api/v1/todos", nil)
	wList := httptest.NewRecorder()
	router.ServeHTTP(wList, listReq)

	if wList.Code != 200 {
		t.Fatalf("Expected 200 OK, got %d: %s", wList.Code, wList.Body.String())
	}

	var listResp TodoListResponse
	_ = json.Unmarshal(wList.Body.Bytes(), &listResp)
	if listResp.TotalCount != 1 {
		t.Errorf("Expected total_count=1, got %d", listResp.TotalCount)
	}

	// 3. Delete Todo (DELETE /api/v1/todos/{todo_id})
	delReq, _ := http.NewRequest("DELETE", fmt.Sprintf("/api/v1/todos/%s", created.ID), nil)
	wDel := httptest.NewRecorder()
	router.ServeHTTP(wDel, delReq)

	if wDel.Code != 200 {
		t.Fatalf("Expected 200 OK, got %d: %s", wDel.Code, wDel.Body.String())
	}

	var delResp DeleteTodoResponse
	_ = json.Unmarshal(wDel.Body.Bytes(), &delResp)
	if !delResp.Deleted {
		t.Errorf("Expected deleted=true")
	}
}
