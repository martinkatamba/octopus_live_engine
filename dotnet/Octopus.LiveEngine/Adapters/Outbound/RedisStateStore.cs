using System.Collections.Concurrent;
using System.Text.Json;
using System.Threading.Channels;
using Octopus.LiveEngine.Domain.Models;
using Octopus.LiveEngine.Domain.Ports;
using StackExchange.Redis;

namespace Octopus.LiveEngine.Adapters.Outbound;

public class RedisStateStore : IStateStore
{
    private readonly IConnectionMultiplexer? _redis;
    private readonly IDatabase? _db;
    private readonly string _prefix;
    private readonly TimeSpan _ttlSuccess;
    private readonly TimeSpan _ttlFailure;
    private readonly TimeSpan _ttlRunning;

    private readonly ConcurrentDictionary<string, ExecutionState> _fallbackStates = new();
    private readonly ConcurrentDictionary<string, List<Channel<LiveEvent>>> _subscribers = new();
    private readonly object _subLock = new();

    private static readonly JsonSerializerOptions JsonOpts = new()
    {
        PropertyNameCaseInsensitive = true,
        PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower
    };

    public RedisStateStore(
        string connectionString = "localhost:6379",
        TimeSpan? ttlSuccess = null,
        TimeSpan? ttlFailure = null,
        TimeSpan? ttlRunning = null,
        string prefix = "octopus:live")
    {
        _prefix = string.IsNullOrWhiteSpace(prefix) ? "octopus:live" : prefix;
        _ttlSuccess = ttlSuccess ?? TimeSpan.FromHours(1);
        _ttlFailure = ttlFailure ?? TimeSpan.FromDays(7);
        _ttlRunning = ttlRunning ?? TimeSpan.FromHours(2);

        try
        {
            var connStr = connectionString.Contains("localhost") ? connectionString.Replace("localhost", "127.0.0.1") : connectionString;
            var options = ConfigurationOptions.Parse(connStr);
            options.AbortOnConnectFail = false;
            options.ConnectTimeout = 3000;
            _redis = ConnectionMultiplexer.Connect(options);
            _db = _redis.GetDatabase();
        }
        catch (Exception ex)
        {
            Console.WriteLine($"⚠️ [REDIS STATE STORE] Failed to connect: {ex.Message}. Falling back to in-memory store.");
            _redis = null;
            _db = null;
        }
    }

    private string StateKey(string jobId) => $"{_prefix}:state:{jobId}";
    private string TraceKey(string jobId) => $"{_prefix}:traces:{jobId}";
    private string EventChannel(string jobId) => $"{_prefix}:events:{jobId}";

    private TimeSpan ComputeTtl(ExecutionStatus status)
    {
        var st = status.ToString().Trim().ToUpperInvariant();
        if (st == "FAILED" || st == "ERROR" || st == "TIMEOUT" || st == "ROLLED_BACK")
        {
            return _ttlFailure;
        }
        if (st == "COMPLETED" || st == "SUCCESS")
        {
            return _ttlSuccess;
        }
        return _ttlRunning;
    }

    public async Task SaveStateAsync(ExecutionState state, CancellationToken cancellationToken = default)
    {
        if (_db == null || _redis == null || !_redis.IsConnected)
        {
            _fallbackStates[state.JobId] = state;
            return;
        }

        try
        {
            var ttl = ComputeTtl(state.Status);
            var stateJson = JsonSerializer.Serialize(state, JsonOpts);

            var batch = _db.CreateBatch();
            var t1 = batch.StringSetAsync(StateKey(state.JobId), stateJson, ttl);

            Task? t2 = null;
            if (state.Traces != null && state.Traces.Count > 0)
            {
                var tracesJson = JsonSerializer.Serialize(state.Traces, JsonOpts);
                t2 = batch.StringSetAsync(TraceKey(state.JobId), tracesJson, ttl);
            }

            batch.Execute();
            await t1;
            if (t2 != null) await t2;
        }
        catch (Exception ex)
        {
            Console.WriteLine($"⚠️ [REDIS SAVE STATE ERROR]: {ex.Message}");
            _fallbackStates[state.JobId] = state;
        }
    }

    public async Task<ExecutionState?> GetStateAsync(string jobId, CancellationToken cancellationToken = default)
    {
        if (_db != null && _redis != null && _redis.IsConnected)
        {
            try
            {
                var val = await _db.StringGetAsync(StateKey(jobId));
                if (val.HasValue && !string.IsNullOrEmpty(val))
                {
                    var state = JsonSerializer.Deserialize<ExecutionState>((string)val!, JsonOpts);
                    if (state != null)
                    {
                        var tracesVal = await _db.StringGetAsync(TraceKey(jobId));
                        if (tracesVal.HasValue && !string.IsNullOrEmpty(tracesVal))
                        {
                            var traces = JsonSerializer.Deserialize<List<StepExecutionTrace>>((string)tracesVal!, JsonOpts);
                            if (traces != null)
                            {
                                state.Traces = traces;
                            }
                        }
                        return state;
                    }
                }
                else
                {
                    return null;
                }
            }
            catch (Exception ex)
            {
                Console.WriteLine($"⚠️ [REDIS GET STATE ERROR]: {ex.Message}");
            }
        }

        _fallbackStates.TryGetValue(jobId, out var fallback);
        return fallback;
    }

    public async Task<IReadOnlyList<ExecutionState>> ListStatesAsync(string? workflowName = null, int limit = 100, CancellationToken cancellationToken = default)
    {
        if (_redis != null && _db != null && _redis.IsConnected)
        {
            try
            {
                var server = _redis.GetServer(_redis.GetEndPoints().First());
                var pattern = $"{_prefix}:state:*";
                var keys = server.Keys(pattern: pattern).Take(limit * 3).ToArray();

                var list = new List<ExecutionState>();
                foreach (var k in keys)
                {
                    try
                    {
                        var val = await _db.StringGetAsync(k);
                        if (val.HasValue && !string.IsNullOrEmpty(val))
                        {
                            var state = JsonSerializer.Deserialize<ExecutionState>((string)val!, JsonOpts);
                            if (state != null && !string.IsNullOrEmpty(state.JobId))
                            {
                                if (string.IsNullOrWhiteSpace(workflowName) || string.Equals(state.WorkflowName, workflowName, StringComparison.OrdinalIgnoreCase))
                                {
                                    list.Add(state);
                                }
                            }
                        }
                    }
                    catch
                    {
                        // Skip any incompatible key format
                    }
                }

                return list.OrderByDescending(s => s.StartedAt).Take(limit).ToList();
            }
            catch (Exception ex)
            {
                Console.WriteLine($"⚠️ [REDIS LIST STATES ERROR]: {ex.Message}");
            }
        }

        var query = _fallbackStates.Values.AsEnumerable();
        if (!string.IsNullOrWhiteSpace(workflowName))
        {
            query = query.Where(s => string.Equals(s.WorkflowName, workflowName, StringComparison.OrdinalIgnoreCase));
        }
        return query.OrderByDescending(s => s.StartedAt).Take(limit).ToList();
    }

    public async Task PublishEventAsync(string jobId, LiveEvent liveEvent, CancellationToken cancellationToken = default)
    {
        lock (_subLock)
        {
            if (_subscribers.TryGetValue(jobId, out var channels))
            {
                foreach (var ch in channels)
                {
                    ch.Writer.TryWrite(liveEvent);
                }
            }
        }

        if (_db != null && _redis != null && _redis.IsConnected)
        {
            try
            {
                var sub = _redis.GetSubscriber();
                var eventJson = JsonSerializer.Serialize(liveEvent);
                await sub.PublishAsync(RedisChannel.Literal(EventChannel(jobId)), eventJson);
            }
            catch
            {
                // ignore
            }
        }
    }

    public async IAsyncEnumerable<LiveEvent> SubscribeEventsAsync(string jobId, [System.Runtime.CompilerServices.EnumeratorCancellation] CancellationToken cancellationToken = default)
    {
        var ch = Channel.CreateUnbounded<LiveEvent>();
        lock (_subLock)
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
            lock (_subLock)
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
