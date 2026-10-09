package outbound

import (
	"context"
	"fmt"
	"log"
	"strings"
	"sync"
	"time"

	"github.com/octopus-workflow/octopus_live_engine/go/pkg/domain/models"
	"github.com/octopus-workflow/octopus_live_engine/go/pkg/domain/ports"
	"go.temporal.io/sdk/activity"
	"go.temporal.io/sdk/client"
	"go.temporal.io/sdk/temporal"
	"go.temporal.io/sdk/worker"
	"go.temporal.io/sdk/workflow"
)

// Reserved payload keys carrying routing info to the worker; stripped before
// the activity runs.
const (
	keyJobID       = "_octopus_job_id"
	keyStepID      = "_octopus_step_id"
	keyMaxAttempts = "_octopus_max_attempts"
)

// defaultAsyncRetry is used when a step has no retry_policy (pre-hardening
// behaviour: up to 5 attempts with exponential backoff).
var defaultAsyncRetry = models.RetryPolicy{MaxAttempts: 5, InitialInterval: 500 * time.Millisecond, BackoffCoefficient: 2}

const defaultAsyncTimeout = 30 * time.Second

func stepOptions(step *models.WorkflowStep) (models.RetryPolicy, time.Duration) {
	rp := models.StepRetryPolicy(step, defaultAsyncRetry)
	timeout := defaultAsyncTimeout
	if step != nil && step.TimeoutSeconds > 0 {
		timeout = time.Duration(step.TimeoutSeconds) * time.Second
	}
	return rp, timeout
}

// IOctopusWorkflow runs one durable activity with the options the step
// declared (timeout_seconds, retry_policy).
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

	timeout, attempts := defaultAsyncTimeout, defaultAsyncRetry.MaxAttempts
	initial, backoff := defaultAsyncRetry.InitialInterval, defaultAsyncRetry.BackoffCoefficient
	var maxInterval time.Duration
	if opts, ok := input["options"].(map[string]interface{}); ok {
		if v := models.ToFloat(opts["timeout_seconds"]); v > 0 {
			timeout = time.Duration(v * float64(time.Second))
		}
		if v := models.ToInt(opts["max_attempts"]); v > 0 {
			attempts = v
		}
		if v := models.ToFloat(opts["initial_interval_ms"]); v > 0 {
			initial = time.Duration(v) * time.Millisecond
		}
		if v := models.ToFloat(opts["backoff_coefficient"]); v >= 1 {
			backoff = v
		}
		if v := models.ToFloat(opts["maximum_interval_ms"]); v > 0 {
			maxInterval = time.Duration(v) * time.Millisecond
		}
	}
	ctx = workflow.WithActivityOptions(ctx, workflow.ActivityOptions{
		StartToCloseTimeout: timeout,
		RetryPolicy: &temporal.RetryPolicy{
			InitialInterval:    initial,
			BackoffCoefficient: backoff,
			MaximumInterval:    maxInterval,
			MaximumAttempts:    int32(attempts),
		},
	})

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

// runWithRetry executes an activity in-process with the step's policy and
// returns the final outcome.
func runWithRetry(executor ports.IActivityExecutor, name string, payload map[string]interface{}, rp models.RetryPolicy, timeout time.Duration) (map[string]interface{}, error, int) {
	delay := rp.InitialInterval
	var res map[string]interface{}
	var err error
	attempt := 0
	for attempt < rp.MaxAttempts {
		attempt++
		ctx, cancel := context.WithTimeout(context.Background(), timeout)
		res, err = executor.ExecuteActivity(ctx, name, copyMap(payload))
		cancel()
		if err == nil || attempt >= rp.MaxAttempts {
			break
		}
		time.Sleep(delay)
		delay = time.Duration(float64(delay) * rp.BackoffCoefficient)
		if rp.MaximumInterval > 0 && delay > rp.MaximumInterval {
			delay = rp.MaximumInterval
		}
	}
	return res, err, attempt
}

// InMemoryAsyncActivityDispatcher runs async activities in background
// goroutines (not durable across restarts).
type InMemoryAsyncActivityDispatcher struct {
	mu   sync.RWMutex
	hook ports.AsyncCompletion
}

func NewInMemoryAsyncActivityDispatcher() *InMemoryAsyncActivityDispatcher {
	return &InMemoryAsyncActivityDispatcher{}
}

func (d *InMemoryAsyncActivityDispatcher) SetCompletionHook(hook ports.AsyncCompletion) {
	d.mu.Lock()
	d.hook = hook
	d.mu.Unlock()
}

func (d *InMemoryAsyncActivityDispatcher) Dispatch(
	ctx context.Context,
	activityName string,
	payload map[string]interface{},
	jobID, stepID string,
	executor ports.IActivityExecutor,
) {
	d.DispatchStep(ctx, &models.WorkflowStep{ID: stepID, Activity: activityName, RetryPolicy: map[string]interface{}{"max_attempts": 1}}, payload, jobID, executor)
}

func (d *InMemoryAsyncActivityDispatcher) DispatchStep(_ context.Context, step *models.WorkflowStep, payload map[string]interface{}, jobID string, executor ports.IActivityExecutor) {
	if step == nil || step.Activity == "" || executor == nil {
		return
	}
	rp := models.StepRetryPolicy(step, models.RetryPolicy{MaxAttempts: 1, InitialInterval: 500 * time.Millisecond, BackoffCoefficient: 2})
	_, timeout := stepOptions(step)
	payloadCopy := copyMap(payload)
	go func() {
		res, err, attempts := runWithRetry(executor, step.Activity, payloadCopy, rp, timeout)
		if err != nil {
			log.Printf("⚡ [InMemoryAsyncDispatcher] Background activity '%s' (job=%s, step=%s) failed after %d attempt(s): %v", step.Activity, jobID, step.ID, attempts, err)
		}
		d.mu.RLock()
		hook := d.hook
		d.mu.RUnlock()
		if hook != nil {
			hook(jobID, step.ID, step.Activity, res, err)
		}
	}()
}

func (d *InMemoryAsyncActivityDispatcher) GetDispatcherType() string {
	return "IN_MEMORY"
}

// TemporalAsyncActivityDispatcher schedules durable activity execution with
// retries and hosts the SDK worker.
type TemporalAsyncActivityDispatcher struct {
	Host             string
	Namespace        string
	TaskQueue        string
	MaxRetries       int
	InitialBackoffMs int
	BackoffCoeff     float64
	// FallbackInMemory runs activities in-process when Temporal is
	// unreachable (not durable). When false, the dispatch fails loudly.
	FallbackInMemory bool
	// RedialAfter is how long to wait before dialling an unreachable
	// Temporal again (default 30s), so requests don't each pay a dial timeout.
	RedialAfter time.Duration
	// MaxHistory bounds GetDispatchedTasksHistory (default 500).
	MaxHistory int

	temporalClient   client.Client
	temporalWorker   worker.Worker
	workerStarted    bool
	stopCh           chan interface{}
	workerDone       chan struct{}
	executor         ports.IActivityExecutor
	hook             ports.AsyncCompletion
	unavailableUntil time.Time

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
		FallbackInMemory: true,
		RedialAfter:      30 * time.Second,
		MaxHistory:       500,
		tasksHistory:     make([]map[string]interface{}, 0),
	}
}

func (d *TemporalAsyncActivityDispatcher) SetCompletionHook(hook ports.AsyncCompletion) {
	d.mu.Lock()
	d.hook = hook
	d.mu.Unlock()
}

func (d *TemporalAsyncActivityDispatcher) completed(jobID, stepID, activityName string, res map[string]interface{}, err error) {
	d.mu.Lock()
	hook := d.hook
	d.mu.Unlock()
	if hook != nil && jobID != "" {
		hook(jobID, stepID, activityName, res, err)
	}
}

func (d *TemporalAsyncActivityDispatcher) GetClient() (client.Client, error) {
	d.mu.Lock()
	defer d.mu.Unlock()
	if d.temporalClient != nil {
		return d.temporalClient, nil
	}
	if time.Now().Before(d.unavailableUntil) {
		return nil, fmt.Errorf("temporal server at %s unavailable (retrying after %s)", d.Host, d.unavailableUntil.Format(time.RFC3339))
	}
	c, err := client.Dial(client.Options{
		HostPort:  d.Host,
		Namespace: d.Namespace,
	})
	if err != nil {
		redial := d.RedialAfter
		if redial <= 0 {
			redial = 30 * time.Second
		}
		d.unavailableUntil = time.Now().Add(redial)
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
	w.RegisterWorkflowWithOptions(IOctopusWorkflow, workflow.RegisterOptions{Name: "IOctopusWorkflow"})
	w.RegisterWorkflowWithOptions(OctopusReplicatedWorkflow, workflow.RegisterOptions{Name: "OctopusReplicatedWorkflow"})

	if executor != nil {
		for _, actName := range executor.ListActivities() {
			name := actName
			w.RegisterActivityWithOptions(func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
				jobID, _ := input[keyJobID].(string)
				stepID, _ := input[keyStepID].(string)
				maxAttempts := models.ToInt(input[keyMaxAttempts])
				clean := make(map[string]interface{}, len(input))
				for k, v := range input {
					if !strings.HasPrefix(k, "_octopus_") {
						clean[k] = v
					}
				}
				res, err := executor.ExecuteActivity(ctx, name, clean)
				// Report the final outcome only: success, or the last attempt.
				if err == nil || (maxAttempts > 0 && int(activity.GetInfo(ctx).Attempt) >= maxAttempts) {
					d.completed(jobID, stepID, name, res, err)
				}
				return res, err
			}, activity.RegisterOptions{Name: name})
		}
	}

	d.temporalWorker = w
	d.workerStarted = true
	d.stopCh = make(chan interface{})
	d.workerDone = make(chan struct{})
	stop, done := d.stopCh, d.workerDone

	go func() {
		defer close(done)
		log.Printf("👷 [TEMPORAL WORKER] Worker poller started on task queue '%s' (Host: %s, Namespace: %s)", d.TaskQueue, d.Host, d.Namespace)
		// Stopped by StopWorker; the host application owns process signals.
		if err := w.Run(stop); err != nil {
			log.Printf("⚠️ [TEMPORAL WORKER] Worker stopped with error: %v", err)
		}
	}()

	return nil
}

// StopWorker stops the worker (once; later calls are no-ops for it), waits
// for in-flight activities to finish, then closes the client.
func (d *TemporalAsyncActivityDispatcher) StopWorker() {
	d.mu.Lock()
	done := d.workerDone
	if d.stopCh != nil {
		close(d.stopCh)
		d.stopCh, d.workerDone = nil, nil
	}
	d.mu.Unlock()
	if done != nil {
		<-done // Run returns once the worker has stopped
	}

	d.mu.Lock()
	defer d.mu.Unlock()
	d.temporalWorker = nil
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

// Dispatch keeps the original signature (default retry policy).
func (d *TemporalAsyncActivityDispatcher) Dispatch(
	ctx context.Context,
	activityName string,
	payload map[string]interface{},
	jobID, stepID string,
	executor ports.IActivityExecutor,
) {
	d.DispatchStep(ctx, &models.WorkflowStep{ID: stepID, Activity: activityName}, payload, jobID, executor)
}

// DispatchStep starts a durable workflow for the step, honouring its
// timeout_seconds and retry_policy (e.g. max_attempts: 1 for "at most once").
func (d *TemporalAsyncActivityDispatcher) DispatchStep(ctx context.Context, step *models.WorkflowStep, payload map[string]interface{}, jobID string, executor ports.IActivityExecutor) {
	if step == nil || step.Activity == "" {
		return
	}
	activityName, stepID := step.Activity, step.ID
	rp, timeout := stepOptions(step)
	temporalWorkflowID := fmt.Sprintf("octopus_act_%s_%s_%s", jobID, stepID, activityName)
	taskRecord := map[string]interface{}{
		"temporal_workflow_id": temporalWorkflowID,
		"task_queue":           d.TaskQueue,
		"activity_name":        activityName,
		"job_id":               jobID,
		"step_id":              stepID,
		"max_attempts":         rp.MaxAttempts,
		"scheduled_at":         time.Now().UTC().Format(time.RFC3339),
		"status":               "SCHEDULED",
	}
	d.record(taskRecord)

	payloadCopy := copyMap(payload)
	payloadCopy["temporal_workflow_id"] = temporalWorkflowID
	payloadCopy["temporal_task_queue"] = d.TaskQueue
	payloadCopy[keyJobID] = jobID
	payloadCopy[keyStepID] = stepID
	payloadCopy[keyMaxAttempts] = rp.MaxAttempts

	wfInput := map[string]interface{}{
		"activity_name": activityName,
		"payload":       payloadCopy,
		"options": map[string]interface{}{
			"timeout_seconds":     timeout.Seconds(),
			"max_attempts":        rp.MaxAttempts,
			"initial_interval_ms": rp.InitialInterval.Milliseconds(),
			"backoff_coefficient": rp.BackoffCoefficient,
			"maximum_interval_ms": rp.MaximumInterval.Milliseconds(),
		},
	}

	c, err := d.GetClient()
	if err == nil {
		_, err = c.ExecuteWorkflow(context.Background(), client.StartWorkflowOptions{ID: temporalWorkflowID, TaskQueue: d.TaskQueue}, "IOctopusWorkflow", wfInput)
		if err == nil {
			d.setStatus(taskRecord, "DISPATCHED_TO_TEMPORAL")
			return
		}
	}
	if !d.FallbackInMemory {
		log.Printf("❌ [TEMPORAL DISPATCH] Activity '%s' (job=%s) not scheduled: %v", activityName, jobID, err)
		d.setStatus(taskRecord, "FAILED")
		d.completed(jobID, stepID, activityName, nil, fmt.Errorf("not scheduled: %w", err))
		return
	}
	log.Printf("⚠️ [TEMPORAL DISPATCH] %v. Running '%s' in-process (not durable).", err, activityName)
	d.setStatus(taskRecord, "FALLBACK_IN_MEMORY")
	go func() {
		res, runErr, attempts := runWithRetry(executor, activityName, copyMap(payload), rp, timeout)
		status := "COMPLETED"
		if runErr != nil {
			status = "FAILED"
		}
		d.mu.Lock()
		taskRecord["status"], taskRecord["attempts"] = status, attempts
		d.mu.Unlock()
		d.completed(jobID, stepID, activityName, res, runErr)
	}()
}

func (d *TemporalAsyncActivityDispatcher) record(rec map[string]interface{}) {
	d.mu.Lock()
	defer d.mu.Unlock()
	max := d.MaxHistory
	if max <= 0 {
		max = 500
	}
	d.tasksHistory = append(d.tasksHistory, rec)
	if len(d.tasksHistory) > max {
		d.tasksHistory = append([]map[string]interface{}(nil), d.tasksHistory[len(d.tasksHistory)-max:]...)
	}
}

func (d *TemporalAsyncActivityDispatcher) setStatus(rec map[string]interface{}, status string) {
	d.mu.Lock()
	rec["status"] = status
	d.mu.Unlock()
}

func (d *TemporalAsyncActivityDispatcher) GetDispatcherType() string {
	return "TEMPORAL"
}

func (d *TemporalAsyncActivityDispatcher) GetDispatchedTasksHistory() []map[string]interface{} {
	d.mu.Lock()
	defer d.mu.Unlock()
	res := make([]map[string]interface{}, len(d.tasksHistory))
	for i, rec := range d.tasksHistory {
		cp := make(map[string]interface{}, len(rec))
		for k, v := range rec {
			cp[k] = v
		}
		res[i] = cp
	}
	return res
}

func copyMap(src map[string]interface{}) map[string]interface{} {
	dst := make(map[string]interface{}, len(src))
	for k, v := range src {
		dst[k] = v
	}
	return dst
}
