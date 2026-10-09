package outbound

import (
	"context"
	"fmt"
	"sync"

	"github.com/octopus-workflow/octopus_live_engine/go/pkg/domain/ports"
)

type CallableActivityExecutor struct {
	activities map[string]ports.ActivityFunc
	mu         sync.RWMutex
}

func NewCallableActivityExecutor() *CallableActivityExecutor {
	return &CallableActivityExecutor{
		activities: make(map[string]ports.ActivityFunc),
	}
}

func (e *CallableActivityExecutor) RegisterActivity(name string, handler ports.ActivityFunc) {
	e.mu.Lock()
	defer e.mu.Unlock()
	e.activities[name] = handler
}

func (e *CallableActivityExecutor) HasActivity(name string) bool {
	e.mu.RLock()
	defer e.mu.RUnlock()
	_, exists := e.activities[name]
	return exists
}

func (e *CallableActivityExecutor) ListActivities() []string {
	e.mu.RLock()
	defer e.mu.RUnlock()
	names := make([]string, 0, len(e.activities))
	for name := range e.activities {
		names = append(names, name)
	}
	return names
}

func (e *CallableActivityExecutor) ExecuteActivity(
	ctx context.Context,
	name string,
	input map[string]interface{},
) (map[string]interface{}, error) {
	e.mu.RLock()
	handler, exists := e.activities[name]
	e.mu.RUnlock()

	if !exists {
		return nil, fmt.Errorf("activity handler '%s' not registered in Live Engine", name)
	}

	return handler(ctx, input)
}
