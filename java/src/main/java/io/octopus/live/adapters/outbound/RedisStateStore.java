package io.octopus.live.adapters.outbound;

import com.fasterxml.jackson.databind.ObjectMapper;
import io.octopus.live.domain.models.ExecutionState;
import io.octopus.live.domain.models.ExecutionStatus;
import io.octopus.live.domain.models.LiveEvent;
import io.octopus.live.domain.models.StepExecutionTrace;
import io.octopus.live.domain.ports.IStateStore;
import redis.clients.jedis.Jedis;
import redis.clients.jedis.JedisPool;
import redis.clients.jedis.JedisPoolConfig;

import java.net.URI;
import java.util.*;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.CopyOnWriteArrayList;
import java.util.function.Consumer;

public class RedisStateStore implements IStateStore {

    private final JedisPool jedisPool;
    private final String prefix;
    private final long ttlSuccess;
    private final long ttlFailure;
    private final long ttlRunning;
    private final ObjectMapper mapper = new ObjectMapper();

    private final Map<String, ExecutionState> fallbackStates = new ConcurrentHashMap<>();
    private final Map<String, List<Consumer<LiveEvent>>> subscribers = new ConcurrentHashMap<>();

    public RedisStateStore(String redisUrl, Long ttlSuccessSeconds, Long ttlFailureSeconds, Long ttlRunningSeconds, String prefix) {
        this.prefix = (prefix != null && !prefix.isBlank()) ? prefix : "octopus:live";
        this.ttlSuccess = (ttlSuccessSeconds != null && ttlSuccessSeconds > 0) ? ttlSuccessSeconds : 3600L;
        this.ttlFailure = (ttlFailureSeconds != null && ttlFailureSeconds > 0) ? ttlFailureSeconds : 604800L;
        this.ttlRunning = (ttlRunningSeconds != null && ttlRunningSeconds > 0) ? ttlRunningSeconds : 7200L;

        JedisPool pool = null;
        try {
            String host = "127.0.0.1";
            int port = 6379;
            String url = (redisUrl != null && !redisUrl.isBlank()) ? redisUrl : "redis://127.0.0.1:6379";
            if (!url.startsWith("redis://") && !url.startsWith("rediss://")) {
                url = "redis://" + url;
            }
            try {
                URI uri = URI.create(url);
                if (uri.getHost() != null && !uri.getHost().isBlank() && !"localhost".equalsIgnoreCase(uri.getHost())) {
                    host = uri.getHost();
                }
                if (uri.getPort() > 0) {
                    port = uri.getPort();
                }
            } catch (Exception ignored) {}

            JedisPoolConfig poolConfig = new JedisPoolConfig();
            poolConfig.setMaxTotal(32);
            poolConfig.setMaxIdle(8);
            pool = new JedisPool(poolConfig, host, port);
            try (Jedis j = pool.getResource()) {
                j.ping();
            }
        } catch (Exception e) {
            System.err.println("⚠️ [REDIS STATE STORE] Failed to connect: " + e.getMessage() + ". Falling back to in-memory store.");
            pool = null;
        }
        this.jedisPool = pool;
    }

    private String stateKey(String jobId) {
        return prefix + ":state:" + jobId;
    }

    private String traceKey(String jobId) {
        return prefix + ":traces:" + jobId;
    }

    private String eventChannel(String jobId) {
        return prefix + ":events:" + jobId;
    }

    private long computeTtl(ExecutionStatus status) {
        if (status == null) return ttlRunning;
        String st = status.name().toUpperCase();
        if (st.equals("FAILED") || st.equals("ERROR") || st.equals("TIMEOUT") || st.equals("ROLLED_BACK")) {
            return ttlFailure;
        }
        if (st.equals("COMPLETED") || st.equals("SUCCESS")) {
            return ttlSuccess;
        }
        return ttlRunning;
    }

    @Override
    public void saveState(ExecutionState state) {
        if (state == null || state.getJobId() == null) return;
        if (jedisPool == null) {
            fallbackStates.put(state.getJobId(), state);
            return;
        }

        try (Jedis jedis = jedisPool.getResource()) {
            long ttl = computeTtl(state.getStatus());
            String stateJson = mapper.writeValueAsString(state);
            jedis.setex(stateKey(state.getJobId()), ttl, stateJson);

            if (state.getTraces() != null && !state.getTraces().isEmpty()) {
                String tracesJson = mapper.writeValueAsString(state.getTraces());
                jedis.setex(traceKey(state.getJobId()), ttl, tracesJson);
            }
        } catch (Exception e) {
            System.err.println("⚠️ [REDIS SAVE STATE ERROR]: " + e.getMessage());
            fallbackStates.put(state.getJobId(), state);
        }
    }

    @Override
    public ExecutionState getState(String jobId) {
        if (jobId == null) return null;

        if (jedisPool != null) {
            try (Jedis jedis = jedisPool.getResource()) {
                String stateJson = jedis.get(stateKey(jobId));
                if (stateJson != null && !stateJson.isBlank()) {
                    ExecutionState state = mapper.readValue(stateJson, ExecutionState.class);
                    String tracesJson = jedis.get(traceKey(jobId));
                    if (tracesJson != null && !tracesJson.isBlank()) {
                        StepExecutionTrace[] traces = mapper.readValue(tracesJson, StepExecutionTrace[].class);
                        state.setTraces(new ArrayList<>(Arrays.asList(traces)));
                    }
                    return state;
                } else {
                    return null;
                }
            } catch (Exception e) {
                System.err.println("⚠️ [REDIS GET STATE ERROR]: " + e.getMessage());
            }
        }

        return fallbackStates.get(jobId);
    }

    @Override
    public List<ExecutionState> listStates(String workflowName, int limit) {
        if (jedisPool != null) {
            try (Jedis jedis = jedisPool.getResource()) {
                Set<String> keys = jedis.keys(prefix + ":state:*");
                List<ExecutionState> list = new ArrayList<>();
                if (keys != null) {
                    for (String k : keys) {
                        try {
                            String json = jedis.get(k);
                            if (json != null && !json.isBlank()) {
                                ExecutionState state = mapper.readValue(json, ExecutionState.class);
                                if (workflowName == null || workflowName.isBlank() || workflowName.equalsIgnoreCase(state.getWorkflowName())) {
                                    list.add(state);
                                }
                            }
                        } catch (Exception ignored) {
                        }
                    }
                }
                list.sort((a, b) -> {
                    String sa = a.getStartedAt() != null ? a.getStartedAt() : "";
                    String sb = b.getStartedAt() != null ? b.getStartedAt() : "";
                    return sb.compareTo(sa);
                });
                if (limit > 0 && list.size() > limit) {
                    return list.subList(0, limit);
                }
                return list;
            } catch (Exception e) {
                System.err.println("⚠️ [REDIS LIST STATES ERROR]: " + e.getMessage());
            }
        }

        List<ExecutionState> list = new ArrayList<>();
        for (ExecutionState s : fallbackStates.values()) {
            if (workflowName == null || workflowName.isBlank() || workflowName.equalsIgnoreCase(s.getWorkflowName())) {
                list.add(s);
            }
        }
        list.sort((a, b) -> {
            String sa = a.getStartedAt() != null ? a.getStartedAt() : "";
            String sb = b.getStartedAt() != null ? b.getStartedAt() : "";
            return sb.compareTo(sa);
        });
        if (limit > 0 && list.size() > limit) {
            return list.subList(0, limit);
        }
        return list;
    }

    @Override
    public void publishEvent(String jobId, LiveEvent event) {
        List<Consumer<LiveEvent>> listeners = subscribers.get(jobId);
        if (listeners != null) {
            for (Consumer<LiveEvent> c : listeners) {
                try {
                    c.accept(event);
                } catch (Exception ignored) {}
            }
        }

        if (jedisPool != null) {
            try (Jedis jedis = jedisPool.getResource()) {
                String eventJson = mapper.writeValueAsString(event);
                jedis.publish(eventChannel(jobId), eventJson);
            } catch (Exception ignored) {}
        }
    }

    @Override
    public AutoCloseable subscribeEvents(String jobId, Consumer<LiveEvent> consumer) {
        subscribers.computeIfAbsent(jobId, k -> new CopyOnWriteArrayList<>()).add(consumer);
        return () -> {
            List<Consumer<LiveEvent>> list = subscribers.get(jobId);
            if (list != null) {
                list.remove(consumer);
                if (list.isEmpty()) subscribers.remove(jobId);
            }
        };
    }
}
