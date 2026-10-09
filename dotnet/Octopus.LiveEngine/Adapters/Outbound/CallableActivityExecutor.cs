using System.Collections.Concurrent;
using Octopus.LiveEngine.Domain.Ports;

namespace Octopus.LiveEngine.Adapters.Outbound;

public class CallableActivityExecutor : IActivityExecutor
{
    private readonly ConcurrentDictionary<string, ActivityHandlerFunc> _handlers = new();

    public void RegisterActivity(string activityName, ActivityHandlerFunc handler)
    {
        _handlers[activityName] = handler;
    }

    public bool HasActivity(string activityName)
    {
        return _handlers.ContainsKey(activityName);
    }

    public Task<Dictionary<string, object>> ExecuteActivityAsync(
        string activityName,
        Dictionary<string, object> input,
        CancellationToken cancellationToken = default
    )
    {
        if (!_handlers.TryGetValue(activityName, out var handler))
        {
            throw new InvalidOperationException($"Activity handler '{activityName}' is not registered in Live Engine.");
        }
        return handler(input, cancellationToken);
    }
}
