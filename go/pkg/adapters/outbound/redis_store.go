package outbound

import (
	"context"
	"encoding/json"
	"fmt"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/octopus-workflow/octopus_live_engine/go/pkg/domain/models"
	"github.com/redis/go-redis/v9"
)

// RedisStateStore implements IStateStore with Redis backend and dynamic differential retention.
type RedisStateStore struct {
	client      *redis.Client
	prefix      string
	ttlSuccess  time.Duration
	ttlFailure  time.Duration
	ttlRunning  time.Duration
	subscribers map[string][]chan models.LiveEvent
	fallback    map[string]*models.ExecutionState
	mu          sync.RWMutex
}

func NewRedisStateStore(redisURL string, ttlSuccess, ttlFailure, ttlRunning time.Duration, prefix string) (*RedisStateStore, error) {
	if prefix == "" {
		prefix = "octopus:live"
	}
	if ttlSuccess <= 0 {
		ttlSuccess = 1 * time.Hour
	}
	if ttlFailure <= 0 {
		ttlFailure = 7 * 24 * time.Hour
	}
	if ttlRunning <= 0 {
		ttlRunning = 2 * time.Hour
	}

	opts, err := redis.ParseURL(redisURL)
	if err != nil {
		// Fallback to direct Addr if standard URL parse fails
		opts = &redis.Options{
			Addr: "localhost:6379",
		}
	}

	rdb := redis.NewClient(opts)
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()

	if err := rdb.Ping(ctx).Err(); err != nil {
		return nil, fmt.Errorf("redis ping failed: %w", err)
	}

	return &RedisStateStore{
		client:      rdb,
		prefix:      prefix,
		ttlSuccess:  ttlSuccess,
		ttlFailure:  ttlFailure,
		ttlRunning:  ttlRunning,
		subscribers: make(map[string][]chan models.LiveEvent),
		fallback:    make(map[string]*models.ExecutionState),
	}, nil
}

func (s *RedisStateStore) computeTTL(status models.ExecutionStatus) time.Duration {
	st := strings.ToUpper(string(status))
	if st == "FAILED" || st == "ERROR" || st == "TIMEOUT" || st == "ROLLED_BACK" {
		return s.ttlFailure
	} else if st == "COMPLETED" || st == "SUCCESS" {
		return s.ttlSuccess
	}
	return s.ttlRunning
}

func (s *RedisStateStore) stateKey(jobID string) string {
	return fmt.Sprintf("%s:state:%s", s.prefix, jobID)
}

func (s *RedisStateStore) traceKey(jobID string) string {
	return fmt.Sprintf("%s:traces:%s", s.prefix, jobID)
}

func (s *RedisStateStore) eventChannel(jobID string) string {
	return fmt.Sprintf("%s:events:%s", s.prefix, jobID)
}

func (s *RedisStateStore) SaveState(ctx context.Context, state *models.ExecutionState) error {
	if s.client == nil {
		s.mu.Lock()
		s.fallback[state.JobID] = state
		s.mu.Unlock()
		return nil
	}

	ttl := s.computeTTL(state.Status)

	// Serialize state
	data, err := json.Marshal(state)
	if err != nil {
		return err
	}

	pipe := s.client.Pipeline()
	pipe.Set(ctx, s.stateKey(state.JobID), data, ttl)

	if len(state.Traces) > 0 {
		tracesData, err := json.Marshal(state.Traces)
		if err == nil {
			pipe.Set(ctx, s.traceKey(state.JobID), tracesData, ttl)
		}
	}

	_, err = pipe.Exec(ctx)
	if err != nil {
		s.mu.Lock()
		s.fallback[state.JobID] = state
		s.mu.Unlock()
	}
	return err
}

func (s *RedisStateStore) GetState(ctx context.Context, jobID string) (*models.ExecutionState, error) {
	if s.client != nil {
		val, err := s.client.Get(ctx, s.stateKey(jobID)).Result()
		if err == nil {
			var state models.ExecutionState
			if err := json.Unmarshal([]byte(val), &state); err == nil {
				// fetch traces if present
				tracesVal, err := s.client.Get(ctx, s.traceKey(jobID)).Result()
				if err == nil {
					var traces []models.StepExecutionTrace
					if err := json.Unmarshal([]byte(tracesVal), &traces); err == nil {
						state.Traces = traces
					}
				}
				return &state, nil
			}
		} else if err == redis.Nil {
			return nil, fmt.Errorf("state for job '%s' not found", jobID)
		}
	}

	s.mu.RLock()
	defer s.mu.RUnlock()
	if st, ok := s.fallback[jobID]; ok {
		return st, nil
	}
	return nil, fmt.Errorf("state for job '%s' not found", jobID)
}

func (s *RedisStateStore) ListStates(ctx context.Context, workflowName string, limit int) ([]*models.ExecutionState, error) {
	if s.client != nil {
		pattern := fmt.Sprintf("%s:state:*", s.prefix)
		keys, err := s.client.Keys(ctx, pattern).Result()
		if err == nil {
			var results []*models.ExecutionState
			for _, k := range keys {
				val, err := s.client.Get(ctx, k).Result()
				if err == nil {
					var state models.ExecutionState
					if err := json.Unmarshal([]byte(val), &state); err == nil {
						if workflowName == "" || state.WorkflowName == workflowName {
							results = append(results, &state)
						}
					}
				}
			}

			sort.Slice(results, func(i, j int) bool {
				return results[i].StartedAt > results[j].StartedAt
			})

			if limit <= 0 {
				limit = 100
			}
			if len(results) > limit {
				results = results[:limit]
			}
			return results, nil
		}
	}

	s.mu.RLock()
	defer s.mu.RUnlock()
	var result []*models.ExecutionState
	for _, state := range s.fallback {
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

func (s *RedisStateStore) PublishEvent(ctx context.Context, jobID string, event models.LiveEvent) error {
	s.mu.RLock()
	if subs, exists := s.subscribers[jobID]; exists {
		for _, ch := range subs {
			select {
			case ch <- event:
			default:
			}
		}
	}
	s.mu.RUnlock()

	if s.client != nil {
		data, err := json.Marshal(event)
		if err == nil {
			s.client.Publish(ctx, s.eventChannel(jobID), data)
		}
	}
	return nil
}

func (s *RedisStateStore) SubscribeEvents(ctx context.Context, jobID string) (<-chan models.LiveEvent, func(), error) {
	s.mu.Lock()
	defer s.mu.Unlock()

	eventChan := make(chan models.LiveEvent, 50)
	s.subscribers[jobID] = append(s.subscribers[jobID], eventChan)

	unsubscribe := func() {
		s.mu.Lock()
		defer s.mu.Unlock()

		if subs, exists := s.subscribers[jobID]; exists {
			var newSubs []chan models.LiveEvent
			for _, ch := range subs {
				if ch != eventChan {
					newSubs = append(newSubs, ch)
				}
			}
			if len(newSubs) == 0 {
				delete(s.subscribers, jobID)
			} else {
				s.subscribers[jobID] = newSubs
			}
		}
		close(eventChan)
	}

	return eventChan, unsubscribe, nil
}
