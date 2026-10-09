package io.octopus.live.adapters.outbound;

import io.octopus.live.adapters.outbound.temporal.IOctopusActivity;
import io.octopus.live.adapters.outbound.temporal.IOctopusWorkflow;
import io.octopus.live.adapters.outbound.temporal.OctopusActivityImpl;
import io.octopus.live.adapters.outbound.temporal.OctopusWorkflowImpl;
import io.octopus.live.domain.ports.IActivityExecutor;
import io.octopus.live.domain.ports.IAsyncActivityDispatcher;
import io.temporal.client.WorkflowClient;
import io.temporal.client.WorkflowClientOptions;
import io.temporal.client.WorkflowOptions;
import io.temporal.serviceclient.WorkflowServiceStubs;
import io.temporal.serviceclient.WorkflowServiceStubsOptions;
import io.temporal.worker.Worker;
import io.temporal.worker.WorkerFactory;

import java.time.Instant;
import java.util.*;
import java.util.concurrent.*;
import java.util.logging.Level;
import java.util.logging.Logger;

/**
 * Temporal-backed durable async activity dispatcher & embedded worker for Java.
 * 
 * Supports both:
 * 1. Embedded Worker Mode (automatically started alongside the HTTP server)
 * 2. Standalone Worker Mode (started via CLI/external commands for dedicated worker nodes)
 */
public class TemporalAsyncActivityDispatcher implements IAsyncActivityDispatcher {

    private static final Logger logger = Logger.getLogger(TemporalAsyncActivityDispatcher.class.getName());

    private final String host;
    private final String namespace;
    private final String taskQueue;
    private final int maxRetries;
    private final long retryInitialIntervalMs;
    private final double retryBackoffCoefficient;

    private final ScheduledExecutorService fallbackPool;
    private final List<Map<String, Object>> dispatchedTasksHistory = new CopyOnWriteArrayList<>();
    private final Set<String> registeredActivities = ConcurrentHashMap.newKeySet();

    private WorkflowServiceStubs serviceStubs;
    private WorkflowClient workflowClient;
    private WorkerFactory workerFactory;
    private boolean workerStarted = false;

    public TemporalAsyncActivityDispatcher(String host, String namespace, String taskQueue) {
        this(host, namespace, taskQueue, 5, 500, 2.0);
    }

    public TemporalAsyncActivityDispatcher(
            String host,
            String namespace,
            String taskQueue,
            int maxRetries,
            long retryInitialIntervalMs,
            double retryBackoffCoefficient
    ) {
        this.host = host != null && !host.isBlank() ? host : "127.0.0.1:7233";
        this.namespace = namespace != null && !namespace.isBlank() ? namespace : "default";
        this.taskQueue = taskQueue != null && !taskQueue.isBlank() ? taskQueue : "octopus-temporal-queue";
        this.maxRetries = maxRetries;
        this.retryInitialIntervalMs = retryInitialIntervalMs;
        this.retryBackoffCoefficient = retryBackoffCoefficient;

        this.fallbackPool = Executors.newScheduledThreadPool(4, r -> {
            Thread t = new Thread(r, "Octopus-Temporal-Fallback-Worker");
            t.setDaemon(true);
            return t;
        });

        initTemporalClient();
    }

    private synchronized void initTemporalClient() {
        try {
            WorkflowServiceStubsOptions stubsOptions = WorkflowServiceStubsOptions.newBuilder()
                    .setTarget(this.host)
                    .build();
            this.serviceStubs = WorkflowServiceStubs.newServiceStubs(stubsOptions);

            WorkflowClientOptions clientOptions = WorkflowClientOptions.newBuilder()
                    .setNamespace(this.namespace)
                    .build();
            this.workflowClient = WorkflowClient.newInstance(this.serviceStubs, clientOptions);

            logger.info("🛡️ Temporal SDK Client connected -> Host: " + this.host + 
                        ", Namespace: " + this.namespace + ", TaskQueue: " + this.taskQueue);
        } catch (Exception ex) {
            logger.log(Level.WARNING, "⚠️ Could not connect Temporal SDK client to " + host + ": " + ex.getMessage(), ex);
        }
    }

    /**
     * Starts the Temporal Worker polling the task queue.
     * Can be invoked embedded by the engine or standalone in worker nodes.
     */
    public synchronized void startWorker(IActivityExecutor activityExecutor) {
        if (workerStarted) {
            return;
        }

        if (workflowClient == null) {
            initTemporalClient();
        }

        if (workflowClient != null) {
            try {
                this.workerFactory = WorkerFactory.newInstance(workflowClient);
                Worker worker = workerFactory.newWorker(taskQueue);

                // Register workflow & activity implementations
                worker.registerWorkflowImplementationTypes(OctopusWorkflowImpl.class);
                worker.registerActivitiesImplementations(new OctopusActivityImpl(activityExecutor));

                workerFactory.start();
                this.workerStarted = true;
                logger.info("👷 [TEMPORAL WORKER] Worker actively polling TaskQueue '" + taskQueue + "' on Temporal Server (" + host + ")");
            } catch (Exception ex) {
                logger.log(Level.WARNING, "⚠️ Failed to start Temporal Worker on queue " + taskQueue + ": " + ex.getMessage(), ex);
            }
        }
    }

    public boolean isWorkerStarted() {
        return workerStarted;
    }

    public void registerActivity(String activityName) {
        if (activityName != null && !activityName.isBlank()) {
            registeredActivities.add(activityName);
        }
    }

    @Override
    public void dispatch(
            String activityName,
            Map<String, Object> payload,
            String jobId,
            String stepId,
            IActivityExecutor localExecutor
    ) {
        if (activityName == null || activityName.isBlank()) {
            return;
        }

        String temporalWorkflowId = "octopus_act_" + jobId + "_" + stepId + "_" + activityName;
        String temporalRunId = "run_" + UUID.randomUUID().toString().substring(0, 8);
        String scheduledAt = Instant.now().toString();

        Map<String, Object> taskRecord = new HashMap<>();
        taskRecord.put("temporal_workflow_id", temporalWorkflowId);
        taskRecord.put("temporal_run_id", temporalRunId);
        taskRecord.put("task_queue", taskQueue);
        taskRecord.put("activity_name", activityName);
        taskRecord.put("job_id", jobId);
        taskRecord.put("step_id", stepId);
        taskRecord.put("scheduled_at", scheduledAt);
        taskRecord.put("status", "SCHEDULED");

        dispatchedTasksHistory.add(taskRecord);

        Map<String, Object> payloadCopy = payload != null ? new HashMap<>(payload) : new HashMap<>();
        payloadCopy.put("activity_name", activityName);
        payloadCopy.put("temporal_workflow_id", temporalWorkflowId);
        payloadCopy.put("temporal_task_queue", taskQueue);

        logger.info("⏳ [TEMPORAL DISPATCH] Queuing durable activity '" + activityName + "' -> WorkflowID: " + temporalWorkflowId);

        boolean dispatchedViaSdk = false;
        if (workflowClient != null) {
            try {
                WorkflowOptions options = WorkflowOptions.newBuilder()
                        .setWorkflowId(temporalWorkflowId)
                        .setTaskQueue(taskQueue)
                        .build();

                IOctopusWorkflow workflowStub = workflowClient.newWorkflowStub(IOctopusWorkflow.class, options);
                WorkflowClient.start(workflowStub::execute, payloadCopy);
                dispatchedViaSdk = true;
                taskRecord.put("status", "SUBMITTED_TO_TEMPORAL");
                logger.info("📡 [TEMPORAL DISPATCH] Dispatched workflow '" + temporalWorkflowId + "' to Temporal Server");
            } catch (Exception ex) {
                logger.warning("⚠️ [TEMPORAL DISPATCH] SDK dispatch failed for '" + temporalWorkflowId + "': " + ex.getMessage());
            }
        }

        // If worker is not started or SDK dispatch failed, fallback to local pool execution
        if (!workerStarted || !dispatchedViaSdk) {
            executeWithRetryFallback(activityName, payloadCopy, jobId, stepId, localExecutor, taskRecord, 0, retryInitialIntervalMs);
        }
    }

    private void executeWithRetryFallback(
            String activityName,
            Map<String, Object> payload,
            String jobId,
            String stepId,
            IActivityExecutor localExecutor,
            Map<String, Object> taskRecord,
            int attempt,
            long delayMs
    ) {
        fallbackPool.schedule(() -> {
            try {
                if (localExecutor != null) {
                    Map<String, Object> result = localExecutor.executeActivity(activityName, payload);
                    taskRecord.put("status", "COMPLETED");
                    taskRecord.put("completed_at", Instant.now().toString());
                    taskRecord.put("attempts", attempt + 1);
                    if (result != null) {
                        taskRecord.put("result", result);
                    }
                    logger.info("✅ [FALLBACK WORKER] Activity '" + activityName + "' completed durably (attempt " + (attempt + 1) + ")");
                }
            } catch (Exception ex) {
                int nextAttempt = attempt + 1;
                if (nextAttempt < maxRetries) {
                    long nextDelay = (long) (delayMs * retryBackoffCoefficient);
                    logger.warning("⚠️ [FALLBACK WORKER] Activity '" + activityName + "' failed on attempt " + nextAttempt + 
                                   ": " + ex.getMessage() + ". Retrying in " + nextDelay + "ms...");
                    executeWithRetryFallback(activityName, payload, jobId, stepId, localExecutor, taskRecord, nextAttempt, nextDelay);
                } else {
                    taskRecord.put("status", "FAILED");
                    taskRecord.put("error", ex.getMessage());
                    taskRecord.put("failed_at", Instant.now().toString());
                    logger.severe("❌ [FALLBACK WORKER] Activity '" + activityName + "' exhausted all " + maxRetries + " retries: " + ex.getMessage());
                }
            }
        }, delayMs, TimeUnit.MILLISECONDS);
    }

    @Override
    public String getDispatcherType() {
        return "TEMPORAL";
    }

    public String getHost() {
        return host;
    }

    public String getNamespace() {
        return namespace;
    }

    public String getTaskQueue() {
        return taskQueue;
    }

    public List<Map<String, Object>> getDispatchedTasksHistory() {
        return Collections.unmodifiableList(dispatchedTasksHistory);
    }

    public synchronized void shutdown() {
        if (workerFactory != null) {
            try {
                workerFactory.shutdown();
                workerFactory.awaitTermination(5, TimeUnit.SECONDS);
            } catch (Exception ignored) {}
        }
        if (serviceStubs != null) {
            try {
                serviceStubs.shutdown();
            } catch (Exception ignored) {}
        }
        fallbackPool.shutdown();
        workerStarted = false;
    }
}
