using System.Collections.Concurrent;
using Octopus.LiveEngine;
using Octopus.LiveEngine.Adapters.Inbound;

var builder = WebApplication.CreateBuilder(args);
var app = builder.Build();

var engine = new OctopusLiveEngine();

// ==========================================
// In-Memory Store
// ==========================================
var todosDB = new ConcurrentDictionary<string, Dictionary<string, object>>();

// 1. Register Models
engine.RegisterModel<CreateTodoRequest>("CreateTodoRequest");
engine.RegisterModel<UpdateTodoRequest>("UpdateTodoRequest");
engine.RegisterModel<TodoResponse>("TodoResponse");
engine.RegisterModel<TodoListResponse>("TodoListResponse");
engine.RegisterModel<DeleteTodoResponse>("DeleteTodoResponse");

// 2. Register Activities
engine.RegisterActivity("ValidateTodoInput", (input, ct) => {
    var title = input.GetValueOrDefault("title")?.ToString()?.Trim();
    if (string.IsNullOrEmpty(title))
    {
        throw new ArgumentException("Todo 'title' cannot be empty.");
    }
    return Task.FromResult(new Dictionary<string, object> {
        ["validated_title"] = title,
        ["validated_desc"] = input.GetValueOrDefault("description")?.ToString() ?? ""
    });
});

engine.RegisterActivity("PersistTodo", (input, ct) => {
    var todoId = $"todo_{Guid.NewGuid():N}"[..13];
    var createdAt = DateTime.UtcNow.ToString("o");
    var item = new Dictionary<string, object> {
        ["id"] = todoId,
        ["title"] = input["validated_title"],
        ["description"] = input.GetValueOrDefault("validated_desc", ""),
        ["completed"] = false,
        ["created_at"] = createdAt
    };
    todosDB[todoId] = item;
    return Task.FromResult(new Dictionary<string, object> {
        ["todo_item"] = item
    });
});

engine.RegisterActivity("FetchTodos", (input, ct) => {
    var items = todosDB.Values.ToList();
    return Task.FromResult(new Dictionary<string, object> {
        ["items"] = items,
        ["count"] = items.Count
    });
});

engine.RegisterActivity("FindTodoById", (input, ct) => {
    var id = input.GetValueOrDefault("todo_id")?.ToString();
    if (id == null || !todosDB.TryGetValue(id, out var item))
    {
        throw new KeyNotFoundException($"Todo with id '{id}' not found.");
    }
    return Task.FromResult(new Dictionary<string, object> {
        ["todo_item"] = item
    });
});

engine.RegisterActivity("UpdateTodo", (input, ct) => {
    var id = input.GetValueOrDefault("todo_id")?.ToString();
    if (id == null || !todosDB.TryGetValue(id, out var item))
    {
        throw new KeyNotFoundException($"Todo with id '{id}' not found.");
    }
    if (input.TryGetValue("title", out var t) && t != null && !string.IsNullOrWhiteSpace(t.ToString()))
    {
        item["title"] = t.ToString()!.Trim();
    }
    if (input.TryGetValue("completed", out var c) && c != null)
    {
        item["completed"] = Convert.ToBoolean(c);
    }
    todosDB[id] = item;
    return Task.FromResult(new Dictionary<string, object> {
        ["todo_item"] = item
    });
});

engine.RegisterActivity("DeleteTodo", (input, ct) => {
    var id = input.GetValueOrDefault("todo_id")?.ToString();
    if (id == null || !todosDB.TryRemove(id, out _))
    {
        throw new KeyNotFoundException($"Todo with id '{id}' not found.");
    }
    return Task.FromResult(new Dictionary<string, object> {
        ["deleted_id"] = id
    });
});

// Response Packaging Activities (is_output: true)
engine.RegisterActivity("PackageTodoResponse", (input, ct) => {
    var item = (input.GetValueOrDefault("todo_item") as Dictionary<string, object>) ?? input;
    return Task.FromResult(new Dictionary<string, object> {
        ["id"] = item.GetValueOrDefault("id", "")?.ToString() ?? "",
        ["title"] = item.GetValueOrDefault("title", "")?.ToString() ?? "",
        ["description"] = item.GetValueOrDefault("description", "")?.ToString() ?? "",
        ["completed"] = Convert.ToBoolean(item.GetValueOrDefault("completed", false)),
        ["created_at"] = item.GetValueOrDefault("created_at", "")?.ToString() ?? ""
    });
});

engine.RegisterActivity("PackageTodoListResponse", (input, ct) => {
    var items = input.GetValueOrDefault("items") as IEnumerable<object> ?? new List<object>();
    var count = Convert.ToInt32(input.GetValueOrDefault("count", 0));
    return Task.FromResult(new Dictionary<string, object> {
        ["todos"] = items,
        ["total_count"] = count
    });
});

engine.RegisterActivity("PackageDeleteResponse", (input, ct) => {
    return Task.FromResult(new Dictionary<string, object> {
        ["id"] = input.GetValueOrDefault("deleted_id", "")?.ToString() ?? "",
        ["deleted"] = true,
        ["message"] = "Todo deleted successfully"
    });
});

// 3. Load Blueprint & Map Endpoints
engine.LoadBlueprintFile("todo_workflow.yaml");
app.MapLiveEndpoints(engine);

var port = Environment.GetEnvironmentVariable("PORT") ?? "8000";
Console.WriteLine("\n==================================================================");
Console.WriteLine($"🚀 OCTOPUS Live Engine - .NET 8 Todo REST API");
Console.WriteLine($"🎨 Live Visualizer: http://localhost:{port}/visualizer?domain=todo_api_service");
Console.WriteLine("==================================================================\n");

app.Run($"http://0.0.0.0:{port}");

// ==========================================
// Domain Models
// ==========================================
public record CreateTodoRequest(string Title, string? Description);
public record UpdateTodoRequest(string? Title, bool? Completed);
public record TodoResponse(string Id, string Title, string? Description, bool Completed, string CreatedAt);
public record TodoListResponse(IEnumerable<TodoResponse> Todos, int TotalCount);
public record DeleteTodoResponse(string Id, bool Deleted, string Message);
