package main

import (
	"context"
	"fmt"
	"log"
	"os"
	"strings"
	"sync"
	"time"

	"github.com/octopus-workflow/octopus_live_engine/go/pkg/octopus_live"
)

// ==========================================
// 1. Domain Models
// ==========================================

type CreateTodoRequest struct {
	Title       string `json:"title"`
	Description string `json:"description"`
}

type UpdateTodoRequest struct {
	Title     string `json:"title"`
	Completed *bool  `json:"completed"`
}

type TodoResponse struct {
	ID          string `json:"id"`
	Title       string `json:"title"`
	Description string `json:"description"`
	Completed   bool   `json:"completed"`
	CreatedAt   string `json:"created_at"`
}

type TodoListResponse struct {
	Todos      []TodoResponse `json:"todos"`
	TotalCount int            `json:"total_count"`
}

type DeleteTodoResponse struct {
	ID      string `json:"id"`
	Deleted bool   `json:"deleted"`
	Message string `json:"message"`
}

// ==========================================
// 2. In-Memory Store
// ==========================================

var (
	todosMu sync.RWMutex
	todosDB = make(map[string]map[string]interface{})
)

func main() {
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

	// Response Packaging Activities (is_output: true)
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

	// 3. Load Blueprint & Run Gin Server
	engine.LoadBlueprintFile("todo_workflow.yaml")

	port := os.Getenv("PORT")
	if port == "" {
		port = "8000"
	}
	if !strings.HasPrefix(port, ":") {
		port = ":" + port
	}

	r := engine.GetGinEngine()
	fmt.Println("\n==================================================================")
	fmt.Printf("🚀 OCTOPUS Live Engine - Go Todo REST API on port %s\n", port)
	fmt.Printf("📖 Swagger Docs:  http://localhost%s/docs\n", port)
	fmt.Printf("🎨 Live Visualizer: http://localhost%s/visualizer?domain=todo_api_service\n", port)
	fmt.Println("==================================================================")

	if err := r.Run(port); err != nil {
		log.Fatalf("Server failed: %v", err)
	}
}
