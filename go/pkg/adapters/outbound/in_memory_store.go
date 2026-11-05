package outbound

import (
	"context"
	"fmt"
	"sort"
	"sync"

	"github.com/martinkatamba/octopus_live_engine/go/pkg/domain/models"
)

// InMemoryStateStore keeps the most recent executions (default 1000) in
// memory. Older ones are evicted, so memory stays bounded.
type InMemoryStateStore struct {
	states      map[string]*models.ExecutionState
	order       []string // insertion order, oldest first
	limit       int
	subscribers map[string][]chan models.LiveEvent
	mu          sync.RWMutex
}

const defaultStoredExecutions = 1000

// SetLimit bounds the number of stored executions (<= 0 = default).
func (s *InMemoryStateStore) SetLimit(n int) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if n <= 0 {
		n = defaultStoredExecutions
	}
	s.limit = n
	s.evict()
}

func (s *InMemoryStateStore) evict() {
	for len(s.order) > s.limit {
		delete(s.states, s.order[0])
		s.order = s.order[1:]
	}
}

func NewInMemoryStateStore() *InMemoryStateStore {
	return &InMemoryStateStore{
		states:      make(map[string]*models.ExecutionState),
		subscribers: make(map[string][]chan models.LiveEvent),
	}
}

func (s *InMemoryStateStore) SaveState(ctx context.Context, state *models.ExecutionState) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.limit == 0 {
		s.limit = defaultStoredExecutions
	}
	if _, exists := s.states[state.JobID]; !exists {
		s.order = append(s.order, state.JobID)
	}
	s.states[state.JobID] = state
	s.evict()
	return nil
}

func (s *InMemoryStateStore) GetState(ctx context.Context, jobID string) (*models.ExecutionState, error) {
	s.mu.RLock()
	defer s.mu.RUnlock()
	state, exists := s.states[jobID]
	if !exists {
		return nil, fmt.Errorf("state for job '%s' not found", jobID)
	}
	return state, nil
}

func (s *InMemoryStateStore) ListStates(ctx context.Context, workflowName string, limit int) ([]*models.ExecutionState, error) {
	s.mu.RLock()
	defer s.mu.RUnlock()

	var result []*models.ExecutionState
	for _, state := range s.states {
		if workflowName == "" || state.WorkflowName == workflowName {
			result = append(result, state)
		}
	}

	sort.Slice(result, func(i, j int) bool {
		return result[i].StartedAt > result[j].StartedAt
	})

	if limit <= 0 {
		limit = 100
	}
	if len(result) > limit {
		result = result[:limit]
	}

	return result, nil
}

func (s *InMemoryStateStore) PublishEvent(ctx context.Context, jobID string, event models.LiveEvent) error {
	s.mu.RLock()
	defer s.mu.RUnlock()
	if subs, exists := s.subscribers[jobID]; exists {
		for _, ch := range subs {
			select {
			case ch <- event:
			default:
			}
		}
	}
	return nil
}

func (s *InMemoryStateStore) SubscribeEvents(
	ctx context.Context,
	jobID string,
) (<-chan models.LiveEvent, func(), error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	ch := make(chan models.LiveEvent, 50)
	s.subscribers[jobID] = append(s.subscribers[jobID], ch)

	cleanup := func() {
		s.mu.Lock()
		defer s.mu.Unlock()
		if subs, ok := s.subscribers[jobID]; ok {
			var newSubs []chan models.LiveEvent
			for _, sub := range subs {
				if sub != ch {
					newSubs = append(newSubs, sub)
				}
			}
			if len(newSubs) == 0 {
				delete(s.subscribers, jobID)
			} else {
				s.subscribers[jobID] = newSubs
			}
			close(ch)
		}
	}

	return ch, cleanup, nil
}
