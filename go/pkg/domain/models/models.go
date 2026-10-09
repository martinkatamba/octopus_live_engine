package models

import (
	"strings"
	"time"
)

type ExecutionStatus string

const (
	StatusIdle       ExecutionStatus = "IDLE"
	StatusRunning    ExecutionStatus = "RUNNING"
	StatusCompleted  ExecutionStatus = "COMPLETED"
	StatusFailed     ExecutionStatus = "FAILED"
	StatusRolledBack ExecutionStatus = "ROLLED_BACK"
	StatusCancelled  ExecutionStatus = "CANCELLED"
)

type StepType string

const (
	StepTypeActivity     StepType = "activity"
	StepTypeBranch       StepType = "branching_step"
	StepTypeLoop         StepType = "loop"
	StepTypeCompensation StepType = "compensation"
)

type StepStatus string

const (
	StepStatusPending        StepStatus = "PENDING"
	StepStatusRunning        StepStatus = "RUNNING"
	StepStatusCompleted      StepStatus = "COMPLETED"
	StepStatusFailed         StepStatus = "FAILED"
	StepStatusCompensated    StepStatus = "COMPENSATED"
	StepStatusAwaitingSignal StepStatus = "AWAITING_SIGNAL"
	StepStatusAwaitingReview StepStatus = "AWAITING_APPROVAL"
	StepStatusSlaBreach      StepStatus = "SLA_BREACH"
	StepStatusBypassed       StepStatus = "BYPASSED"
	StepStatusRewound        StepStatus = "REWOUND"
)

type StepProgress struct {
	StepID         string                 `json:"step_id"`
	Activity       string                 `json:"activity,omitempty"`
	StepType       StepType               `json:"step_type"`
	Status         StepStatus             `json:"status"`
	StartedAt      time.Time              `json:"started_at"`
	CompletedAt    *time.Time             `json:"completed_at,omitempty"`
	InputPayload   map[string]interface{} `json:"input_payload,omitempty"`
	OutputPayload  map[string]interface{} `json:"output_payload,omitempty"`
	ErrorMessage   string                 `json:"error_message,omitempty"`
	IdempotencyKey string                 `json:"idempotency_key,omitempty"`
	Attempts       int                    `json:"attempts"`
}

type EngineProgress struct {
	WorkflowID    string                 `json:"workflow_id"`
	RunID         string                 `json:"run_id"`
	WorkflowName  string                 `json:"workflow_name"`
	TaskQueue     string                 `json:"task_queue"`
	CurrentStatus StepStatus             `json:"current_status"`
	CurrentStepID string                 `json:"current_step_id,omitempty"`
	CurrentIndex  int                    `json:"current_index"`
	TotalSteps    int                    `json:"total_steps"`
	StartedAt     time.Time              `json:"started_at"`
	FinishedAt    *time.Time             `json:"finished_at,omitempty"`
	JobData       map[string]interface{} `json:"job_data"`
	Steps         []StepProgress         `json:"steps"`
	Compensations []string               `json:"compensations,omitempty"`
	Error         string                 `json:"error,omitempty"`
}

type ConditionClause struct {
	Variable string      `json:"variable,omitempty" yaml:"variable,omitempty"`
	Field    string      `json:"field,omitempty" yaml:"field,omitempty"`
	Operator string      `json:"operator,omitempty" yaml:"operator,omitempty"`
	Value    interface{} `json:"value,omitempty" yaml:"value,omitempty"`
}

type ConditionsGroup struct {
	Match      string            `json:"match,omitempty" yaml:"match,omitempty"`
	Conditions []ConditionClause `json:"conditions,omitempty" yaml:"conditions,omitempty"`
}

type StepCondition struct {
	Rule      string         `json:"rule,omitempty" yaml:"rule,omitempty"`
	Operator  string         `json:"operator,omitempty" yaml:"operator,omitempty"`
	Field     string         `json:"field,omitempty" yaml:"field,omitempty"`
	Variable  string         `json:"variable,omitempty" yaml:"variable,omitempty"`
	Value     interface{}    `json:"value,omitempty" yaml:"value,omitempty"`
	ThenSteps []WorkflowStep `json:"then_steps,omitempty" yaml:"then_steps,omitempty"`
	ElseSteps []WorkflowStep `json:"else_steps,omitempty" yaml:"else_steps,omitempty"`
}

type WorkflowStep struct {
	ID                  string                 `json:"id" yaml:"id"`
	Activity            string                 `json:"activity,omitempty" yaml:"activity,omitempty"`
	BranchingStep       string                 `json:"branching_step,omitempty" yaml:"branching_step,omitempty"`
	HumanStep           string                 `json:"human_step,omitempty" yaml:"human_step,omitempty"`
	Description         string                 `json:"description,omitempty" yaml:"description,omitempty"`
	TimeoutSeconds      int                    `json:"timeout_seconds,omitempty" yaml:"timeout_seconds,omitempty"`
	DelaySeconds        float64                `json:"delay_seconds,omitempty" yaml:"delay_seconds,omitempty"`
	Compensation        string                 `json:"compensation,omitempty" yaml:"compensation,omitempty"`
	Condition           *StepCondition         `json:"condition,omitempty" yaml:"condition,omitempty"`
	Conditions          *ConditionsGroup       `json:"conditions,omitempty" yaml:"conditions,omitempty"`
	ThenSteps           []WorkflowStep         `json:"then_steps,omitempty" yaml:"then_steps,omitempty"`
	ElseSteps           []WorkflowStep         `json:"else_steps,omitempty" yaml:"else_steps,omitempty"`
	Then                []WorkflowStep         `json:"then,omitempty" yaml:"then,omitempty"`
	Else                []WorkflowStep         `json:"else,omitempty" yaml:"else,omitempty"`
	Loop                map[string]interface{} `json:"loop,omitempty" yaml:"loop,omitempty"`
	ForEach             map[string]interface{} `json:"for_each,omitempty" yaml:"for_each,omitempty"`
	RetryPolicy         map[string]interface{} `json:"retry_policy,omitempty" yaml:"retry_policy,omitempty"`
	JumpTo              string                 `json:"jump_to,omitempty" yaml:"jump_to,omitempty"`
	JumpToStepIndex     *int                   `json:"jump_to_step_index,omitempty" yaml:"jump_to_step_index,omitempty"`
	RewindToStepIndex   *int                   `json:"rewind_to_step_index,omitempty" yaml:"rewind_to_step_index,omitempty"`
	Async               bool                   `json:"async,omitempty" yaml:"async,omitempty"`
	IsAsync             bool                   `json:"is_async,omitempty" yaml:"is_async,omitempty"`
	Compensate          bool                   `json:"compensate,omitempty" yaml:"compensate,omitempty"`
	Rollback            bool                   `json:"rollback,omitempty" yaml:"rollback,omitempty"`
	TriggerCompensation bool                   `json:"trigger_compensation,omitempty" yaml:"trigger_compensation,omitempty"`
	Terminate           bool                   `json:"terminate,omitempty" yaml:"terminate,omitempty"`
	Settle              bool                   `json:"settle,omitempty" yaml:"settle,omitempty"`
	Reason              string                 `json:"reason,omitempty" yaml:"reason,omitempty"`
	DynamicBranching    map[string]interface{} `json:"dynamic_branching,omitempty" yaml:"dynamic_branching,omitempty"`
	IsOutput            bool                       `json:"is_output,omitempty" yaml:"is_output,omitempty"`
	Output              bool                       `json:"output,omitempty" yaml:"output,omitempty"`
	Exceptions          []MiddlewareExceptionConfig `json:"exceptions,omitempty" yaml:"exceptions,omitempty"`
	MaskFields          []string                   `json:"mask_fields,omitempty" yaml:"mask_fields,omitempty"`
	Masking             *MaskingConfig             `json:"masking,omitempty" yaml:"masking,omitempty"`
}

func (s *WorkflowStep) GetThenSteps() []WorkflowStep {
	if len(s.ThenSteps) > 0 {
		return s.ThenSteps
	}
	if len(s.Then) > 0 {
		return s.Then
	}
	if s.Condition != nil && len(s.Condition.ThenSteps) > 0 {
		return s.Condition.ThenSteps
	}
	return nil
}

func (s *WorkflowStep) GetElseSteps() []WorkflowStep {
	if len(s.ElseSteps) > 0 {
		return s.ElseSteps
	}
	if len(s.Else) > 0 {
		return s.Else
	}
	if s.Condition != nil && len(s.Condition.ElseSteps) > 0 {
		return s.Condition.ElseSteps
	}
	return nil
}

type APIEndpointConfig struct {
	Route       string         `json:"route" yaml:"route"`
	Method      string         `json:"method" yaml:"method"`
	InputModel  string         `json:"input_model,omitempty" yaml:"input_model,omitempty"`
	OutputModel string         `json:"output_model,omitempty" yaml:"output_model,omitempty"`
	StatusCode  int            `json:"status_code,omitempty" yaml:"status_code,omitempty"`
	Summary     string         `json:"summary,omitempty" yaml:"summary,omitempty"`
	Description string         `json:"description,omitempty" yaml:"description,omitempty"`
	Tags        []string       `json:"tags,omitempty" yaml:"tags,omitempty"`
	Steps       []WorkflowStep `json:"steps,omitempty" yaml:"steps,omitempty"`
}

type APIEndpoints []APIEndpointConfig

func (a *APIEndpoints) UnmarshalYAML(unmarshal func(interface{}) error) error {
	var list []APIEndpointConfig
	if err := unmarshal(&list); err == nil {
		*a = list
		return nil
	}
	var obj struct {
		ID         string              `yaml:"id"`
		BasePrefix string              `yaml:"base_prefix"`
		BaseURL    string              `yaml:"base_url"`
		Version    string              `yaml:"version"`
		Endpoints  []APIEndpointConfig `yaml:"endpoints"`
		Routes     []APIEndpointConfig `yaml:"routes"`
	}
	if err := unmarshal(&obj); err == nil && (len(obj.Endpoints) > 0 || len(obj.Routes) > 0) {
		if len(obj.Endpoints) > 0 {
			*a = obj.Endpoints
		} else {
			*a = obj.Routes
		}
		return nil
	}
	var single APIEndpointConfig
	if err := unmarshal(&single); err == nil && single.Route != "" {
		*a = []APIEndpointConfig{single}
		return nil
	}
	return nil
}

func (b *WorkflowBlueprint) NormalizeEndpoints() {
	if len(b.API) == 0 {
		return
	}
	wfName := strings.Trim(b.WorkflowName, "/ ")
	ver := strings.Trim(b.Version, "/ ")
	if ver == "" {
		ver = "1.0.0"
	}
	verStr := ver
	if !strings.HasPrefix(verStr, "v") && !strings.HasPrefix(verStr, "/") {
		verStr = "v" + verStr
	}
	prefix := "/api/" + strings.Trim(verStr, "/") + "/" + wfName

	for i := range b.API {
		r := strings.TrimSpace(b.API[i].Route)
		if r != "" && !strings.HasPrefix(r, "/api/") && !strings.HasPrefix(r, prefix) {
			b.API[i].Route = prefix + "/" + strings.TrimLeft(r, "/")
		}
	}
}

type MiddlewareExceptionConfig struct {
	Exception string            `json:"exception" yaml:"exception"`
	Code      int               `json:"code,omitempty" yaml:"code,omitempty"`
	Message   string            `json:"message,omitempty" yaml:"message,omitempty"`
	Activity  string            `json:"activity,omitempty" yaml:"activity,omitempty"`
	IsAsync   bool              `json:"is_async,omitempty" yaml:"is_async,omitempty"`
	Async     bool              `json:"async,omitempty" yaml:"async,omitempty"`
	Headers   map[string]string `json:"headers,omitempty" yaml:"headers,omitempty"`
}

func (m *MiddlewareExceptionConfig) GetIsAsync() bool {
	if m == nil {
		return false
	}
	return m.IsAsync || m.Async
}

type MiddlewareConfig struct {
	Name       string                      `json:"name" yaml:"name"`
	Type       string                      `json:"type,omitempty" yaml:"type,omitempty"`
	Activity   string                      `json:"activity,omitempty" yaml:"activity,omitempty"`
	IsAsync    bool                        `json:"is_async,omitempty" yaml:"is_async,omitempty"`
	Excludes   []string                    `json:"excludes,omitempty" yaml:"excludes,omitempty"`
	Includes   []string                    `json:"includes,omitempty" yaml:"includes,omitempty"`
	Exceptions []MiddlewareExceptionConfig `json:"exceptions,omitempty" yaml:"exceptions,omitempty"`
	Config     map[string]interface{}      `json:"config,omitempty" yaml:"config,omitempty"`
}

func (m *MiddlewareConfig) UnmarshalYAML(unmarshal func(interface{}) error) error {
	var raw map[string]interface{}
	if err := unmarshal(&raw); err != nil {
		var strName string
		if errStr := unmarshal(&strName); errStr == nil {
			m.Name = strName
			m.Type = "pre-request"
			return nil
		}
		return err
	}

	m.Type = "pre-request"
	if name, ok := raw["name"].(string); ok {
		m.Name = name
	}
	if t, ok := raw["type"].(string); ok {
		if strings.EqualFold(t, "pre-request") || strings.EqualFold(t, "post-request") {
			m.Type = strings.ToLower(t)
		} else if m.Name == "" {
			m.Name = t
		}
	}

	if act, ok := raw["activity"].(string); ok {
		m.Activity = act
	}
	if isAsync, ok := raw["async"].(bool); ok {
		m.IsAsync = isAsync
	} else if isAsync, ok := raw["is_async"].(bool); ok {
		m.IsAsync = isAsync
	}

	if exc, ok := raw["excludes"]; ok {
		m.Excludes = toStringSlice(exc)
	} else if exc, ok := raw["exclude"]; ok {
		m.Excludes = toStringSlice(exc)
	}

	if inc, ok := raw["includes"]; ok {
		m.Includes = toStringSlice(inc)
	} else if inc, ok := raw["include"]; ok {
		m.Includes = toStringSlice(inc)
	}

	if rawExc, ok := raw["exceptions"]; ok {
		m.Exceptions = parseMiddlewareExceptions(rawExc)
	}

	cfg := make(map[string]interface{})
	if nestedCfg, ok := raw["config"].(map[string]interface{}); ok {
		for k, v := range nestedCfg {
			cfg[k] = v
		}
	}
	reserved := map[string]bool{
		"name": true, "type": true, "excludes": true, "exclude": true,
		"includes": true, "include": true, "exceptions": true, "config": true,
	}
	for k, v := range raw {
		if !reserved[k] {
			cfg[k] = v
		}
	}
	m.Config = cfg
	return nil
}

func toStringSlice(val interface{}) []string {
	if val == nil {
		return nil
	}
	if s, ok := val.(string); ok {
		return []string{s}
	}
	if list, ok := val.([]interface{}); ok {
		res := make([]string, 0, len(list))
		for _, item := range list {
			if s, ok := item.(string); ok {
				res = append(res, s)
			}
		}
		return res
	}
	return nil
}

func parseMiddlewareExceptions(val interface{}) []MiddlewareExceptionConfig {
	var result []MiddlewareExceptionConfig
	if list, ok := val.([]interface{}); ok {
		for _, item := range list {
			if m, ok := item.(map[string]interface{}); ok {
				exc := MiddlewareExceptionConfig{
					Headers: make(map[string]string),
				}
				if e, ok := m["exception"].(string); ok {
					exc.Exception = e
				} else if e, ok := m["name"].(string); ok {
					exc.Exception = e
				}
				if code, ok := m["code"].(int); ok {
					exc.Code = code
				} else if code, ok := m["status_code"].(int); ok {
					exc.Code = code
				}
				if msg, ok := m["message"].(string); ok {
					exc.Message = msg
				} else if msg, ok := m["detail"].(string); ok {
					exc.Message = msg
				}
				if act, ok := m["activity"].(string); ok {
					exc.Activity = act
				} else if act, ok := m["handler_activity"].(string); ok {
					exc.Activity = act
				}
				if asyncVal, ok := m["async"].(bool); ok {
					exc.IsAsync = asyncVal
				} else if asyncVal, ok := m["is_async"].(bool); ok {
					exc.IsAsync = asyncVal
				} else if asyncVal, ok := m["fire_and_forget"].(bool); ok {
					exc.IsAsync = asyncVal
				}
				if hdrs, ok := m["headers"].(map[string]interface{}); ok {
					for hk, hv := range hdrs {
						if hvs, ok := hv.(string); ok {
							exc.Headers[hk] = hvs
						}
					}
				}
				result = append(result, exc)
			}
		}
	}
	return result
}


type MaskingConfig struct {
	Enabled   bool     `json:"enabled" yaml:"enabled"`
	Fields    []string `json:"fields" yaml:"fields"`
	MaskValue string   `json:"mask_value,omitempty" yaml:"mask_value,omitempty"`
}

func (m *MaskingConfig) UnmarshalYAML(unmarshal func(interface{}) error) error {
	var list []string
	if err := unmarshal(&list); err == nil {
		m.Enabled = true
		m.Fields = list
		m.MaskValue = "********"
		return nil
	}
	var raw struct {
		Enabled    *bool    `yaml:"enabled"`
		Fields     []string `yaml:"fields"`
		MaskFields []string `yaml:"mask_fields"`
		MaskValue  string   `yaml:"mask_value"`
		MaskVal    string   `yaml:"mask"`
	}
	if err := unmarshal(&raw); err == nil {
		if raw.Enabled != nil {
			m.Enabled = *raw.Enabled
		} else {
			m.Enabled = true
		}
		if len(raw.Fields) > 0 {
			m.Fields = raw.Fields
		} else if len(raw.MaskFields) > 0 {
			m.Fields = raw.MaskFields
		}
		if raw.MaskValue != "" {
			m.MaskValue = raw.MaskValue
		} else if raw.MaskVal != "" {
			m.MaskValue = raw.MaskVal
		} else {
			m.MaskValue = "********"
		}
		return nil
	}
	return nil
}

type TelemetryConfig struct {
	MaskFields []string       `json:"mask_fields,omitempty" yaml:"mask_fields,omitempty"`
	Masking    *MaskingConfig `json:"masking,omitempty" yaml:"masking,omitempty"`
}

func (t *TelemetryConfig) UnmarshalYAML(unmarshal func(interface{}) error) error {
	var raw struct {
		MaskFields []string       `yaml:"mask_fields"`
		Masking    *MaskingConfig `yaml:"masking"`
	}
	if err := unmarshal(&raw); err == nil {
		t.MaskFields = raw.MaskFields
		t.Masking = raw.Masking
		return nil
	}
	return nil
}

type GlobalExceptionHandlerConfig struct {
	Activity          string            `json:"activity,omitempty" yaml:"activity,omitempty"`
	Code              int               `json:"code,omitempty" yaml:"code,omitempty"`
	Message           string            `json:"message,omitempty" yaml:"message,omitempty"`
	IsAsync           bool              `json:"is_async,omitempty" yaml:"is_async,omitempty"`
	Async             bool              `json:"async,omitempty" yaml:"async,omitempty"`
	IncludeStackTrace bool              `json:"include_stack_trace,omitempty" yaml:"include_stack_trace,omitempty"`
	Headers           map[string]string `json:"headers,omitempty" yaml:"headers,omitempty"`
}

func (g *GlobalExceptionHandlerConfig) GetIsAsync() bool {
	if g == nil {
		return false
	}
	return g.IsAsync || g.Async
}

type ErrorHandlingConfig struct {
	GlobalHandler *GlobalExceptionHandlerConfig `json:"global_handler,omitempty" yaml:"global_handler,omitempty"`
}

type WorkflowBlueprint struct {
	WorkflowName           string                        `json:"workflow_name" yaml:"workflow_name"`
	Version                string                        `json:"version" yaml:"version"`
	Namespace              string                        `json:"namespace,omitempty" yaml:"namespace,omitempty"`
	Description            string                        `json:"description,omitempty" yaml:"description,omitempty"`
	Mode                   string                        `json:"mode,omitempty" yaml:"mode,omitempty"`
	CompensationMode       string                        `json:"compensation_mode,omitempty" yaml:"compensation_mode,omitempty"`
	AllowedInitiatorRoles  []string                      `json:"allowed_initiator_roles,omitempty" yaml:"allowed_initiator_roles,omitempty"`
	Middlewares            []MiddlewareConfig            `json:"middlewares,omitempty" yaml:"middlewares,omitempty"`
	Steps                  []WorkflowStep                `json:"steps" yaml:"steps"`
	API                    APIEndpoints                  `json:"api,omitempty" yaml:"api,omitempty"`
	Persistence            map[string]interface{}        `json:"persistence,omitempty" yaml:"persistence,omitempty"`
	Metadata               map[string]interface{}        `json:"metadata,omitempty" yaml:"metadata,omitempty"`
	Masking                *MaskingConfig                `json:"masking,omitempty" yaml:"masking,omitempty"`
	Telemetry              *TelemetryConfig              `json:"telemetry,omitempty" yaml:"telemetry,omitempty"`
	Redaction              *MaskingConfig                `json:"redaction,omitempty" yaml:"redaction,omitempty"`
	ErrorHandling          *ErrorHandlingConfig          `json:"error_handling,omitempty" yaml:"error_handling,omitempty"`
	GlobalExceptionHandler *GlobalExceptionHandlerConfig `json:"global_exception_handler,omitempty" yaml:"global_exception_handler,omitempty"`
}

func (b *WorkflowBlueprint) GetMaskConfig(step *WorkflowStep) ([]string, string) {
	maskValue := "********"
	var fields []string
	seen := make(map[string]bool)

	addFields := func(fList []string) {
		for _, f := range fList {
			trimmed := strings.TrimSpace(f)
			if trimmed != "" && !seen[strings.ToLower(trimmed)] {
				seen[strings.ToLower(trimmed)] = true
				fields = append(fields, trimmed)
			}
		}
	}

	if b != nil {
		// 1. Blueprint Top-Level Masking
		if b.Masking != nil && b.Masking.Enabled {
			if b.Masking.MaskValue != "" {
				maskValue = b.Masking.MaskValue
			}
			addFields(b.Masking.Fields)
		}

		// 2. Blueprint Telemetry Masking / mask_fields
		if b.Telemetry != nil {
			if b.Telemetry.Masking != nil && b.Telemetry.Masking.Enabled {
				if b.Telemetry.Masking.MaskValue != "" {
					maskValue = b.Telemetry.Masking.MaskValue
				}
				addFields(b.Telemetry.Masking.Fields)
			}
			if len(b.Telemetry.MaskFields) > 0 {
				addFields(b.Telemetry.MaskFields)
			}
		}

		// 3. Blueprint Redaction
		if b.Redaction != nil && b.Redaction.Enabled {
			if b.Redaction.MaskValue != "" {
				maskValue = b.Redaction.MaskValue
			}
			addFields(b.Redaction.Fields)
		}
	}

	// 4. Step-level Masking (if provided)
	if step != nil {
		if step.Masking != nil && step.Masking.Enabled {
			if step.Masking.MaskValue != "" {
				maskValue = step.Masking.MaskValue
			}
			addFields(step.Masking.Fields)
		}
		if len(step.MaskFields) > 0 {
			addFields(step.MaskFields)
		}
	}

	return fields, maskValue
}

type StepExecutionTrace struct {
	StepID        string                 `json:"step_id"`
	ActivityName  string                 `json:"activity_name,omitempty"`
	Status        string                 `json:"status"`
	IsAsync       bool                   `json:"is_async,omitempty"`
	InputPayload  map[string]interface{} `json:"input_payload,omitempty"`
	OutputPayload map[string]interface{} `json:"output_payload,omitempty"`
	ErrorMessage  string                 `json:"error_message,omitempty"`
	StartedAt     string                 `json:"started_at"`
	FinishedAt    string                 `json:"finished_at"`
	DurationMs    float64                `json:"duration_ms"`
}

type ExecutionState struct {
	JobID         string                 `json:"job_id"`
	WorkflowName  string                 `json:"workflow_name"`
	Status        ExecutionStatus        `json:"status"`
	Route         string                 `json:"route,omitempty"`
	Method        string                 `json:"method,omitempty"`
	CurrentStepID string                 `json:"current_step_id,omitempty"`
	CurrentIndex  int                    `json:"current_index"`
	TotalSteps    int                    `json:"total_steps"`
	Data          map[string]interface{} `json:"data"`
	OutputData    map[string]interface{} `json:"output_data,omitempty"`
	Traces        []StepExecutionTrace   `json:"traces"`
	StartedAt     string                 `json:"started_at"`
	FinishedAt    *string                `json:"finished_at,omitempty"`
	ClientHost    string                 `json:"client_host,omitempty"`
	DurationMs    float64                `json:"duration_ms,omitempty"`
	Error         *string                `json:"error,omitempty"`
	ErrorCode     *int                   `json:"error_code,omitempty"`
	ErrorType     *string                `json:"error_type,omitempty"`
}

type LiveEvent struct {
	Event string      `json:"event"`
	Data  interface{} `json:"data"`
}

func NowUTC() string {
	return time.Now().UTC().Format(time.RFC3339Nano)
}
