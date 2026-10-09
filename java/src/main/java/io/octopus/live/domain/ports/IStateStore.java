package io.octopus.live.domain.ports;

import io.octopus.live.domain.models.ExecutionState;
import io.octopus.live.domain.models.LiveEvent;
import java.util.List;
import java.util.function.Consumer;

public interface IStateStore {
    void saveState(ExecutionState state);
    ExecutionState getState(String jobId);
    List<ExecutionState> listStates(String workflowName, int limit);
    void publishEvent(String jobId, LiveEvent event);
    AutoCloseable subscribeEvents(String jobId, Consumer<LiveEvent> consumer);
}
