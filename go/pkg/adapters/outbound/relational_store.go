package outbound

import (
	"context"
	"database/sql"
	"encoding/json"
	"fmt"
	"strings"
	"sync"
	"time"

	"github.com/martinkatamba/octopus_live_engine/go/pkg/domain/models"
)

// SQLDialect defines the SQL statement generator for diverse relational databases.
type SQLDialect interface {
	Name() string
	Placeholder(index int) string
	DDLExecutionStates() string
	DDLStepTraces() string
	UpsertExecutionStateSQL() string
	InsertStepTraceSQL() string
	SelectStateSQL() string
	SelectTracesSQL() string
	ListStatesSQL(hasWorkflow bool) string
	DeleteTracesSQL() string
}

// ============================================================================
// PostgreSQL Dialect
// ============================================================================

type PostgresDialect struct{}

func (d *PostgresDialect) Name() string { return "postgres" }
func (d *PostgresDialect) Placeholder(index int) string {
	return fmt.Sprintf("$%d", index)
}

func (d *PostgresDialect) DDLExecutionStates() string {
	return `
		CREATE TABLE IF NOT EXISTS execution_states (
			job_id VARCHAR(255) PRIMARY KEY,
			workflow_name VARCHAR(255) NOT NULL,
			status VARCHAR(64) NOT NULL,
			route VARCHAR(512),
			method VARCHAR(32),
			current_step_id VARCHAR(255),
			current_index INT,
			total_steps INT,
			data TEXT,
			output_data TEXT,
			error TEXT,
			started_at VARCHAR(64),
			finished_at VARCHAR(64),
			client_host VARCHAR(255),
			duration_ms DOUBLE PRECISION,
			error_code INT,
			error_type VARCHAR(255)
		);
		CREATE INDEX IF NOT EXISTS idx_exec_states_wf_started ON execution_states(workflow_name, started_at DESC);
	`
}

func (d *PostgresDialect) DDLStepTraces() string {
	return `
		CREATE TABLE IF NOT EXISTS step_traces (
			id BIGSERIAL PRIMARY KEY,
			job_id VARCHAR(255) NOT NULL REFERENCES execution_states(job_id) ON DELETE CASCADE,
			step_id VARCHAR(255) NOT NULL,
			activity_name VARCHAR(255),
			status VARCHAR(64) NOT NULL,
			input_payload TEXT,
			output_payload TEXT,
			error_message TEXT,
			started_at VARCHAR(64),
			finished_at VARCHAR(64),
			duration_ms DOUBLE PRECISION
		);
		CREATE INDEX IF NOT EXISTS idx_step_traces_job_id ON step_traces(job_id);
	`
}

func (d *PostgresDialect) UpsertExecutionStateSQL() string {
	return `
		INSERT INTO execution_states (
			job_id, workflow_name, status, route, method,
			current_step_id, current_index, total_steps,
			data, output_data, error, started_at, finished_at,
			client_host, duration_ms, error_code, error_type
		) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)
		ON CONFLICT (job_id) DO UPDATE SET
			status = EXCLUDED.status,
			route = EXCLUDED.route,
			method = EXCLUDED.method,
			current_step_id = EXCLUDED.current_step_id,
			current_index = EXCLUDED.current_index,
			total_steps = EXCLUDED.total_steps,
			data = EXCLUDED.data,
			output_data = EXCLUDED.output_data,
			error = EXCLUDED.error,
			finished_at = EXCLUDED.finished_at,
			client_host = EXCLUDED.client_host,
			duration_ms = EXCLUDED.duration_ms,
			error_code = EXCLUDED.error_code,
			error_type = EXCLUDED.error_type
	`
}

func (d *PostgresDialect) InsertStepTraceSQL() string {
	return `
		INSERT INTO step_traces (
			job_id, step_id, activity_name, status,
			input_payload, output_payload, error_message,
			started_at, finished_at, duration_ms
		) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
	`
}

func (d *PostgresDialect) SelectStateSQL() string {
	return `
		SELECT job_id, workflow_name, status, route, method,
		       current_step_id, current_index, total_steps,
		       data, output_data, error, started_at, finished_at,
		       client_host, duration_ms, error_code, error_type
		FROM execution_states WHERE job_id = $1
	`
}

func (d *PostgresDialect) SelectTracesSQL() string {
	return `
		SELECT step_id, activity_name, status, input_payload,
		       output_payload, error_message, started_at, finished_at, duration_ms
		FROM step_traces WHERE job_id = $1 ORDER BY id ASC
	`
}

func (d *PostgresDialect) ListStatesSQL(hasWorkflow bool) string {
	if hasWorkflow {
		return "SELECT job_id FROM execution_states WHERE workflow_name = $1 ORDER BY started_at DESC LIMIT $2"
	}
	return "SELECT job_id FROM execution_states ORDER BY started_at DESC LIMIT $1"
}

func (d *PostgresDialect) DeleteTracesSQL() string {
	return "DELETE FROM step_traces WHERE job_id = $1"
}

// ============================================================================
// MySQL Dialect
// ============================================================================

type MySqlDialect struct{}

func (d *MySqlDialect) Name() string                 { return "mysql" }
func (d *MySqlDialect) Placeholder(index int) string { return "?" }

func (d *MySqlDialect) DDLExecutionStates() string {
	return `
		CREATE TABLE IF NOT EXISTS execution_states (
			job_id VARCHAR(255) PRIMARY KEY,
			workflow_name VARCHAR(255) NOT NULL,
			status VARCHAR(64) NOT NULL,
			route VARCHAR(512),
			method VARCHAR(32),
			current_step_id VARCHAR(255),
			current_index INT,
			total_steps INT,
			data LONGTEXT,
			output_data LONGTEXT,
			error TEXT,
			started_at VARCHAR(64),
			finished_at VARCHAR(64),
			client_host VARCHAR(255),
			duration_ms DOUBLE,
			error_code INT,
			error_type VARCHAR(255),
			INDEX idx_exec_states_wf_started (workflow_name, started_at DESC)
		) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;
	`
}

func (d *MySqlDialect) DDLStepTraces() string {
	return `
		CREATE TABLE IF NOT EXISTS step_traces (
			id BIGINT AUTO_INCREMENT PRIMARY KEY,
			job_id VARCHAR(255) NOT NULL,
			step_id VARCHAR(255) NOT NULL,
			activity_name VARCHAR(255),
			status VARCHAR(64) NOT NULL,
			input_payload LONGTEXT,
			output_payload LONGTEXT,
			error_message TEXT,
			started_at VARCHAR(64),
			finished_at VARCHAR(64),
			duration_ms DOUBLE,
			INDEX idx_step_traces_job_id (job_id),
			FOREIGN KEY (job_id) REFERENCES execution_states(job_id) ON DELETE CASCADE
		) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;
	`
}

func (d *MySqlDialect) UpsertExecutionStateSQL() string {
	return `
		INSERT INTO execution_states (
			job_id, workflow_name, status, route, method,
			current_step_id, current_index, total_steps,
			data, output_data, error, started_at, finished_at,
			client_host, duration_ms, error_code, error_type
		) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
		ON DUPLICATE KEY UPDATE
			status = VALUES(status),
			route = VALUES(route),
			method = VALUES(method),
			current_step_id = VALUES(current_step_id),
			current_index = VALUES(current_index),
			total_steps = VALUES(total_steps),
			data = VALUES(data),
			output_data = VALUES(output_data),
			error = VALUES(error),
			finished_at = VALUES(finished_at),
			client_host = VALUES(client_host),
			duration_ms = VALUES(duration_ms),
			error_code = VALUES(error_code),
			error_type = VALUES(error_type)
	`
}

func (d *MySqlDialect) InsertStepTraceSQL() string {
	return `
		INSERT INTO step_traces (
			job_id, step_id, activity_name, status,
			input_payload, output_payload, error_message,
			started_at, finished_at, duration_ms
		) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
	`
}

func (d *MySqlDialect) SelectStateSQL() string {
	return `
		SELECT job_id, workflow_name, status, route, method,
		       current_step_id, current_index, total_steps,
		       data, output_data, error, started_at, finished_at,
		       client_host, duration_ms, error_code, error_type
		FROM execution_states WHERE job_id = ?
	`
}

func (d *MySqlDialect) SelectTracesSQL() string {
	return `
		SELECT step_id, activity_name, status, input_payload,
		       output_payload, error_message, started_at, finished_at, duration_ms
		FROM step_traces WHERE job_id = ? ORDER BY id ASC
	`
}

func (d *MySqlDialect) ListStatesSQL(hasWorkflow bool) string {
	if hasWorkflow {
		return "SELECT job_id FROM execution_states WHERE workflow_name = ? ORDER BY started_at DESC LIMIT ?"
	}
	return "SELECT job_id FROM execution_states ORDER BY started_at DESC LIMIT ?"
}

func (d *MySqlDialect) DeleteTracesSQL() string {
	return "DELETE FROM step_traces WHERE job_id = ?"
}

// ============================================================================
// SQL Server (T-SQL) Dialect
// ============================================================================

type SqlServerDialect struct{}

func (d *SqlServerDialect) Name() string                 { return "sqlserver" }
func (d *SqlServerDialect) Placeholder(index int) string { return "?" }

func (d *SqlServerDialect) DDLExecutionStates() string {
	return `
		IF NOT EXISTS (SELECT * FROM sysobjects WHERE name='execution_states' and xtype='U')
		BEGIN
			CREATE TABLE execution_states (
				job_id NVARCHAR(255) PRIMARY KEY,
				workflow_name NVARCHAR(255) NOT NULL,
				status NVARCHAR(64) NOT NULL,
				route NVARCHAR(512),
				method NVARCHAR(32),
				current_step_id NVARCHAR(255),
				current_index INT,
				total_steps INT,
				data NVARCHAR(MAX),
				output_data NVARCHAR(MAX),
				error NVARCHAR(MAX),
				started_at NVARCHAR(64),
				finished_at NVARCHAR(64),
				client_host NVARCHAR(255),
				duration_ms FLOAT,
				error_code INT,
				error_type NVARCHAR(255)
			);
			CREATE INDEX idx_exec_states_wf_started ON execution_states(workflow_name, started_at DESC);
		END
	`
}

func (d *SqlServerDialect) DDLStepTraces() string {
	return `
		IF NOT EXISTS (SELECT * FROM sysobjects WHERE name='step_traces' and xtype='U')
		BEGIN
			CREATE TABLE step_traces (
				id BIGINT IDENTITY(1,1) PRIMARY KEY,
				job_id NVARCHAR(255) NOT NULL FOREIGN KEY REFERENCES execution_states(job_id) ON DELETE CASCADE,
				step_id NVARCHAR(255) NOT NULL,
				activity_name NVARCHAR(255),
				status NVARCHAR(64) NOT NULL,
				input_payload NVARCHAR(MAX),
				output_payload NVARCHAR(MAX),
				error_message NVARCHAR(MAX),
				started_at NVARCHAR(64),
				finished_at NVARCHAR(64),
				duration_ms FLOAT
			);
			CREATE INDEX idx_step_traces_job_id ON step_traces(job_id);
		END
	`
}

func (d *SqlServerDialect) UpsertExecutionStateSQL() string {
	return `
		MERGE INTO execution_states AS target
		USING (
			SELECT ? AS job_id, ? AS workflow_name, ? AS status, ? AS route, ? AS method,
			       ? AS current_step_id, ? AS current_index, ? AS total_steps,
			       ? AS data, ? AS output_data, ? AS error, ? AS started_at, ? AS finished_at,
			       ? AS client_host, ? AS duration_ms, ? AS error_code, ? AS error_type
		) AS source
		ON target.job_id = source.job_id
		WHEN MATCHED THEN
			UPDATE SET
				status = source.status,
				route = source.route,
				method = source.method,
				current_step_id = source.current_step_id,
				current_index = source.current_index,
				total_steps = source.total_steps,
				data = source.data,
				output_data = source.output_data,
				error = source.error,
				finished_at = source.finished_at,
				client_host = source.client_host,
				duration_ms = source.duration_ms,
				error_code = source.error_code,
				error_type = source.error_type
		WHEN NOT MATCHED THEN
			INSERT (
				job_id, workflow_name, status, route, method,
				current_step_id, current_index, total_steps,
				data, output_data, error, started_at, finished_at,
				client_host, duration_ms, error_code, error_type
			) VALUES (
				source.job_id, source.workflow_name, source.status, source.route, source.method,
				source.current_step_id, source.current_index, source.total_steps,
				source.data, source.output_data, source.error, source.started_at, source.finished_at,
				source.client_host, source.duration_ms, source.error_code, source.error_type
			);
	`
}

func (d *SqlServerDialect) InsertStepTraceSQL() string {
	return `
		INSERT INTO step_traces (
			job_id, step_id, activity_name, status,
			input_payload, output_payload, error_message,
			started_at, finished_at, duration_ms
		) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
	`
}

func (d *SqlServerDialect) SelectStateSQL() string {
	return `
		SELECT job_id, workflow_name, status, route, method,
		       current_step_id, current_index, total_steps,
		       data, output_data, error, started_at, finished_at,
		       client_host, duration_ms, error_code, error_type
		FROM execution_states WHERE job_id = ?
	`
}

func (d *SqlServerDialect) SelectTracesSQL() string {
	return `
		SELECT step_id, activity_name, status, input_payload,
		       output_payload, error_message, started_at, finished_at, duration_ms
		FROM step_traces WHERE job_id = ? ORDER BY id ASC
	`
}

func (d *SqlServerDialect) ListStatesSQL(hasWorkflow bool) string {
	if hasWorkflow {
		return "SELECT TOP (?) job_id FROM execution_states WHERE workflow_name = ? ORDER BY started_at DESC"
	}
	return "SELECT TOP (?) job_id FROM execution_states ORDER BY started_at DESC"
}

func (d *SqlServerDialect) DeleteTracesSQL() string {
	return "DELETE FROM step_traces WHERE job_id = ?"
}

// ============================================================================
// SQLite Dialect
// ============================================================================

type SqliteDialect struct{}

func (d *SqliteDialect) Name() string                 { return "sqlite" }
func (d *SqliteDialect) Placeholder(index int) string { return "?" }

func (d *SqliteDialect) DDLExecutionStates() string {
	return `
		CREATE TABLE IF NOT EXISTS execution_states (
			job_id TEXT PRIMARY KEY,
			workflow_name TEXT NOT NULL,
			status TEXT NOT NULL,
			route TEXT,
			method TEXT,
			current_step_id TEXT,
			current_index INTEGER,
			total_steps INTEGER,
			data TEXT,
			output_data TEXT,
			error TEXT,
			started_at TEXT,
			finished_at TEXT,
			client_host TEXT,
			duration_ms REAL,
			error_code INTEGER,
			error_type TEXT
		);
	`
}

func (d *SqliteDialect) DDLStepTraces() string {
	return `
		CREATE TABLE IF NOT EXISTS step_traces (
			id INTEGER PRIMARY KEY AUTOINCREMENT,
			job_id TEXT NOT NULL,
			step_id TEXT NOT NULL,
			activity_name TEXT,
			status TEXT NOT NULL,
			input_payload TEXT,
			output_payload TEXT,
			error_message TEXT,
			started_at TEXT,
			finished_at TEXT,
			duration_ms REAL,
			FOREIGN KEY (job_id) REFERENCES execution_states(job_id)
		);
	`
}

func (d *SqliteDialect) UpsertExecutionStateSQL() string {
	return `
		INSERT INTO execution_states (
			job_id, workflow_name, status, route, method,
			current_step_id, current_index, total_steps,
			data, output_data, error, started_at, finished_at,
			client_host, duration_ms, error_code, error_type
		) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
		ON CONFLICT(job_id) DO UPDATE SET
			status=excluded.status,
			route=excluded.route,
			method=excluded.method,
			current_step_id=excluded.current_step_id,
			current_index=excluded.current_index,
			total_steps=excluded.total_steps,
			data=excluded.data,
			output_data=excluded.output_data,
			error=excluded.error,
			finished_at=excluded.finished_at,
			client_host=excluded.client_host,
			duration_ms=excluded.duration_ms,
			error_code=excluded.error_code,
			error_type=excluded.error_type
	`
}

func (d *SqliteDialect) InsertStepTraceSQL() string {
	return `
		INSERT INTO step_traces (
			job_id, step_id, activity_name, status,
			input_payload, output_payload, error_message,
			started_at, finished_at, duration_ms
		) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
	`
}

func (d *SqliteDialect) SelectStateSQL() string {
	return `
		SELECT job_id, workflow_name, status, route, method,
		       current_step_id, current_index, total_steps,
		       data, output_data, error, started_at, finished_at,
		       client_host, duration_ms, error_code, error_type
		FROM execution_states WHERE job_id = ?
	`
}

func (d *SqliteDialect) SelectTracesSQL() string {
	return `
		SELECT step_id, activity_name, status, input_payload,
		       output_payload, error_message, started_at, finished_at, duration_ms
		FROM step_traces WHERE job_id = ? ORDER BY id ASC
	`
}

func (d *SqliteDialect) ListStatesSQL(hasWorkflow bool) string {
	if hasWorkflow {
		return "SELECT job_id FROM execution_states WHERE workflow_name = ? ORDER BY started_at DESC LIMIT ?"
	}
	return "SELECT job_id FROM execution_states ORDER BY started_at DESC LIMIT ?"
}

func (d *SqliteDialect) DeleteTracesSQL() string {
	return "DELETE FROM step_traces WHERE job_id = ?"
}

// ============================================================================
// Relational State Store Implementation
// ============================================================================

type RelationalStateStore struct {
	db          *sql.DB
	dialect     SQLDialect
	subscribers map[string][]chan models.LiveEvent
	mu          sync.RWMutex
	fallback    *InMemoryStateStore
}

func NewRelationalStateStore(db *sql.DB, dialect SQLDialect) *RelationalStateStore {
	if dialect == nil {
		dialect = &SqliteDialect{}
	}
	store := &RelationalStateStore{
		db:          db,
		dialect:     dialect,
		subscribers: make(map[string][]chan models.LiveEvent),
		fallback:    NewInMemoryStateStore(),
	}
	if db != nil {
		store.initDB()
	}
	return store
}

func (s *RelationalStateStore) Dialect() SQLDialect {
	return s.dialect
}

func (s *RelationalStateStore) initDB() {
	if s.db == nil {
		return
	}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	// Execute tables creation DDL
	_ = s.execDDL(ctx, s.dialect.DDLExecutionStates())
	_ = s.execDDL(ctx, s.dialect.DDLStepTraces())
}

func (s *RelationalStateStore) execDDL(ctx context.Context, ddl string) error {
	queries := strings.Split(ddl, ";")
	for _, q := range queries {
		q = strings.TrimSpace(q)
		if q != "" {
			_, err := s.db.ExecContext(ctx, q)
			if err != nil {
				return err
			}
		}
	}
	return nil
}

func (s *RelationalStateStore) SaveState(ctx context.Context, state *models.ExecutionState) error {
	if state == nil {
		return nil
	}

	// Always update in-memory cache / fallback
	_ = s.fallback.SaveState(ctx, state)

	if s.db == nil {
		return nil
	}

	dataJSON, _ := json.Marshal(state.Data)
	var outputJSON []byte
	if state.OutputData != nil {
		outputJSON, _ = json.Marshal(state.OutputData)
	}

	upsertSQL := s.dialect.UpsertExecutionStateSQL()
	_, err := s.db.ExecContext(ctx, upsertSQL,
		state.JobID,
		state.WorkflowName,
		string(state.Status),
		state.Route,
		state.Method,
		state.CurrentStepID,
		state.CurrentIndex,
		state.TotalSteps,
		string(dataJSON),
		string(outputJSON),
		state.Error,
		state.StartedAt,
		state.FinishedAt,
		state.ClientHost,
		state.DurationMs,
		state.ErrorCode,
		state.ErrorType,
	)
	if err != nil {
		fmt.Printf("⚠️ [RELATIONAL STORE ERROR] SaveState failed: %v\n", err)
		return err
	}

	// Delete and insert traces
	_, _ = s.db.ExecContext(ctx, s.dialect.DeleteTracesSQL(), state.JobID)
	insertTraceSQL := s.dialect.InsertStepTraceSQL()

	for _, tr := range state.Traces {
		inJSON, _ := json.Marshal(tr.InputPayload)
		outJSON, _ := json.Marshal(tr.OutputPayload)

		_, trErr := s.db.ExecContext(ctx, insertTraceSQL,
			state.JobID,
			tr.StepID,
			tr.ActivityName,
			tr.Status,
			string(inJSON),
			string(outJSON),
			tr.ErrorMessage,
			tr.StartedAt,
			tr.FinishedAt,
			tr.DurationMs,
		)
		if trErr != nil {
			fmt.Printf("⚠️ [RELATIONAL STORE ERROR] InsertStepTrace failed: %v\n", trErr)
		}
	}

	return nil
}

func (s *RelationalStateStore) GetState(ctx context.Context, jobID string) (*models.ExecutionState, error) {
	if s.db == nil {
		return s.fallback.GetState(ctx, jobID)
	}

	var st models.ExecutionState
	var dataStr, outputStr, statusStr sql.NullString
	var errorStr, startedStr, finishedStr, hostStr, errTypeStr sql.NullString
	var routeStr, methodStr, stepIDStr sql.NullString
	var currIdx, totalSteps, errCode sql.NullInt64
	var durationMs sql.NullFloat64

	row := s.db.QueryRowContext(ctx, s.dialect.SelectStateSQL(), jobID)
	err := row.Scan(
		&st.JobID, &st.WorkflowName, &statusStr, &routeStr, &methodStr,
		&stepIDStr, &currIdx, &totalSteps, &dataStr, &outputStr, &errorStr,
		&startedStr, &finishedStr, &hostStr, &durationMs, &errCode, &errTypeStr,
	)
	if err != nil {
		// Fallback to in-memory lookup
		return s.fallback.GetState(ctx, jobID)
	}

	st.Status = models.ExecutionStatus(statusStr.String)
	st.Route = routeStr.String
	st.Method = methodStr.String
	st.CurrentStepID = stepIDStr.String
	st.CurrentIndex = int(currIdx.Int64)
	st.TotalSteps = int(totalSteps.Int64)
	if errorStr.Valid && errorStr.String != "" {
		st.Error = &errorStr.String
	}
	st.StartedAt = startedStr.String
	if finishedStr.Valid && finishedStr.String != "" {
		st.FinishedAt = &finishedStr.String
	}
	st.ClientHost = hostStr.String
	st.DurationMs = durationMs.Float64
	if errCode.Valid {
		ec := int(errCode.Int64)
		st.ErrorCode = &ec
	}
	if errTypeStr.Valid && errTypeStr.String != "" {
		st.ErrorType = &errTypeStr.String
	}

	if dataStr.Valid && dataStr.String != "" {
		_ = json.Unmarshal([]byte(dataStr.String), &st.Data)
	}
	if outputStr.Valid && outputStr.String != "" {
		_ = json.Unmarshal([]byte(outputStr.String), &st.OutputData)
	}

	// Fetch traces
	traceRows, err := s.db.QueryContext(ctx, s.dialect.SelectTracesSQL(), jobID)
	if err == nil {
		defer traceRows.Close()
		var traces []models.StepExecutionTrace
		for traceRows.Next() {
			var tr models.StepExecutionTrace
			var inP, outP, errM, startT, endT sql.NullString
			var dur sql.NullFloat64

			if err := traceRows.Scan(&tr.StepID, &tr.ActivityName, &tr.Status, &inP, &outP, &errM, &startT, &endT, &dur); err == nil {
				tr.ErrorMessage = errM.String
				tr.StartedAt = startT.String
				tr.FinishedAt = endT.String
				tr.DurationMs = dur.Float64
				if inP.Valid && inP.String != "" {
					_ = json.Unmarshal([]byte(inP.String), &tr.InputPayload)
				}
				if outP.Valid && outP.String != "" {
					_ = json.Unmarshal([]byte(outP.String), &tr.OutputPayload)
				}
				traces = append(traces, tr)
			}
		}
		st.Traces = traces
	}

	return &st, nil
}

func (s *RelationalStateStore) ListStates(ctx context.Context, workflowName string, limit int) ([]*models.ExecutionState, error) {
	if s.db == nil {
		return s.fallback.ListStates(ctx, workflowName, limit)
	}

	if limit <= 0 {
		limit = 100
	}

	var rows *sql.Rows
	var err error
	if workflowName != "" && !strings.EqualFold(workflowName, "ALL") {
		rows, err = s.db.QueryContext(ctx, s.dialect.ListStatesSQL(true), workflowName, limit)
	} else {
		rows, err = s.db.QueryContext(ctx, s.dialect.ListStatesSQL(false), limit)
	}

	if err != nil {
		return s.fallback.ListStates(ctx, workflowName, limit)
	}
	defer rows.Close()

	var jobIDs []string
	for rows.Next() {
		var jid string
		if err := rows.Scan(&jid); err == nil {
			jobIDs = append(jobIDs, jid)
		}
	}

	var result []*models.ExecutionState
	for _, jid := range jobIDs {
		st, err := s.GetState(ctx, jid)
		if err == nil && st != nil {
			result = append(result, st)
		}
	}

	return result, nil
}

func (s *RelationalStateStore) PublishEvent(ctx context.Context, jobID string, event models.LiveEvent) error {
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

func (s *RelationalStateStore) SubscribeEvents(
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
		}
	}

	return ch, cleanup, nil
}
