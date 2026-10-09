package io.octopus.examples.todo;

import io.octopus.live.LiveEngine;
import java.time.Instant;
import java.util.*;
import java.util.concurrent.ConcurrentHashMap;

public class TodoApplication {

    // ==========================================
    // 1. Domain Models
    // ==========================================

    public static class CreateTodoRequest {
        public String title;
        public String description;
    }

    public static class UpdateTodoRequest {
        public String title;
        public Boolean completed;
    }

    public static class TodoResponse {
        public String id;
        public String title;
        public String description;
        public boolean completed;
        public String created_at;
    }

    public static class TodoListResponse {
        public List<Map<String, Object>> todos;
        public int total_count;
    }

    public static class DeleteTodoResponse {
        public String id;
        public boolean deleted;
        public String message;
    }

    // ==========================================
    // 2. In-Memory Database
    // ==========================================

    private static final Map<String, Map<String, Object>> TODOS_DB = new ConcurrentHashMap<>();

    public static void main(String[] args) throws Exception {
        LiveEngine engine = new LiveEngine();

        // 1. Register Models
        engine.registerModel("CreateTodoRequest", CreateTodoRequest.class);
        engine.registerModel("UpdateTodoRequest", UpdateTodoRequest.class);
        engine.registerModel("TodoResponse", TodoResponse.class);
        engine.registerModel("TodoListResponse", TodoListResponse.class);
        engine.registerModel("DeleteTodoResponse", DeleteTodoResponse.class);

        // 2. Register Activities
        engine.registerActivity("ValidateTodoInput", input -> {
            String title = input.get("title") != null ? input.get("title").toString().trim() : "";
            if (title.isEmpty()) {
                throw new IllegalArgumentException("Todo 'title' cannot be empty.");
            }
            Map<String, Object> out = new HashMap<>();
            out.put("validated_title", title);
            out.put("validated_desc", input.getOrDefault("description", ""));
            return out;
        });

        engine.registerActivity("PersistTodo", input -> {
            String todoId = "todo_" + UUID.randomUUID().toString().replace("-", "").substring(0, 8);
            String createdAt = Instant.now().toString();

            Map<String, Object> item = new HashMap<>();
            item.put("id", todoId);
            item.put("title", input.get("validated_title"));
            item.put("description", input.getOrDefault("validated_desc", ""));
            item.put("completed", false);
            item.put("created_at", createdAt);

            TODOS_DB.put(todoId, item);
            return Map.of("todo_item", item);
        });

        engine.registerActivity("FetchTodos", input -> {
            List<Map<String, Object>> items = new ArrayList<>(TODOS_DB.values());
            Map<String, Object> out = new HashMap<>();
            out.put("items", items);
            out.put("count", items.size());
            return out;
        });

        engine.registerActivity("FindTodoById", input -> {
            String todoId = input.get("todo_id") != null ? input.get("todo_id").toString() : "";
            Map<String, Object> item = TODOS_DB.get(todoId);
            if (item == null) {
                throw new NoSuchElementException("Todo with id '" + todoId + "' not found.");
            }
            return Map.of("todo_item", item);
        });

        engine.registerActivity("UpdateTodo", input -> {
            String todoId = input.get("todo_id") != null ? input.get("todo_id").toString() : "";
            Map<String, Object> item = TODOS_DB.get(todoId);
            if (item == null) {
                throw new NoSuchElementException("Todo with id '" + todoId + "' not found.");
            }

            if (input.get("title") != null && !input.get("title").toString().isBlank()) {
                item.put("title", input.get("title").toString().trim());
            }
            if (input.get("completed") != null) {
                item.put("completed", Boolean.valueOf(input.get("completed").toString()));
            }
            TODOS_DB.put(todoId, item);
            return Map.of("todo_item", item);
        });

        engine.registerActivity("DeleteTodo", input -> {
            String todoId = input.get("todo_id") != null ? input.get("todo_id").toString() : "";
            if (!TODOS_DB.containsKey(todoId)) {
                throw new NoSuchElementException("Todo with id '" + todoId + "' not found.");
            }
            TODOS_DB.remove(todoId);
            return Map.of("deleted_id", todoId);
        });

        // Response Packaging Activities (is_output: true)
        engine.registerActivity("PackageTodoResponse", input -> {
            @SuppressWarnings("unchecked")
            Map<String, Object> item = (Map<String, Object>) input.getOrDefault("todo_item", input);
            Map<String, Object> out = new HashMap<>();
            out.put("id", item.getOrDefault("id", ""));
            out.put("title", item.getOrDefault("title", ""));
            out.put("description", item.getOrDefault("description", ""));
            out.put("completed", item.getOrDefault("completed", false));
            out.put("created_at", item.getOrDefault("created_at", ""));
            return out;
        });

        engine.registerActivity("PackageTodoListResponse", input -> {
            @SuppressWarnings("unchecked")
            List<Map<String, Object>> items = (List<Map<String, Object>>) input.getOrDefault("items", List.of());
            int count = (int) input.getOrDefault("count", items.size());
            Map<String, Object> out = new HashMap<>();
            out.put("todos", items);
            out.put("total_count", count);
            return out;
        });

        engine.registerActivity("PackageDeleteResponse", input -> {
            Map<String, Object> out = new HashMap<>();
            out.put("id", input.getOrDefault("deleted_id", ""));
            out.put("deleted", true);
            out.put("message", "Todo deleted successfully");
            return out;
        });

        // 3. Load Blueprint & Start HTTP Server
        engine.loadBlueprintsFromDirectory(".");
        int port = System.getenv("PORT") != null ? Integer.parseInt(System.getenv("PORT")) : 8000;
        engine.startServer(port);

        System.out.println("\n==================================================================");
        System.out.println("🚀 OCTOPUS Live Engine - Java 21+ Todo REST API");
        System.out.println("📖 Swagger Docs:  http://localhost:" + port + "/docs");
        System.out.println("🎨 Live Visualizer: http://localhost:" + port + "/visualizer?domain=todo_api_service");
        System.out.println("==================================================================\n");
    }
}
