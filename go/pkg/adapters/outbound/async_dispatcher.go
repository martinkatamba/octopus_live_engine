package outbound

import (
	"context"
	"fmt"
	"log"
	"sync"
	"time"

	"github.com/octopus-workflow/octopus_live_engine/go/pkg/domain/ports"
	"go.temporal.io/sdk/activity"
	"go.temporal.io/sdk/client"
	"go.temporal.io/sdk/temporal"
	"go.temporal.io/sdk/worker"
	"go.temporal.io/sdk/workflow"
)

// IOctopusWorkflow is the standard workflow registered for dynamic durable activity execution
func IOctopusWorkflow(ctx workflow.Context, input map[string]interface{}) (map[string]interface{}, error) {
	activityName, _ := input["activity_name"].(string)
	if activityName == "" {
		if act, ok := input["activity"].(string); ok {
			activityName = act
		}
	}
	payload, _ := input["payload"].(map[string]interface{})
	if payload == nil {
		payload = input
	}

	ao := workflow.ActivityOptions{
		StartToCloseTimeout: 10 * time.Second,
		RetryPolicy: &temporal.RetryPolicy{
			InitialInterval:    500 * time.Millisecond,
			BackoffCoefficient: 2.0,
			MaximumAttempts:    5,
		},
	}
	ctx = workflow.WithActivityOptions(ctx, ao)

	var result map[string]interface{}
	err := workflow.ExecuteActivity(ctx, activityName, payload).Get(ctx, &result)
	if err != nil {
		return nil, err
	}
	return result, nil
}

// OctopusReplicatedWorkflow alias for backward compatibility
func OctopusReplicatedWorkflow(ctx workflow.Context, input map[string]interface{}) (map[string]interface{}, error) {
	return IOctopusWorkflow(ctx, input)
}

// InMemoryAsyncActivityDispatcher runs async activities in background goroutines
type InMemoryAsyncActivityDispatcher struct{}

func NewInMemoryAsyncActivityDispatcher() *InMemoryAsyncActivityDispatcher {
	return &InMemoryAsyncActivityDispatcher{}
}

func (d *InMemoryAsyncActivityDispatcher) Dispatch(
	ctx context.Context,
	activityName string,
	payload map[string]interface{},
	jobID, stepID string,
	executor ports.IActivityExecutor,
) {
	if activityName == "" || executor == nil {
		return
	}

	payloadCopy := copyMap(payload)
	go func() {
		bgCtx := context.Background()
		if _, err := executor.ExecuteActivity(bgCtx, activityName, payloadCopy); err != nil {
			log.Printf("⚡ [InMemoryAsyncDispatcher] Background activity '%s' (job=%s, step=%s) failed: %v", activityName, jobID, stepID, err)
		}
	}()
}

func (d *InMemoryAsyncActivityDispatcher) GetDispatcherType() string {
	return "IN_MEMORY"
}

// TemporalAsyncActivityDispatcher schedules durable activity execution with retries and hosts the SDK Worker
type TemporalAsyncActivityDispatcher struct {
	Host             string
	Namespace        string
	TaskQueue        string
	MaxRetries       int
	InitialBackoffMs int
	BackoffCoeff     float64

	temporalClient client.Client
	temporalWorker worker.Worker
	workerStarted  bool
	executor       ports.IActivityExecutor

	mu           sync.Mutex
	tasksHistory []map[string]interface{}
}

func NewTemporalAsyncActivityDispatcher(host, namespace, taskQueue string) *TemporalAsyncActivityDispatcher {
	if host == "" {
		host = "localhost:7233"
	}
	if namespace == "" {
		namespace = "default"
	}
	if taskQueue == "" {
		taskQueue = "octopus-temporal-queue"
	}

	return &TemporalAsyncActivityDispatcher{
		Host:             host,
		Namespace:        namespace,
		TaskQueue:        taskQueue,
		MaxRetries:       5,
		InitialBackoffMs: 500,
		BackoffCoeff:     2.0,
		tasksHistory:     make([]map[string]interface{}, 0),
	}
}

func (d *TemporalAsyncActivityDispatcher) GetClient() (client.Client, error) {
	d.mu.Lock()
	defer d.mu.Unlock()
	if d.temporalClient != nil {
		return d.temporalClient, nil
	}
	c, err := client.Dial(client.Options{
		HostPort:  d.Host,
		Namespace: d.Namespace,
	})
	if err != nil {
		return nil, fmt.Errorf("failed to dial Temporal server at %s: %w", d.Host, err)
	}
	d.temporalClient = c
	return d.temporalClient, nil
}

func (d *TemporalAsyncActivityDispatcher) StartWorker(executor ports.IActivityExecutor) error {
	d.mu.Lock()
	if d.workerStarted {
		d.mu.Unlock()
		return nil
	}
	d.executor = executor
	d.mu.Unlock()

	c, err := d.GetClient()
	if err != nil {
		return fmt.Errorf("cannot start Temporal worker without client: %w", err)
	}

	d.mu.Lock()
	defer d.mu.Unlock()

	w := worker.New(c, d.TaskQueue, worker.Options{})

	// Register workflows
	w.RegisterWorkflowWithOptions(IOctopusWorkflow, workflow.RegisterOptions{Name: "IOctopusWorkflow"})
	w.RegisterWorkflowWithOptions(OctopusReplicatedWorkflow, workflow.RegisterOptions{Name: "OctopusReplicatedWorkflow"})

	// Register all activities defined on the executor
	if executor != nil {
		for _, actName := range executor.ListActivities() {
			name := actName
			w.RegisterActivityWithOptions(func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
				log.Printf("⚡ [TEMPORAL WORKER] Executing activity '%s' on task queue '%s'", name, d.TaskQueue)
				return executor.ExecuteActivity(ctx, name, input)
			}, activity.RegisterOptions{
				Name: name,
			})
		}
	}

	d.temporalWorker = w
	d.workerStarted = true

	go func() {
		log.Printf("👷 [TEMPORAL WORKER] Worker poller started on task queue '%s' (Host: %s, Namespace: %s)", d.TaskQueue, d.Host, d.Namespace)
		if err := w.Run(worker.InterruptCh()); err != nil {
			log.Printf("⚠️ [TEMPORAL WORKER] Worker stopped with error: %v", err)
		}
	}()

	return nil
}

func (d *TemporalAsyncActivityDispatcher) StopWorker() {
	d.mu.Lock()
	defer d.mu.Unlock()

	if d.temporalWorker != nil {
		d.temporalWorker.Stop()
		d.temporalWorker = nil
	}
	if d.temporalClient != nil {
		d.temporalClient.Close()
		d.temporalClient = nil
	}
	d.workerStarted = false
	log.Printf("🛑 [TEMPORAL WORKER] Worker stopped and client connection closed")
}

func (d *TemporalAsyncActivityDispatcher) IsWorkerStarted() bool {
	d.mu.Lock()
	defer d.mu.Unlock()
	return d.workerStarted
}

func (d *TemporalAsyncActivityDispatcher) Dispatch(
	ctx context.Context,
	activityName string,
	payload map[string]interface{},
	jobID, stepID string,
	executor ports.IActivityExecutor,
) {
	if activityName == "" {
		return
	}

	temporalWorkflowID := fmt.Sprintf("octopus_act_%s_%s_%s", jobID, stepID, activityName)
	taskRecord := map[string]interface{}{
		"temporal_workflow_id": temporalWorkflowID,
		"task_queue":           d.TaskQueue,
		"activity_name":        activityName,
		"job_id":               jobID,
		"step_id":              stepID,
		"scheduled_at":         time.Now().UTC().Format(time.RFC3339),
		"status":               "SCHEDULED",
	}

	d.mu.Lock()
	d.tasksHistory = append(d.tasksHistory, taskRecord)
	d.mu.Unlock()

	log.Printf("⏳ [TEMPORAL DISPATCH] Queuing durable activity '%s' -> WorkflowID: %s (TaskQueue: %s)", activityName, temporalWorkflowID, d.TaskQueue)

	payloadCopy := copyMap(payload)
	payloadCopy["temporal_workflow_id"] = temporalWorkflowID
	payloadCopy["temporal_task_queue"] = d.TaskQueue

	wfInput := map[string]interface{}{
		"activity_name": activityName,
		"payload":       payloadCopy,
	}

	c, err := d.GetClient()
	if err == nil && c != nil {
		workflowOptions := client.StartWorkflowOptions{
			ID:        temporalWorkflowID,
			TaskQueue: d.TaskQueue,
		}
		bgCtx := context.Background()
		_, err := c.ExecuteWorkflow(bgCtx, workflowOptions, "IOctopusWorkflow", wfInput)
		if err != nil {
			log.Printf("⚠️ [TEMPORAL DISPATCH] ExecuteWorkflow failed: %v. Falling back to in-memory execution with retry.", err)
			go d.executeWithRetry(activityName, payloadCopy, jobID, stepID, executor, taskRecord, 0, time.Duration(d.InitialBackoffMs)*time.Millisecond)
		} else {
			d.mu.Lock()
			taskRecord["status"] = "DISPATCHED_TO_TEMPORAL"
			d.mu.Unlock()
		}
	} else {
		log.Printf("⚠️ [TEMPORAL DISPATCH] Temporal client unavailable (%v). Falling back to in-memory execution with retry.", err)
		go d.executeWithRetry(activityName, payloadCopy, jobID, stepID, executor, taskRecord, 0, time.Duration(d.InitialBackoffMs)*time.Millisecond)
	}
}

func (d *TemporalAsyncActivityDispatcher) executeWithRetry(
	activityName string,
	payload map[string]interface{},
	jobID, stepID string,
	executor ports.IActivityExecutor,
	taskRecord map[string]interface{},
	attempt int,
	delay time.Duration,
) {
	time.Sleep(delay)

	bgCtx := context.Background()
	res, err := executor.ExecuteActivity(bgCtx, activityName, payload)
	if err == nil {
		d.mu.Lock()
		taskRecord["status"] = "COMPLETED"
		taskRecord["completed_at"] = time.Now().UTC().Format(time.RFC3339)
		taskRecord["attempts"] = attempt + 1
		taskRecord["result"] = res
		d.mu.Unlock()
		log.Printf("✅ [TEMPORAL WORKER FALLBACK] Activity '%s' completed durably (attempt %d)", activityName, attempt+1)
		return
	}

	nextAttempt := attempt + 1
	if nextAttempt < d.MaxRetries {
		nextDelay := time.Duration(float64(delay) * d.BackoffCoeff)
		log.Printf("⚠️ [TEMPORAL WORKER FALLBACK] Activity '%s' failed attempt %d: %v. Retrying in %v...", activityName, nextAttempt, err, nextDelay)
		go d.executeWithRetry(activityName, payload, jobID, stepID, executor, taskRecord, nextAttempt, nextDelay)
	} else {
		d.mu.Lock()
		taskRecord["status"] = "FAILED"
		taskRecord["error"] = err.Error()
		taskRecord["failed_at"] = time.Now().UTC().Format(time.RFC3339)
		d.mu.Unlock()
		log.Printf("❌ [TEMPORAL WORKER FALLBACK] Activity '%s' exhausted retries: %v", activityName, err)
	}
}

func (d *TemporalAsyncActivityDispatcher) GetDispatcherType() string {
	return "TEMPORAL"
}

func (d *TemporalAsyncActivityDispatcher) GetDispatchedTasksHistory() []map[string]interface{} {
	d.mu.Lock()
	defer d.mu.Unlock()
	res := make([]map[string]interface{}, len(d.tasksHistory))
	copy(res, d.tasksHistory)
	return res
}

func copyMap(src map[string]interface{}) map[string]interface{} {
	if src == nil {
		return make(map[string]interface{})
	}
	dst := make(map[string]interface{}, len(src))
	for k, v := range src {
		dst[k] = v
	}
	return dst
}
