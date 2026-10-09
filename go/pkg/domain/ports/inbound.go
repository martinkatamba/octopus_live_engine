package ports

import (
	"context"

	"github.com/octopus-workflow/octopus_live_engine/go/pkg/domain/models"
)

type ILiveWorkflowUseCase interface {
	ExecuteSync(ctx context.Context, workflowName string, payload map[string]interface{}) (*models.ExecutionState, error)
	InitiateAsync(ctx context.Context, workflowName string, payload map[string]interface{}) (*models.ExecutionState, error)
	StreamEvents(ctx context.Context, jobID string) (<-chan models.LiveEvent, error)
	GetExecutionState(ctx context.Context, jobID string) (*models.ExecutionState, error)
	ListExecutionStates(ctx context.Context, workflowName string, limit int) ([]*models.ExecutionState, error)
}

type IVisualizerUseCase interface {
	CompileX6Diagram(ctx context.Context, workflowName string, jobID *string, route *string, method *string) (map[string]interface{}, error)
	ListBlueprints(ctx context.Context) ([]models.WorkflowBlueprint, error)
}
