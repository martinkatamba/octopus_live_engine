package services

import (
	"context"
	"fmt"

	"github.com/martinkatamba/octopus_live_engine/go/pkg/domain/ports"
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
	_, errs := s.RunCompensations(ctx, actions)
	return errs
}

// RunCompensations runs the compensations in reverse order and returns their
// merged output, which callers merge into the job data (decision D4).
func (s *SagaManager) RunCompensations(ctx context.Context, actions []SagaAction) (map[string]interface{}, []error) {
	output := map[string]interface{}{}
	var errs []error
	for i := len(actions) - 1; i >= 0; i-- {
		action := actions[i]
		if action.Compensation == "" {
			continue
		}
		if s.activityExecutor.HasActivity(action.Compensation) {
			out, err := s.activityExecutor.ExecuteActivity(ctx, action.Compensation, action.Payload)
			if err != nil {
				errs = append(errs, fmt.Errorf("compensation '%s' failed for step '%s': %w", action.Compensation, action.StepID, err))
				continue
			}
			for k, v := range out {
				output[k] = v
			}
		}
	}
	return output, errs
}
