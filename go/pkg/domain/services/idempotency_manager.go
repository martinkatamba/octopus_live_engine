package services

import (
	"container/list"
	"sync"
	"time"

	"github.com/octopus-workflow/octopus_live_engine/go/pkg/domain/models"
)

type cacheEntry struct {
	key       string
	state     *models.ExecutionState
	expiresAt time.Time
}

// IdempotencyManager remembers recent outcomes by key for replay. It holds at
// most maxSize entries (least recently stored are evicted first) for ttl.
type IdempotencyManager struct {
	mu      sync.Mutex
	maxSize int
	ttl     time.Duration
	order   *list.List // front = newest
	index   map[string]*list.Element
}

func NewIdempotencyManager(maxSize int, ttl time.Duration) *IdempotencyManager {
	if maxSize <= 0 {
		maxSize = 1000
	}
	return &IdempotencyManager{maxSize: maxSize, ttl: ttl, order: list.New(), index: make(map[string]*list.Element)}
}

func (m *IdempotencyManager) Get(key string) (*models.ExecutionState, bool) {
	m.mu.Lock()
	defer m.mu.Unlock()
	el, ok := m.index[key]
	if !ok {
		return nil, false
	}
	entry := el.Value.(*cacheEntry)
	if time.Now().After(entry.expiresAt) {
		m.order.Remove(el)
		delete(m.index, key)
		return nil, false
	}
	return entry.state, true
}

func (m *IdempotencyManager) Set(key string, state *models.ExecutionState) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if el, ok := m.index[key]; ok {
		m.order.Remove(el)
	}
	m.index[key] = m.order.PushFront(&cacheEntry{key: key, state: state, expiresAt: time.Now().Add(m.ttl)})
	for m.order.Len() > m.maxSize {
		oldest := m.order.Back()
		m.order.Remove(oldest)
		delete(m.index, oldest.Value.(*cacheEntry).key)
	}
}

// Len reports the number of cached entries.
func (m *IdempotencyManager) Len() int {
	m.mu.Lock()
	defer m.mu.Unlock()
	return m.order.Len()
}
