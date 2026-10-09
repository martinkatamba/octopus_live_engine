package ports

import (
	"context"

	"github.com/octopus-workflow/octopus_live_engine/go/pkg/domain/models"
)

type ActivityFunc func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error)

type IActivityExecutor interface {
	ExecuteActivity(ctx context.Context, activityName string, input map[string]interface{}) (map[string]interface{}, error)
	RegisterActivity(activityName string, handler ActivityFunc)
	HasActivity(activityName string) bool
	ListActivities() []string
}

type IBlueprintRepository interface {
	GetBlueprint(ctx context.Context, workflowName string) (*models.WorkflowBlueprint, error)
	ListBlueprints(ctx context.Context) ([]models.WorkflowBlueprint, error)
	SaveBlueprint(ctx context.Context, blueprint *models.WorkflowBlueprint) error
}

type IStateStore interface {
	SaveState(ctx context.Context, state *models.ExecutionState) error
	GetState(ctx context.Context, jobID string) (*models.ExecutionState, error)
	ListStates(ctx context.Context, workflowName string, limit int) ([]*models.ExecutionState, error)
	PublishEvent(ctx context.Context, jobID string, event models.LiveEvent) error
	SubscribeEvents(ctx context.Context, jobID string) (<-chan models.LiveEvent, func(), error)
}

type IAsyncActivityDispatcher interface {
	Dispatch(ctx context.Context, activityName string, payload map[string]interface{}, jobID, stepID string, executor IActivityExecutor)
	GetDispatcherType() string
}

// AsyncCompletion reports the final outcome of an async step that ran in this
// process, so the execution history can show it.
type AsyncCompletion func(jobID, stepID, activity string, result map[string]interface{}, err error)

// IStepAwareDispatcher is optionally implemented by dispatchers that honour a
// step's timeout_seconds / retry_policy and report outcomes.
type IStepAwareDispatcher interface {
	DispatchStep(ctx context.Context, step *models.WorkflowStep, payload map[string]interface{}, jobID string, executor IActivityExecutor)
	SetCompletionHook(hook AsyncCompletion)
}
