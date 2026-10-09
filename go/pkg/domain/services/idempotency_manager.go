package services

import (
	"sync"
	"time"

	"github.com/octopus-workflow/octopus_live_engine/go/pkg/domain/models"
)

type cacheEntry struct {
	state     *models.ExecutionState
	expiresAt time.Time
}

type IdempotencyManager struct {
	cache   map[string]cacheEntry
	maxSize int
	ttl     time.Duration
	mu      sync.RWMutex
}

func NewIdempotencyManager(maxSize int, ttl time.Duration) *IdempotencyManager {
	return &IdempotencyManager{
		cache:   make(map[string]cacheEntry),
		maxSize: maxSize,
		ttl:     ttl,
	}
}

func (m *IdempotencyManager) Get(key string) (*models.ExecutionState, bool) {
	m.mu.RLock()
	defer m.mu.RUnlock()
	entry, exists := m.cache[key]
	if !exists || time.Now().After(entry.expiresAt) {
		return nil, false
	}
	return entry.state, true
}

func (m *IdempotencyManager) Set(key string, state *models.ExecutionState) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if len(m.cache) >= m.maxSize {
		// Clean expired or oldest
		now := time.Now()
		for k, v := range m.cache {
			if now.After(v.expiresAt) {
				delete(m.cache, k)
			}
		}
	}
	m.cache[key] = cacheEntry{
		state:     state,
		expiresAt: time.Now().Add(m.ttl),
	}
}
