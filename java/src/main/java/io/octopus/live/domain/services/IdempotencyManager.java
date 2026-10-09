package io.octopus.live.domain.services;

import io.octopus.live.domain.models.ExecutionState;
import java.time.Duration;
import java.time.Instant;
import java.util.concurrent.ConcurrentHashMap;

public class IdempotencyManager {

    private static class CacheEntry {
        final ExecutionState state;
        final Instant expiresAt;

        CacheEntry(ExecutionState state, Instant expiresAt) {
            this.state = state;
            this.expiresAt = expiresAt;
        }
    }

    private final ConcurrentHashMap<String, CacheEntry> cache = new ConcurrentHashMap<>();
    private final int maxSize;
    private final Duration ttl;

    public IdempotencyManager(int maxSize, Duration ttl) {
        this.maxSize = maxSize;
        this.ttl = ttl;
    }

    public IdempotencyManager() {
        this(10000, Duration.ofHours(1));
    }

    public ExecutionState get(String key) {
        CacheEntry entry = cache.get(key);
        if (entry == null) return null;
        if (Instant.now().isAfter(entry.expiresAt)) {
            cache.remove(key);
            return null;
        }
        return entry.state;
    }

    public void set(String key, ExecutionState state) {
        if (cache.size() >= maxSize) {
            Instant now = Instant.now();
            cache.entrySet().removeIf(e -> now.isAfter(e.getValue().expiresAt));
        }
        cache.put(key, new CacheEntry(state, Instant.now().plus(ttl)));
    }
}
