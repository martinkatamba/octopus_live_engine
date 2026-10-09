using System.Collections.Concurrent;
using System.Threading.Channels;
using Octopus.LiveEngine.Domain.Models;
using Octopus.LiveEngine.Domain.Ports;

namespace Octopus.LiveEngine.Adapters.Outbound;

public class InMemoryStateStore : IStateStore
{
    private readonly ConcurrentDictionary<string, ExecutionState> _states = new();
    private readonly ConcurrentDictionary<string, List<Channel<LiveEvent>>> _subscribers = new();
    private readonly object _lock = new();

    public Task SaveStateAsync(ExecutionState state, CancellationToken cancellationToken = default)
    {
        _states[state.JobId] = state;
        return Task.CompletedTask;
    }

    public Task<ExecutionState?> GetStateAsync(string jobId, CancellationToken cancellationToken = default)
    {
        _states.TryGetValue(jobId, out var state);
        return Task.FromResult(state);
    }

    public Task<IReadOnlyList<ExecutionState>> ListStatesAsync(string? workflowName = null, int limit = 100, CancellationToken cancellationToken = default)
    {
        var query = _states.Values.AsEnumerable();
        if (!string.IsNullOrWhiteSpace(workflowName))
        {
            query = query.Where(s => string.Equals(s.WorkflowName, workflowName, StringComparison.OrdinalIgnoreCase));
        }
        var list = query
            .OrderByDescending(s => s.StartedAt)
            .Take(limit)
            .ToList();
        return Task.FromResult<IReadOnlyList<ExecutionState>>(list);
    }

    public Task PublishEventAsync(string jobId, LiveEvent liveEvent, CancellationToken cancellationToken = default)
    {
        lock (_lock)
        {
            if (_subscribers.TryGetValue(jobId, out var channels))
            {
                foreach (var ch in channels)
                {
                    ch.Writer.TryWrite(liveEvent);
                }
            }
        }
        return Task.CompletedTask;
    }

    public async IAsyncEnumerable<LiveEvent> SubscribeEventsAsync(string jobId, [System.Runtime.CompilerServices.EnumeratorCancellation] CancellationToken cancellationToken = default)
    {
        var ch = Channel.CreateUnbounded<LiveEvent>();
        lock (_lock)
        {
            var list = _subscribers.GetOrAdd(jobId, _ => new List<Channel<LiveEvent>>());
            list.Add(ch);
        }

        try
        {
            while (await ch.Reader.WaitToReadAsync(cancellationToken))
            {
                while (ch.Reader.TryRead(out var ev))
                {
                    yield return ev;
                }
            }
        }
        finally
        {
            lock (_lock)
            {
                if (_subscribers.TryGetValue(jobId, out var list))
                {
                    list.Remove(ch);
                    if (list.Count == 0) _subscribers.TryRemove(jobId, out _);
                }
            }
        }
    }
}
