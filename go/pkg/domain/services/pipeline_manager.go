package services

import (
	"context"
	"crypto/rand"
	"fmt"
	"strings"
	"time"

	"github.com/octopus-workflow/octopus_live_engine/go/pkg/domain/models"
	"github.com/octopus-workflow/octopus_live_engine/go/pkg/domain/ports"
)

type PipelineManager struct {
	blueprintRepo      ports.IBlueprintRepository
	activityExecutor   ports.IActivityExecutor
	stateStore         ports.IStateStore
	asyncDispatcher    ports.IAsyncActivityDispatcher
	conditionEvaluator *ConditionEvaluator
	sagaManager        *SagaManager
	idempotencyManager *IdempotencyManager
	options            models.EngineOptions
}

func NewPipelineManager(
	repo ports.IBlueprintRepository,
	executor ports.IActivityExecutor,
	store ports.IStateStore,
) *PipelineManager {
	return NewPipelineManagerWithDispatcher(repo, executor, store, nil)
}

func NewPipelineManagerWithDispatcher(
	repo ports.IBlueprintRepository,
	executor ports.IActivityExecutor,
	store ports.IStateStore,
	dispatcher ports.IAsyncActivityDispatcher,
) *PipelineManager {
	return &PipelineManager{
		blueprintRepo:      repo,
		activityExecutor:   executor,
		stateStore:         store,
		asyncDispatcher:    dispatcher,
		conditionEvaluator: NewConditionEvaluator(),
		sagaManager:        NewSagaManager(executor),
		idempotencyManager: NewIdempotencyManager(10000, 1*time.Hour),
	}
}

func (p *PipelineManager) SetAsyncDispatcher(dispatcher ports.IAsyncActivityDispatcher) {
	p.asyncDispatcher = dispatcher
}

// SetOptions applies engine options (see models.EngineOptions).
func (p *PipelineManager) SetOptions(opts models.EngineOptions) { p.options = opts }

func (p *PipelineManager) SetStateStore(store ports.IStateStore) {
	p.stateStore = store
}

func (p *PipelineManager) ExecuteSync(
	ctx context.Context,
	workflowName string,
	payload map[string]interface{},
) (*models.ExecutionState, error) {
	return p.ExecuteEndpointSync(ctx, workflowName, nil, "", "", payload)
}

func (p *PipelineManager) ExecuteEndpointSync(
	ctx context.Context,
	workflowName string,
	endpoint *models.APIEndpointConfig,
	route string,
	method string,
	payload map[string]interface{},
) (*models.ExecutionState, error) {
	pipelineStartTime := time.Now()
	bp, err := p.blueprintRepo.GetBlueprint(ctx, workflowName)
	if err != nil {
		return nil, fmt.Errorf("workflow blueprint '%s' not found: %w", workflowName, err)
	}

	jobID := ""
	if j, ok := payload["job_id"].(string); ok && j != "" {
		jobID = j
	} else {
		jobID = NewJobID()
	}

	// Job ids double as idempotency keys; scope them so a reused id can
	// never replay another workflow's or route's response.
	idemKey := workflowName + "|" + method + "|" + route + "|" + jobID
	if endpoint != nil {
		idemKey = workflowName + "|" + strings.ToUpper(endpoint.Method) + "|" + endpoint.Route + "|" + jobID
	}
	if cached, ok := p.idempotencyManager.Get(idemKey); ok {
		return cached, nil
	}

	stepsToRun := bp.Steps
	if endpoint != nil && len(endpoint.Steps) > 0 {
		stepsToRun = endpoint.Steps
	} else if len(stepsToRun) == 0 && len(bp.API) > 0 && len(bp.API[0].Steps) > 0 {
		stepsToRun = bp.API[0].Steps
	}

	reqRoute := route
	if reqRoute == "" && endpoint != nil {
		reqRoute = endpoint.Route
	}
	reqMethod := method
	if reqMethod == "" && endpoint != nil {
		reqMethod = endpoint.Method
	}

	clientHost := "127.0.0.1"
	if ch, ok := payload["client_host"].(string); ok && ch != "" {
		clientHost = ch
	} else if ci, ok := payload["client_ip"].(string); ok && ci != "" {
		clientHost = ci
	}

	state := &models.ExecutionState{
		JobID:        jobID,
		WorkflowName: workflowName,
		Status:       models.StatusRunning,
		Route:        reqRoute,
		Method:       reqMethod,
		ClientHost:   clientHost,
		TotalSteps:   len(stepsToRun),
		Data:         make(map[string]interface{}),
		Traces:       make([]models.StepExecutionTrace, 0),
		StartedAt:    models.NowUTC(),
	}

	// Merge initial payload
	for k, v := range payload {
		state.Data[k] = v
	}
	state.Data["job_id"] = jobID

	_ = p.save(ctx, state, bp)
	_ = p.stateStore.PublishEvent(ctx, jobID, models.LiveEvent{
		Event: "workflow_started",
		Data:  p.snapshot(state, bp),
	})

	var sagaActions []SagaAction
	steps := stepsToRun
	stepIndex := 0

	for stepIndex < len(steps) {
		step := steps[stepIndex]
		state.CurrentIndex = stepIndex
		state.CurrentStepID = step.ID

		// Pre-execution delay
		if step.DelaySeconds > 0 {
			time.Sleep(time.Duration(step.DelaySeconds * float64(time.Second)))
		}

		// Early termination / settlement
		if step.Terminate || step.Settle {
			reason := step.Reason
			if reason == "" {
				reason = "Workflow settled gracefully"
			}
			trace := models.StepExecutionTrace{
				StepID:        step.ID,
				ActivityName:  "terminate_settle",
				Status:        "COMPLETED",
				OutputPayload: p.maskPayload(map[string]interface{}{"settled": true, "reason": reason}, bp, &step),
				StartedAt:     models.NowUTC(),
				FinishedAt:    models.NowUTC(),
			}
			state.Traces = append(state.Traces, trace)
			state.Status = models.StatusCompleted
			break
		}

		// Declarative compensation trigger
		if step.Compensate || step.Rollback || step.TriggerCompensation {
			reason := step.Reason
			if reason == "" {
				reason = fmt.Sprintf("Declarative compensation triggered at step '%s'", step.ID)
			}
			state.Error = &reason
			state.Status = models.StatusRolledBack

			if bp.CompensationMode == "async" {
				actionsCopy := append([]SagaAction{}, sagaActions...)
				go func() {
					bgCtx := context.Background()
					_ = p.sagaManager.ExecuteCompensations(bgCtx, actionsCopy)
				}()
			} else {
				_ = p.sagaManager.ExecuteCompensations(ctx, sagaActions)
			}

			now := models.NowUTC()
			state.FinishedAt = &now
			state.DurationMs = float64(time.Since(pipelineStartTime).Microseconds()) / 1000.0
			_ = p.save(ctx, state, bp)
			_ = p.stateStore.PublishEvent(ctx, jobID, models.LiveEvent{
				Event: "workflow_rolled_back",
				Data:  p.snapshot(state, bp),
			})
			return state, fmt.Errorf("%s", reason)
		}

		// Evaluate branching condition if present
		hasBranches := len(step.GetThenSteps()) > 0 || len(step.GetElseSteps()) > 0 || step.Condition != nil || step.Conditions != nil || step.BranchingStep != ""
		if hasBranches {
			evalResult := p.conditionEvaluator.EvaluateStep(&step, state.Data)
			var branchSteps []models.WorkflowStep
			if evalResult {
				branchSteps = step.GetThenSteps()
			} else {
				branchSteps = step.GetElseSteps()
			}

			if len(branchSteps) > 0 {
				newSteps := append([]models.WorkflowStep{}, steps[:stepIndex+1]...)
				newSteps = append(newSteps, branchSteps...)
				newSteps = append(newSteps, steps[stepIndex+1:]...)
				steps = newSteps
				state.TotalSteps = len(steps)
			}

			if step.Activity == "" && step.Loop == nil && step.ForEach == nil {
				stepIndex++
				continue
			}
		}

		// Loop execution
		if step.Loop != nil || step.ForEach != nil {
			p.executeLoopStep(ctx, &step, state, bp)
			stepIndex++
			continue
		}

		// Activity Execution (Sync or Async)
		if step.Activity != "" {
			if step.Async || step.IsAsync {
				p.dispatchAsyncActivity(ctx, &step, state, bp)
			} else {
				_ = p.stateStore.PublishEvent(ctx, jobID, models.LiveEvent{
					Event: "step_started",
					Data: map[string]interface{}{
						"step_id":     step.ID,
						"activity":    step.Activity,
						"index":       stepIndex,
						"total_steps": state.TotalSteps,
					},
				})

				startT := time.Now()
				outPayload, actErr := p.activityExecutor.ExecuteActivity(ctx, step.Activity, state.Data)
				duration := float64(time.Since(startT).Microseconds()) / 1000.0

				trace := models.StepExecutionTrace{
					StepID:        step.ID,
					ActivityName:  step.Activity,
					InputPayload:  p.maskPayload(state.Data, bp, &step),
					OutputPayload: p.maskPayload(outPayload, bp, &step),
					StartedAt:     startT.UTC().Format(time.RFC3339Nano),
					FinishedAt:    models.NowUTC(),
					DurationMs:    duration,
				}

				if actErr != nil {
					trace.Status = "FAILED"
					trace.ErrorMessage = actErr.Error()
					state.Traces = append(state.Traces, trace)

					// Check step-level or blueprint middleware exception config mapping
					excCfg := p.findExceptionConfig(&step, bp, actErr)
					if excCfg != nil {
						msg := excCfg.Message
						if msg == "" {
							msg = actErr.Error()
						}
						code := excCfg.Code
						if code == 0 {
							code = 400
						}
						state.Error = &msg
						state.ErrorCode = &code
						state.ErrorType = &excCfg.Exception

						if excCfg.Activity != "" && p.activityExecutor != nil {
							actPayload := map[string]interface{}{
								"exception":       excCfg.Exception,
								"error":           excCfg.Exception,
								"error_type":      excCfg.Exception,
								"error_message":   actErr.Error(),
								"message":         msg,
								"status_code":     code,
								"job_id":          jobID,
								"workflow_name":   bp.WorkflowName,
								"failing_step_id": step.ID,
								"route":           state.Route,
								"path":            state.Route,
								"method":          state.Method,
								"client_host":     "127.0.0.1",
								"timestamp":       models.NowUTC(),
								"data":            p.snapshot(state, bp).Data,
							}

							if excCfg.GetIsAsync() {
								dispatcherType := "IN_MEMORY"
								if p.asyncDispatcher != nil {
									dispatcherType = p.asyncDispatcher.GetDispatcherType()
								}
								state.Traces = append(state.Traces, models.StepExecutionTrace{
									StepID:        "exc_" + strings.ToLower(excCfg.Activity),
									ActivityName:  excCfg.Activity,
									Status:        "ASYNC_DISPATCHED",
									IsAsync:       true,
									InputPayload:  p.maskPayload(actPayload, bp, &step),
									OutputPayload: p.maskPayload(map[string]interface{}{"async_dispatched": true, "dispatcher": dispatcherType}, bp, &step),
									StartedAt:     models.NowUTC(),
									FinishedAt:    models.NowUTC(),
								})

								if p.asyncDispatcher != nil {
									p.asyncDispatcher.Dispatch(ctx, excCfg.Activity, actPayload, jobID, "exc_"+strings.ToLower(excCfg.Activity), p.activityExecutor)
								} else {
									go func() {
										bgCtx := context.Background()
										_, _ = p.activityExecutor.ExecuteActivity(bgCtx, excCfg.Activity, actPayload)
									}()
								}
							} else {
								actStart := time.Now()
								actRes, actExecErr := p.activityExecutor.ExecuteActivity(ctx, excCfg.Activity, actPayload)
								durMs := float64(time.Since(actStart).Microseconds()) / 1000.0
								if actExecErr == nil {
									state.Traces = append(state.Traces, models.StepExecutionTrace{
										StepID:        "exc_" + strings.ToLower(excCfg.Activity),
										ActivityName:  excCfg.Activity,
										Status:        "COMPLETED",
										InputPayload:  p.maskPayload(actPayload, bp, &step),
										OutputPayload: p.maskPayload(actRes, bp, &step),
										DurationMs:    durMs,
										StartedAt:     models.NowUTC(),
										FinishedAt:    models.NowUTC(),
									})
								}
							}
						}
					} else {
						errMsg := actErr.Error()
						state.Error = &errMsg
						defaultCode := 500
						if mwErr, ok := actErr.(*OctopusMiddlewareError); ok {
							defaultCode = mwErr.Code
							state.ErrorType = &mwErr.Name
						}
						state.ErrorCode = &defaultCode
					}

					if len(sagaActions) > 0 {
						if bp.CompensationMode == "async" {
							actionsCopy := append([]SagaAction{}, sagaActions...)
							for _, action := range actionsCopy {
								if p.asyncDispatcher != nil {
									p.asyncDispatcher.Dispatch(ctx, action.Compensation, action.Payload, jobID, "saga_"+action.StepID, p.activityExecutor)
								} else {
									go func(act SagaAction) {
										bgCtx := context.Background()
										_, _ = p.activityExecutor.ExecuteActivity(bgCtx, act.Compensation, act.Payload)
									}(action)
								}
							}
						} else {
							_ = p.sagaManager.ExecuteCompensations(ctx, sagaActions)
						}
						state.Status = models.StatusRolledBack
					} else {
						state.Status = models.StatusFailed
					}

					now := models.NowUTC()
					state.FinishedAt = &now
					state.DurationMs = float64(time.Since(pipelineStartTime).Microseconds()) / 1000.0

					_ = p.save(ctx, state, bp)
					_ = p.stateStore.PublishEvent(ctx, jobID, models.LiveEvent{
						Event: "workflow_failed",
						Data:  p.snapshot(state, bp),
					})
					return state, actErr
				}

				trace.Status = "COMPLETED"
				state.Traces = append(state.Traces, trace)

				// Merge output to workflow data
				for k, v := range outPayload {
					state.Data[k] = v
				}

				if step.IsOutput || step.Output || state.OutputData == nil {
					state.OutputData = copyMap(outPayload)
				}

				if step.Compensation != "" {
					sagaActions = append(sagaActions, SagaAction{
						StepID:       step.ID,
						Compensation: step.Compensation,
						Payload:      copyMap(state.Data),
					})
				}

				_ = p.stateStore.PublishEvent(ctx, jobID, models.LiveEvent{
					Event: "step_completed",
					Data:  trace,
				})
			}
		}

		// Handle jump by step index
		if step.JumpToStepIndex != nil && *step.JumpToStepIndex >= 0 && *step.JumpToStepIndex < len(steps) {
			stepIndex = *step.JumpToStepIndex
			continue
		}
		if step.RewindToStepIndex != nil && *step.RewindToStepIndex >= 0 && *step.RewindToStepIndex < len(steps) {
			stepIndex = *step.RewindToStepIndex
			continue
		}

		// Handle named jump_to
		if step.JumpTo != "" {
			targetIdx := -1
			for i, s := range steps {
				if s.ID == step.JumpTo {
					targetIdx = i
					break
				}
			}
			if targetIdx >= 0 {
				stepIndex = targetIdx
				continue
			}
		}

		stepIndex++
	}

	if state.Status == models.StatusRunning {
		state.Status = models.StatusCompleted
	}
	if state.OutputData == nil {
		state.OutputData = copyMap(state.Data)
	}
	now := models.NowUTC()
	state.FinishedAt = &now
	state.DurationMs = float64(time.Since(pipelineStartTime).Microseconds()) / 1000.0
	p.idempotencyManager.Set(idemKey, replayCopy(state))
	_ = p.save(ctx, state, bp)

	_ = p.stateStore.PublishEvent(ctx, jobID, models.LiveEvent{
		Event: "workflow_completed",
		Data:  p.snapshot(state, bp),
	})

	return state, nil
}

func (p *PipelineManager) dispatchAsyncActivity(ctx context.Context, step *models.WorkflowStep, state *models.ExecutionState, bp *models.WorkflowBlueprint) {
	dispatcherType := "IN_MEMORY"
	if p.asyncDispatcher != nil {
		dispatcherType = p.asyncDispatcher.GetDispatcherType()
	}

	trace := models.StepExecutionTrace{
		StepID:        step.ID,
		ActivityName:  step.Activity,
		Status:        "ASYNC_DISPATCHED",
		IsAsync:       true,
		InputPayload:  p.maskPayload(state.Data, bp, step),
		OutputPayload: p.maskPayload(map[string]interface{}{"async_dispatched": true, "dispatcher": dispatcherType}, bp, step),
		StartedAt:     models.NowUTC(),
		FinishedAt:    models.NowUTC(),
	}
	state.Traces = append(state.Traces, trace)

	dataCopy := p.asyncPayload(step, state.Data, bp)
	trace.InputPayload = MaskPayload(dataCopy, nil, "")
	state.Traces[len(state.Traces)-1] = trace
	actName := step.Activity
	if p.asyncDispatcher != nil {
		p.asyncDispatcher.Dispatch(ctx, actName, dataCopy, state.JobID, step.ID, p.activityExecutor)
	} else {
		go func() {
			bgCtx := context.Background()
			_, _ = p.activityExecutor.ExecuteActivity(bgCtx, actName, dataCopy)
		}()
	}
}

func (p *PipelineManager) executeLoopStep(ctx context.Context, step *models.WorkflowStep, state *models.ExecutionState, bp *models.WorkflowBlueprint) {
	loopCfg := step.Loop
	if loopCfg == nil {
		loopCfg = step.ForEach
	}
	if loopCfg == nil {
		return
	}

	var items []interface{}
	itemsField, _ := loopCfg["items_field"].(string)
	if itemsField == "" {
		itemsField, _ = loopCfg["items"].(string)
	}
	if itemsField != "" {
		if rawVal, exists := state.Data[itemsField]; exists {
			if listVal, ok := rawVal.([]interface{}); ok {
				items = listVal
			}
		}
	} else if rawList, ok := loopCfg["items"].([]interface{}); ok {
		items = rawList
	}

	maxIter := 100
	if maxVal, ok := loopCfg["max_iterations"].(int); ok && maxVal > 0 {
		maxIter = maxVal
	}
	itemVarName, _ := loopCfg["item_variable"].(string)
	if itemVarName == "" {
		itemVarName = "item"
	}

	var loopResults []interface{}
	for i, item := range items {
		if i >= maxIter {
			break
		}
		itemPayload := copyMap(state.Data)
		itemPayload["_item"] = item
		itemPayload["_index"] = i
		itemPayload[itemVarName] = item

		if step.Activity != "" {
			out, err := p.activityExecutor.ExecuteActivity(ctx, step.Activity, itemPayload)
			if err == nil {
				loopResults = append(loopResults, out)
				for k, v := range out {
					state.Data[k] = v
				}
			}
		}
	}

	state.Data[fmt.Sprintf("%s_results", step.ID)] = loopResults
	trace := models.StepExecutionTrace{
		StepID:        step.ID,
		ActivityName:  step.Activity,
		Status:        "COMPLETED",
		InputPayload:  p.maskPayload(state.Data, bp, step),
		OutputPayload: p.maskPayload(map[string]interface{}{"iterations": len(loopResults), "results": loopResults}, bp, step),
		StartedAt:     models.NowUTC(),
		FinishedAt:    models.NowUTC(),
	}
	state.Traces = append(state.Traces, trace)
}

func (p *PipelineManager) InitiateAsync(
	ctx context.Context,
	workflowName string,
	payload map[string]interface{},
) (*models.ExecutionState, error) {
	bp, err := p.blueprintRepo.GetBlueprint(ctx, workflowName)
	if err != nil {
		return nil, fmt.Errorf("workflow blueprint '%s' not found: %w", workflowName, err)
	}
	// Work on a copy that carries the job id, so the returned id is the one
	// that actually runs and the caller's map is never shared.
	data := copyMap(payload)
	jobID, _ := data["job_id"].(string)
	if jobID == "" {
		jobID = NewJobID()
	}
	data["job_id"] = jobID

	state := &models.ExecutionState{
		JobID:        jobID,
		WorkflowName: workflowName,
		Status:       models.StatusRunning,
		StartedAt:    models.NowUTC(),
		Data:         data,
	}
	_ = p.save(ctx, state, bp)

	go func() {
		_, _ = p.ExecuteSync(context.Background(), workflowName, copyMap(data))
	}()
	return p.snapshot(state, bp), nil
}

func (p *PipelineManager) StreamEvents(ctx context.Context, jobID string) (<-chan models.LiveEvent, error) {
	ch, _, err := p.stateStore.SubscribeEvents(ctx, jobID)
	return ch, err
}

func (p *PipelineManager) GetExecutionState(ctx context.Context, jobID string) (*models.ExecutionState, error) {
	return p.stateStore.GetState(ctx, jobID)
}

func (p *PipelineManager) ListExecutionStates(ctx context.Context, workflowName string, limit int) ([]*models.ExecutionState, error) {
	return p.stateStore.ListStates(ctx, workflowName, limit)
}

func copyMap(m map[string]interface{}) map[string]interface{} {
	cp := make(map[string]interface{})
	for k, v := range m {
		cp[k] = v
	}
	return cp
}

func (p *PipelineManager) findExceptionConfig(step *models.WorkflowStep, blueprint *models.WorkflowBlueprint, err error) *models.MiddlewareExceptionConfig {
	if err == nil {
		return nil
	}

	errStr := err.Error()
	errType := ""
	if mwErr, ok := err.(*OctopusMiddlewareError); ok {
		errType = mwErr.Name
	}

	// 1. Check step-level declared exceptions first
	if step != nil && len(step.Exceptions) > 0 {
		for _, ec := range step.Exceptions {
			if errType != "" && (ec.Exception == errType || strings.EqualFold(ec.Exception, errType)) {
				return &ec
			}
			if ec.Exception != "" && strings.Contains(strings.ToLower(errStr), strings.ToLower(ec.Exception)) {
				return &ec
			}
		}
	}

	// 2. Check workflow-level middleware exceptions
	if blueprint != nil {
		for _, mw := range blueprint.Middlewares {
			for _, ec := range mw.Exceptions {
				// 1. Exact match on error type name
				if errType != "" && (ec.Exception == errType || strings.EqualFold(ec.Exception, errType)) {
					return &ec
				}
				// 2. Substring or prefix match in error string
				if ec.Exception != "" && strings.Contains(strings.ToLower(errStr), strings.ToLower(ec.Exception)) {
					return &ec
				}
			}
		}

		// 3. Middleware Wildcard '*'
		for _, mw := range blueprint.Middlewares {
			for _, ec := range mw.Exceptions {
				if ec.Exception == "*" || strings.EqualFold(ec.Exception, "exception") || strings.EqualFold(ec.Exception, "error") {
					return &ec
				}
			}
		}

		// 4. Global Exception Handler (Declarative Safety Net)
		var gh *models.GlobalExceptionHandlerConfig
		if blueprint.ErrorHandling != nil && blueprint.ErrorHandling.GlobalHandler != nil {
			gh = blueprint.ErrorHandling.GlobalHandler
		} else if blueprint.GlobalExceptionHandler != nil {
			gh = blueprint.GlobalExceptionHandler
		}

		if gh != nil && gh.Activity != "" {
			code := gh.Code
			if code == 0 {
				code = 500
			}
			return &models.MiddlewareExceptionConfig{
				Exception: "*",
				Code:      code,
				Message:   gh.Message,
				Activity:  gh.Activity,
				IsAsync:   gh.GetIsAsync(),
				Headers:   gh.Headers,
			}
		}
	}
	return nil
}

func (p *PipelineManager) maskPayload(payload map[string]interface{}, bp *models.WorkflowBlueprint, step *models.WorkflowStep) map[string]interface{} {
	if bp == nil {
		return copyMap(payload)
	}
	fields, maskVal := bp.GetMaskConfig(step)
	return MaskPayload(payload, fields, maskVal)
}

// snapshot is a deep, redacted copy of the state for storage and events:
// readers never share memory with the running execution, and fields declared
// sensitive anywhere in the blueprint (including struct fields) are masked.
func (p *PipelineManager) snapshot(state *models.ExecutionState, bp *models.WorkflowBlueprint) *models.ExecutionState {
	fields, maskValue := bp.AllMaskFields()
	cp := *state
	cp.Data = MaskPayload(state.Data, fields, maskValue)
	if state.OutputData != nil {
		cp.OutputData = MaskPayload(state.OutputData, fields, maskValue)
	}
	cp.Traces = make([]models.StepExecutionTrace, len(state.Traces))
	for i, t := range state.Traces {
		t.InputPayload = MaskPayload(t.InputPayload, fields, maskValue)
		t.OutputPayload = MaskPayload(t.OutputPayload, fields, maskValue)
		cp.Traces[i] = t
	}
	return &cp
}

func (p *PipelineManager) save(ctx context.Context, state *models.ExecutionState, bp *models.WorkflowBlueprint) error {
	return p.stateStore.SaveState(ctx, p.snapshot(state, bp))
}

// asyncPayload is what an async step receives (and a durable dispatcher
// stores): only the keys in step.Input when set, otherwise everything except
// fields declared sensitive.
func (p *PipelineManager) asyncPayload(step *models.WorkflowStep, data map[string]interface{}, bp *models.WorkflowBlueprint) map[string]interface{} {
	out := make(map[string]interface{})
	if len(step.Input) > 0 {
		for _, k := range step.Input {
			if v, ok := data[k]; ok {
				out[k] = v
			}
		}
		if v, ok := data["job_id"]; ok {
			out["job_id"] = v
		}
		return out
	}
	fields, _ := bp.AllMaskFields()
	sensitive := make(map[string]bool, len(fields))
	for _, f := range fields {
		sensitive[strings.ToLower(f)] = true
	}
	for k, v := range data {
		if !sensitive[strings.ToLower(k)] && !sensitive[strings.ToLower(strings.TrimPrefix(k, "_injected_"))] {
			out[k] = v
		}
	}
	return out
}

// NewJobID returns an unguessable execution id.
func NewJobID() string {
	b := make([]byte, 16)
	_, _ = rand.Read(b)
	b[6], b[8] = (b[6]&0x0f)|0x40, (b[8]&0x3f)|0x80
	return fmt.Sprintf("job_%x-%x-%x-%x-%x", b[0:4], b[4:6], b[6:8], b[8:10], b[10:])
}

// replayCopy keeps what an idempotent retry needs: the outcome and output,
// deep-copied, without the working data.
func replayCopy(state *models.ExecutionState) *models.ExecutionState {
	cp := *state
	cp.Data = map[string]interface{}{"job_id": state.JobID}
	cp.OutputData = copyMapRecursive(state.OutputData)
	cp.Traces = nil
	return &cp
}
