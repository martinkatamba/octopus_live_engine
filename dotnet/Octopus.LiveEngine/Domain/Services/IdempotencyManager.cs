using System.Collections.Concurrent;
using Octopus.LiveEngine.Domain.Models;

namespace Octopus.LiveEngine.Domain.Services;

public class IdempotencyManager
{
    private record CacheItem(ExecutionState State, DateTime ExpiresAt);

    private readonly ConcurrentDictionary<string, CacheItem> _cache = new();
    private readonly int _maxSize;
    private readonly TimeSpan _ttl;

    public IdempotencyManager(int maxSize = 10000, TimeSpan? ttl = null)
    {
        _maxSize = maxSize;
        _ttl = ttl ?? TimeSpan.FromHours(1);
    }

    public bool TryGet(string key, out ExecutionState? state)
    {
        if (_cache.TryGetValue(key, out var item))
        {
            if (DateTime.UtcNow <= item.ExpiresAt)
            {
                state = item.State;
                return true;
            }
            _cache.TryRemove(key, out _);
        }
        state = null;
        return false;
    }

    public void Set(string key, ExecutionState state)
    {
        if (_cache.Count >= _maxSize)
        {
            var now = DateTime.UtcNow;
            foreach (var kvp in _cache)
            {
                if (kvp.Value.ExpiresAt < now)
                {
                    _cache.TryRemove(kvp.Key, out _);
                }
            }
        }
        _cache[key] = new CacheItem(state, DateTime.UtcNow.Add(_ttl));
    }
}
