package models

import (
	"strconv"
	"strings"
	"time"
)

// RetryPolicy is read from a step's retry_policy map.
type RetryPolicy struct {
	MaxAttempts        int
	InitialInterval    time.Duration
	BackoffCoefficient float64
	MaximumInterval    time.Duration
}

// StepRetryPolicy parses retry_policy: {max_attempts, initial_interval_ms,
// backoff_coefficient, maximum_interval_ms}; def applies to unset fields.
func StepRetryPolicy(step *WorkflowStep, def RetryPolicy) RetryPolicy {
	rp := def
	if step == nil || step.RetryPolicy == nil {
		return rp
	}
	if n := ToInt(step.RetryPolicy["max_attempts"]); n > 0 {
		rp.MaxAttempts = n
	}
	if ms := ToFloat(step.RetryPolicy["initial_interval_ms"]); ms > 0 {
		rp.InitialInterval = time.Duration(ms) * time.Millisecond
	}
	if c := ToFloat(step.RetryPolicy["backoff_coefficient"]); c >= 1 {
		rp.BackoffCoefficient = c
	}
	if ms := ToFloat(step.RetryPolicy["maximum_interval_ms"]); ms > 0 {
		rp.MaximumInterval = time.Duration(ms) * time.Millisecond
	}
	return rp
}

// ToInt reads an int from YAML/JSON-decoded values (int, int64, float64, string).
func ToInt(v interface{}) int {
	switch n := v.(type) {
	case int:
		return n
	case int64:
		return int(n)
	case float64:
		return int(n)
	case string:
		i, _ := strconv.Atoi(strings.TrimSpace(n))
		return i
	}
	return 0
}

// ToFloat reads a float from YAML/JSON-decoded values.
func ToFloat(v interface{}) float64 {
	switch n := v.(type) {
	case int:
		return float64(n)
	case int64:
		return float64(n)
	case float64:
		return n
	case string:
		f, _ := strconv.ParseFloat(strings.TrimSpace(n), 64)
		return f
	}
	return 0
}
