package io.octopus.live.adapters.outbound;

import io.octopus.live.domain.models.ExecutionState;
import io.octopus.live.domain.models.LiveEvent;
import io.octopus.live.domain.ports.IStateStore;
import java.util.List;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.CopyOnWriteArrayList;
import java.util.function.Consumer;

public class InMemoryStateStore implements IStateStore {

    private final ConcurrentHashMap<String, ExecutionState> states = new ConcurrentHashMap<>();
    private final ConcurrentHashMap<String, List<Consumer<LiveEvent>>> subscribers = new ConcurrentHashMap<>();

    @Override
    public void saveState(ExecutionState state) {
        states.put(state.getJobId(), state);
    }

    @Override
    public ExecutionState getState(String jobId) {
        return states.get(jobId);
    }

    @Override
    public List<ExecutionState> listStates(String workflowName, int limit) {
        List<ExecutionState> all = new java.util.ArrayList<>(states.values());
        all.sort((a, b) -> {
            String ta = a.getStartedAt() != null ? a.getStartedAt() : "";
            String tb = b.getStartedAt() != null ? b.getStartedAt() : "";
            return tb.compareTo(ta);
        });
        if (workflowName != null && !workflowName.isBlank() && !workflowName.equalsIgnoreCase("ALL")) {
            all = all.stream()
                .filter(s -> workflowName.equalsIgnoreCase(s.getWorkflowName()))
                .collect(java.util.stream.Collectors.toList());
        }
        if (limit > 0 && all.size() > limit) {
            all = all.subList(0, limit);
        }
        return all;
    }

    @Override
    public void publishEvent(String jobId, LiveEvent event) {
        List<Consumer<LiveEvent>> subs = subscribers.get(jobId);
        if (subs != null) {
            for (Consumer<LiveEvent> sub : subs) {
                try {
                    sub.accept(event);
                } catch (Exception ignored) {}
            }
        }
    }

    @Override
    public AutoCloseable subscribeEvents(String jobId, Consumer<LiveEvent> consumer) {
        subscribers.computeIfAbsent(jobId, k -> new CopyOnWriteArrayList<>()).add(consumer);
        return () -> {
            List<Consumer<LiveEvent>> subs = subscribers.get(jobId);
            if (subs != null) {
                subs.remove(consumer);
                if (subs.isEmpty()) {
                    subscribers.remove(jobId);
                }
            }
        };
    }
}
