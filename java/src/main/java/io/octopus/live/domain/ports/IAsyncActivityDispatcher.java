package io.octopus.live.domain.ports;

import java.util.Map;

/**
 * Port interface for dispatching asynchronous activities, middleware exception handlers,
 * and asynchronous saga compensations.
 * 
 * Implementations can execute in-memory or route through Temporal.io for cluster-wide durable execution.
 */
public interface IAsyncActivityDispatcher {

    /**
     * Dispatches an asynchronous activity for durable or background execution.
     *
     * @param activityName The registered activity name
     * @param payload      The input payload parameters
     * @param jobId        Unique workflow job ID
     * @param stepId       Step or trigger identifier (e.g. "step_1" or "middleware_exception_401")
     * @param localExecutor The local activity executor (used for fallback or worker execution)
     */
    void dispatch(
            String activityName,
            Map<String, Object> payload,
            String jobId,
            String stepId,
            IActivityExecutor localExecutor
    );

    /**
     * Name/type of dispatcher (e.g. "TEMPORAL" or "IN_MEMORY").
     */
    String getDispatcherType();
}
