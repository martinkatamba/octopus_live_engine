package services

import (
	"context"
	"fmt"

	"github.com/octopus-workflow/octopus_live_engine/go/pkg/domain/ports"
)

type SagaAction struct {
	StepID       string
	Compensation string
	Payload      map[string]interface{}
}

type SagaManager struct {
	activityExecutor ports.IActivityExecutor
}

func NewSagaManager(executor ports.IActivityExecutor) *SagaManager {
	return &SagaManager{activityExecutor: executor}
}

func (s *SagaManager) ExecuteCompensations(ctx context.Context, actions []SagaAction) []error {
	var errs []error
	// Execute in reverse order
	for i := len(actions) - 1; i >= 0; i-- {
		action := actions[i]
		if action.Compensation == "" {
			continue
		}
		if s.activityExecutor.HasActivity(action.Compensation) {
			_, err := s.activityExecutor.ExecuteActivity(ctx, action.Compensation, action.Payload)
			if err != nil {
				errs = append(errs, fmt.Errorf("compensation '%s' failed for step '%s': %w", action.Compensation, action.StepID, err))
			}
		}
	}
	return errs
}
