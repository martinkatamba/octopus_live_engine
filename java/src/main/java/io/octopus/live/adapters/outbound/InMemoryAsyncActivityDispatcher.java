package io.octopus.live.adapters.outbound;

import io.octopus.live.domain.ports.IActivityExecutor;
import io.octopus.live.domain.ports.IAsyncActivityDispatcher;

import java.util.HashMap;
import java.util.Map;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/**
 * In-memory implementation of IAsyncActivityDispatcher using Java's ExecutorService / CompletableFuture.
 * Ideal for offline development, local unit tests, or zero-dependency setups.
 */
public class InMemoryAsyncActivityDispatcher implements IAsyncActivityDispatcher {

    private final ExecutorService executor;

    public InMemoryAsyncActivityDispatcher() {
        this.executor = Executors.newCachedThreadPool(r -> {
            Thread t = new Thread(r, "Octopus-Async-Worker");
            t.setDaemon(true);
            return t;
        });
    }

    public InMemoryAsyncActivityDispatcher(ExecutorService executor) {
        this.executor = executor != null ? executor : Executors.newCachedThreadPool();
    }

    @Override
    public void dispatch(
            String activityName,
            Map<String, Object> payload,
            String jobId,
            String stepId,
            IActivityExecutor localExecutor
    ) {
        if (activityName == null || activityName.isBlank() || localExecutor == null) {
            return;
        }

        Map<String, Object> payloadCopy = payload != null ? new HashMap<>(payload) : new HashMap<>();
        CompletableFuture.runAsync(() -> {
            try {
                localExecutor.executeActivity(activityName, payloadCopy);
            } catch (Exception ex) {
                System.err.println("⚡ [InMemoryAsyncDispatcher] Background activity '" + activityName + "' (job=" + jobId + ", step=" + stepId + ") failed: " + ex.getMessage());
            }
        }, executor);
    }

    @Override
    public String getDispatcherType() {
        return "IN_MEMORY";
    }

    public void shutdown() {
        executor.shutdown();
    }
}
