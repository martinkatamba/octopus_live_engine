using System.Collections.Concurrent;
using System.Data.Common;
using System.Text.Json;
using System.Threading.Channels;
using Octopus.LiveEngine.Domain.Models;
using Octopus.LiveEngine.Domain.Ports;

namespace Octopus.LiveEngine.Adapters.Outbound;

public interface ISqlDialect
{
    string Name { get; }
    string Placeholder(int index);
    string DdlExecutionStates { get; }
    string DdlStepTraces { get; }
    string UpsertExecutionStateSql { get; }
    string InsertStepTraceSql { get; }
    string SelectStateSql { get; }
    string SelectTracesSql { get; }
    string ListStatesSql(bool hasWorkflow);
    string DeleteTracesSql { get; }
}

public class PostgresDialect : ISqlDialect
{
    public string Name => "postgres";
    public string Placeholder(int index) => $"${index}";

    public string DdlExecutionStates => """
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
    """;

    public string DdlStepTraces => """
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
    """;

    public string UpsertExecutionStateSql => """
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
    """;

    public string InsertStepTraceSql => """
        INSERT INTO step_traces (
            job_id, step_id, activity_name, status,
            input_payload, output_payload, error_message,
            started_at, finished_at, duration_ms
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
    """;

    public string SelectStateSql => """
        SELECT job_id, workflow_name, status, route, method,
               current_step_id, current_index, total_steps,
               data, output_data, error, started_at, finished_at,
               client_host, duration_ms, error_code, error_type
        FROM execution_states WHERE job_id = $1
    """;

    public string SelectTracesSql => """
        SELECT step_id, activity_name, status, input_payload,
               output_payload, error_message, started_at, finished_at, duration_ms
        FROM step_traces WHERE job_id = $1 ORDER BY id ASC
    """;

    public string ListStatesSql(bool hasWorkflow) => hasWorkflow
        ? "SELECT job_id FROM execution_states WHERE workflow_name = $1 ORDER BY started_at DESC LIMIT $2"
        : "SELECT job_id FROM execution_states ORDER BY started_at DESC LIMIT $1";

    public string DeleteTracesSql => "DELETE FROM step_traces WHERE job_id = $1";
}

public class MySqlDialect : ISqlDialect
{
    public string Name => "mysql";
    public string Placeholder(int index) => "?";

    public string DdlExecutionStates => """
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
    """;

    public string DdlStepTraces => """
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
    """;

    public string UpsertExecutionStateSql => """
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
    """;

    public string InsertStepTraceSql => """
        INSERT INTO step_traces (
            job_id, step_id, activity_name, status,
            input_payload, output_payload, error_message,
            started_at, finished_at, duration_ms
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    """;

    public string SelectStateSql => """
        SELECT job_id, workflow_name, status, route, method,
               current_step_id, current_index, total_steps,
               data, output_data, error, started_at, finished_at,
               client_host, duration_ms, error_code, error_type
        FROM execution_states WHERE job_id = ?
    """;

    public string SelectTracesSql => """
        SELECT step_id, activity_name, status, input_payload,
               output_payload, error_message, started_at, finished_at, duration_ms
        FROM step_traces WHERE job_id = ? ORDER BY id ASC
    """;

    public string ListStatesSql(bool hasWorkflow) => hasWorkflow
        ? "SELECT job_id FROM execution_states WHERE workflow_name = ? ORDER BY started_at DESC LIMIT ?"
        : "SELECT job_id FROM execution_states ORDER BY started_at DESC LIMIT ?";

    public string DeleteTracesSql => "DELETE FROM step_traces WHERE job_id = ?";
}

public class SqlServerDialect : ISqlDialect
{
    public string Name => "sqlserver";
    public string Placeholder(int index) => "?";

    public string DdlExecutionStates => """
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
    """;

    public string DdlStepTraces => """
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
    """;

    public string UpsertExecutionStateSql => """
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
    """;

    public string InsertStepTraceSql => """
        INSERT INTO step_traces (
            job_id, step_id, activity_name, status,
            input_payload, output_payload, error_message,
            started_at, finished_at, duration_ms
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    """;

    public string SelectStateSql => """
        SELECT job_id, workflow_name, status, route, method,
               current_step_id, current_index, total_steps,
               data, output_data, error, started_at, finished_at,
               client_host, duration_ms, error_code, error_type
        FROM execution_states WHERE job_id = ?
    """;

    public string SelectTracesSql => """
        SELECT step_id, activity_name, status, input_payload,
               output_payload, error_message, started_at, finished_at, duration_ms
        FROM step_traces WHERE job_id = ? ORDER BY id ASC
    """;

    public string ListStatesSql(bool hasWorkflow) => hasWorkflow
        ? "SELECT TOP (?) job_id FROM execution_states WHERE workflow_name = ? ORDER BY started_at DESC"
        : "SELECT TOP (?) job_id FROM execution_states ORDER BY started_at DESC";

    public string DeleteTracesSql => "DELETE FROM step_traces WHERE job_id = ?";
}

public class SqliteDialect : ISqlDialect
{
    public string Name => "sqlite";
    public string Placeholder(int index) => "?";

    public string DdlExecutionStates => """
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
    """;

    public string DdlStepTraces => """
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
    """;

    public string UpsertExecutionStateSql => """
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
    """;

    public string InsertStepTraceSql => """
        INSERT INTO step_traces (
            job_id, step_id, activity_name, status,
            input_payload, output_payload, error_message,
            started_at, finished_at, duration_ms
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    """;

    public string SelectStateSql => """
        SELECT job_id, workflow_name, status, route, method,
               current_step_id, current_index, total_steps,
               data, output_data, error, started_at, finished_at,
               client_host, duration_ms, error_code, error_type
        FROM execution_states WHERE job_id = ?
    """;

    public string SelectTracesSql => """
        SELECT step_id, activity_name, status, input_payload,
               output_payload, error_message, started_at, finished_at, duration_ms
        FROM step_traces WHERE job_id = ? ORDER BY id ASC
    """;

    public string ListStatesSql(bool hasWorkflow) => hasWorkflow
        ? "SELECT job_id FROM execution_states WHERE workflow_name = ? ORDER BY started_at DESC LIMIT ?"
        : "SELECT job_id FROM execution_states ORDER BY started_at DESC LIMIT ?";

    public string DeleteTracesSql => "DELETE FROM step_traces WHERE job_id = ?";
}

public class RelationalSqlStateStore : IStateStore
{
    private readonly ISqlDialect _dialect;
    private readonly string? _connectionString;
    private readonly DbDataSource? _dataSource;
    private readonly InMemoryStateStore _fallback = new();
    private readonly ConcurrentDictionary<string, List<Channel<LiveEvent>>> _subscribers = new();
    private readonly object _lock = new();

    public ISqlDialect Dialect => _dialect;

    public RelationalSqlStateStore(ISqlDialect? dialect = null, string? connectionString = null)
    {
        _dialect = dialect ?? new SqliteDialect();
        _connectionString = connectionString;

        if (!string.IsNullOrWhiteSpace(_connectionString))
        {
            try
            {
                if (_dialect.Name == "postgres")
                {
                    var normalizedCs = NormalizePostgresConnectionString(_connectionString);
                    _dataSource = new Npgsql.NpgsqlDataSourceBuilder(normalizedCs).Build();
                    InitDb();
                }
                else if (_dialect.Name == "mysql")
                {
                    var normalizedCs = NormalizeMySqlConnectionString(_connectionString);
                    _dataSource = new MySqlConnector.MySqlDataSource(normalizedCs);
                    InitDb();
                }
                else if (_dialect.Name == "sqlserver")
                {
                    var normalizedCs = NormalizeSqlServerConnectionString(_connectionString);
                    _dataSource = new SqlDbDataSource(normalizedCs);
                    InitDb();
                }
            }
            catch (Exception ex)
            {
                Console.WriteLine($"⚠️ [{_dialect.Name.ToUpperInvariant()} STATE STORE] Failed to connect: {ex.Message}. Falling back to in-memory store.");
                _dataSource = null;
            }
        }
    }

    public static string NormalizePostgresConnectionString(string cs)
    {
        if (string.IsNullOrWhiteSpace(cs)) return cs;
        if (cs.StartsWith("postgresql://", StringComparison.OrdinalIgnoreCase) ||
            cs.StartsWith("postgres://", StringComparison.OrdinalIgnoreCase))
        {
            try
            {
                var uri = new Uri(cs);
                var userInfo = uri.UserInfo.Split(':');
                var username = userInfo.Length > 0 ? Uri.UnescapeDataString(userInfo[0]) : "";
                var password = userInfo.Length > 1 ? Uri.UnescapeDataString(userInfo[1]) : "";
                var db = uri.AbsolutePath.TrimStart('/');
                var host = uri.Host;
                var port = uri.Port > 0 ? uri.Port : 5432;

                var csb = new Npgsql.NpgsqlConnectionStringBuilder
                {
                    Host = host,
                    Port = port,
                    Database = string.IsNullOrWhiteSpace(db) ? "postgres" : db
                };
                if (!string.IsNullOrEmpty(username)) csb.Username = username;
                if (!string.IsNullOrEmpty(password)) csb.Password = password;

                if (!string.IsNullOrEmpty(uri.Query))
                {
                    var q = uri.Query.TrimStart('?');
                    foreach (var part in q.Split('&', StringSplitOptions.RemoveEmptyEntries))
                    {
                        var kv = part.Split('=');
                        if (kv.Length == 2)
                        {
                            var key = Uri.UnescapeDataString(kv[0]).ToLowerInvariant();
                            var val = Uri.UnescapeDataString(kv[1]).ToLowerInvariant();
                            if (key == "sslmode")
                            {
                                if (val == "disable" || val == "none") csb.SslMode = Npgsql.SslMode.Disable;
                                else if (val == "require") csb.SslMode = Npgsql.SslMode.Require;
                                else if (val == "prefer") csb.SslMode = Npgsql.SslMode.Prefer;
                                else if (val == "allow") csb.SslMode = Npgsql.SslMode.Allow;
                                else if (val == "verify-ca") csb.SslMode = Npgsql.SslMode.VerifyCA;
                                else if (val == "verify-full") csb.SslMode = Npgsql.SslMode.VerifyFull;
                            }
                        }
                    }
                }
                return csb.ConnectionString;
            }
            catch
            {
                return cs;
            }
        }
        return cs;
    }

    public static string NormalizeMySqlConnectionString(string cs)
    {
        if (string.IsNullOrWhiteSpace(cs)) return cs;
        if (cs.StartsWith("mysql://", StringComparison.OrdinalIgnoreCase) ||
            cs.StartsWith("mariadb://", StringComparison.OrdinalIgnoreCase))
        {
            try
            {
                var uri = new Uri(cs);
                var userInfo = uri.UserInfo.Split(':');
                var username = userInfo.Length > 0 ? Uri.UnescapeDataString(userInfo[0]) : "";
                var password = userInfo.Length > 1 ? Uri.UnescapeDataString(userInfo[1]) : "";
                var db = uri.AbsolutePath.TrimStart('/');
                var host = uri.Host;
                var port = uri.Port > 0 ? uri.Port : 3306;

                var builder = new MySqlConnector.MySqlConnectionStringBuilder
                {
                    Server = host,
                    Port = (uint)port,
                    Database = string.IsNullOrWhiteSpace(db) ? "octopus_db" : db,
                    UserID = username,
                    Password = password,
                    AllowUserVariables = true
                };
                return builder.ConnectionString;
            }
            catch
            {
                return cs;
            }
        }
        return cs;
    }

    public static string NormalizeSqlServerConnectionString(string cs)
    {
        if (string.IsNullOrWhiteSpace(cs)) return cs;
        if (cs.StartsWith("sqlserver://", StringComparison.OrdinalIgnoreCase) ||
            cs.StartsWith("mssql://", StringComparison.OrdinalIgnoreCase))
        {
            try
            {
                var clean = cs.Replace("sqlserver://", "http://", StringComparison.OrdinalIgnoreCase)
                              .Replace("mssql://", "http://", StringComparison.OrdinalIgnoreCase);
                var uri = new Uri(clean);
                var userInfo = uri.UserInfo.Split(':');
                var username = userInfo.Length > 0 ? Uri.UnescapeDataString(userInfo[0]) : "sa";
                var password = userInfo.Length > 1 ? Uri.UnescapeDataString(userInfo[1]) : "";
                var db = uri.AbsolutePath.TrimStart('/');
                var host = uri.Host;
                var port = uri.Port > 0 ? uri.Port : 1433;

                var builder = new Microsoft.Data.SqlClient.SqlConnectionStringBuilder
                {
                    DataSource = $"{host},{port}",
                    InitialCatalog = string.IsNullOrWhiteSpace(db) ? "octopus_db" : db,
                    UserID = username,
                    Password = password,
                    TrustServerCertificate = true,
                    Encrypt = false
                };
                return builder.ConnectionString;
            }
            catch
            {
                return cs;
            }
        }
        return cs;
    }

    public class SqlDbDataSource : DbDataSource
    {
        private readonly string _connectionString;
        public override string ConnectionString => _connectionString;

        public SqlDbDataSource(string connectionString)
        {
            _connectionString = connectionString;
        }

        protected override DbConnection CreateDbConnection() => new Microsoft.Data.SqlClient.SqlConnection(_connectionString);
    }

    private static void AddParameter(DbCommand cmd, object? value)
    {
        var p = cmd.CreateParameter();
        p.Value = value ?? DBNull.Value;
        cmd.Parameters.Add(p);
    }

    private void InitDb()
    {
        if (_dataSource == null) return;
        try
        {
            using var conn = _dataSource.OpenConnection();
            foreach (var ddl in new[] { _dialect.DdlExecutionStates, _dialect.DdlStepTraces })
            {
                foreach (var stmt in ddl.Split(';', StringSplitOptions.RemoveEmptyEntries))
                {
                    var sql = stmt.Trim();
                    if (!string.IsNullOrWhiteSpace(sql))
                    {
                        using var cmd = conn.CreateCommand();
                        cmd.CommandText = sql;
                        cmd.ExecuteNonQuery();
                    }
                }
            }
        }
        catch (Exception ex)
        {
            Console.WriteLine($"⚠️ [{_dialect.Name.ToUpperInvariant()} DDL] Init failed: {ex.Message}");
        }
    }

    public async Task SaveStateAsync(ExecutionState state, CancellationToken cancellationToken = default)
    {
        // Cache in-memory
        await _fallback.SaveStateAsync(state, cancellationToken);

        if (_dataSource == null) return;

        try
        {
            await using var conn = await _dataSource.OpenConnectionAsync(cancellationToken);
            await using var tx = await conn.BeginTransactionAsync(cancellationToken);

            var dataJson = JsonSerializer.Serialize(state.Data ?? new Dictionary<string, object>());
            var outputJson = state.OutputData != null ? JsonSerializer.Serialize(state.OutputData) : null;

            await using (var cmd = conn.CreateCommand())
            {
                cmd.CommandText = _dialect.UpsertExecutionStateSql;
                cmd.Transaction = tx;

                AddParameter(cmd, state.JobId);
                AddParameter(cmd, state.WorkflowName);
                AddParameter(cmd, state.Status.ToString());
                AddParameter(cmd, state.Route);
                AddParameter(cmd, state.Method);
                AddParameter(cmd, state.CurrentStepId);
                AddParameter(cmd, state.CurrentIndex);
                AddParameter(cmd, state.TotalSteps);
                AddParameter(cmd, dataJson);
                AddParameter(cmd, outputJson);
                AddParameter(cmd, state.Error);
                AddParameter(cmd, state.StartedAt);
                AddParameter(cmd, state.FinishedAt);
                AddParameter(cmd, state.ClientHost);
                AddParameter(cmd, state.DurationMs);
                AddParameter(cmd, state.ErrorCode);
                AddParameter(cmd, state.ErrorType);

                await cmd.ExecuteNonQueryAsync(cancellationToken);
            }

            await using (var delCmd = conn.CreateCommand())
            {
                delCmd.CommandText = _dialect.DeleteTracesSql;
                delCmd.Transaction = tx;
                AddParameter(delCmd, state.JobId);
                await delCmd.ExecuteNonQueryAsync(cancellationToken);
            }

            if (state.Traces != null && state.Traces.Count > 0)
            {
                foreach (var tr in state.Traces)
                {
                    await using var insCmd = conn.CreateCommand();
                    insCmd.CommandText = _dialect.InsertStepTraceSql;
                    insCmd.Transaction = tx;

                    var inJson = JsonSerializer.Serialize(tr.InputPayload ?? new Dictionary<string, object>());
                    var outJson = JsonSerializer.Serialize(tr.OutputPayload ?? new Dictionary<string, object>());

                    AddParameter(insCmd, state.JobId);
                    AddParameter(insCmd, tr.StepId);
                    AddParameter(insCmd, tr.ActivityName);
                    AddParameter(insCmd, tr.Status);
                    AddParameter(insCmd, inJson);
                    AddParameter(insCmd, outJson);
                    AddParameter(insCmd, tr.ErrorMessage);
                    AddParameter(insCmd, tr.StartedAt);
                    AddParameter(insCmd, tr.FinishedAt);
                    AddParameter(insCmd, tr.DurationMs);

                    await insCmd.ExecuteNonQueryAsync(cancellationToken);
                }
            }

            await tx.CommitAsync(cancellationToken);
        }
        catch (Exception ex)
        {
            Console.WriteLine($"⚠️ [{_dialect.Name.ToUpperInvariant()} SAVE ERROR] {ex.Message}");
        }
    }

    public async Task<ExecutionState?> GetStateAsync(string jobId, CancellationToken cancellationToken = default)
    {
        if (_dataSource == null)
        {
            return await _fallback.GetStateAsync(jobId, cancellationToken);
        }

        try
        {
            await using var conn = await _dataSource.OpenConnectionAsync(cancellationToken);
            await using var cmd = conn.CreateCommand();
            cmd.CommandText = _dialect.SelectStateSql;
            AddParameter(cmd, jobId);

            await using var reader = await cmd.ExecuteReaderAsync(cancellationToken);
            if (!await reader.ReadAsync(cancellationToken))
            {
                return await _fallback.GetStateAsync(jobId, cancellationToken);
            }

            var st = new ExecutionState
            {
                JobId = reader.GetString(0),
                WorkflowName = reader.GetString(1),
                Status = Enum.TryParse<ExecutionStatus>(reader.GetString(2), true, out var stat) ? stat : ExecutionStatus.COMPLETED,
                Route = reader.IsDBNull(3) ? null : reader.GetString(3),
                Method = reader.IsDBNull(4) ? null : reader.GetString(4),
                CurrentStepId = reader.IsDBNull(5) ? null : reader.GetString(5),
                CurrentIndex = reader.IsDBNull(6) ? 0 : reader.GetInt32(6),
                TotalSteps = reader.IsDBNull(7) ? 0 : reader.GetInt32(7),
                Error = reader.IsDBNull(10) ? null : reader.GetString(10),
                StartedAt = reader.IsDBNull(11) ? null : reader.GetString(11),
                FinishedAt = reader.IsDBNull(12) ? null : reader.GetString(12),
                ClientHost = reader.IsDBNull(13) ? null : reader.GetString(13),
                DurationMs = reader.IsDBNull(14) ? 0.0 : reader.GetDouble(14),
                ErrorCode = reader.IsDBNull(15) ? null : reader.GetInt32(15),
                ErrorType = reader.IsDBNull(16) ? null : reader.GetString(16)
            };

            if (!reader.IsDBNull(8))
            {
                var json = reader.GetString(8);
                try { st.Data = JsonSerializer.Deserialize<Dictionary<string, object>>(json) ?? new(); } catch { }
            }
            if (!reader.IsDBNull(9))
            {
                var json = reader.GetString(9);
                try { st.OutputData = JsonSerializer.Deserialize<Dictionary<string, object>>(json); } catch { }
            }

            await reader.CloseAsync();

            // Fetch Traces
            await using var traceCmd = conn.CreateCommand();
            traceCmd.CommandText = _dialect.SelectTracesSql;
            AddParameter(traceCmd, jobId);
            await using var traceReader = await traceCmd.ExecuteReaderAsync(cancellationToken);
            var traces = new List<StepExecutionTrace>();
            while (await traceReader.ReadAsync(cancellationToken))
            {
                var tr = new StepExecutionTrace
                {
                    StepId = traceReader.GetString(0),
                    ActivityName = traceReader.IsDBNull(1) ? null : traceReader.GetString(1),
                    Status = traceReader.GetString(2),
                    ErrorMessage = traceReader.IsDBNull(5) ? null : traceReader.GetString(5),
                    StartedAt = traceReader.IsDBNull(6) ? null : traceReader.GetString(6),
                    FinishedAt = traceReader.IsDBNull(7) ? null : traceReader.GetString(7),
                    DurationMs = traceReader.IsDBNull(8) ? 0.0 : traceReader.GetDouble(8)
                };

                if (!traceReader.IsDBNull(3))
                {
                    try { tr.InputPayload = JsonSerializer.Deserialize<Dictionary<string, object>>(traceReader.GetString(3)) ?? new(); } catch { }
                }
                if (!traceReader.IsDBNull(4))
                {
                    try { tr.OutputPayload = JsonSerializer.Deserialize<Dictionary<string, object>>(traceReader.GetString(4)) ?? new(); } catch { }
                }
                traces.Add(tr);
            }
            st.Traces = traces;

            return st;
        }
        catch (Exception ex)
        {
            Console.WriteLine($"⚠️ [{_dialect.Name.ToUpperInvariant()} GET ERROR] {ex.Message}");
            return await _fallback.GetStateAsync(jobId, cancellationToken);
        }
    }

    public async Task<IReadOnlyList<ExecutionState>> ListStatesAsync(string? workflowName = null, int limit = 100, CancellationToken cancellationToken = default)
    {
        if (_dataSource == null)
        {
            return await _fallback.ListStatesAsync(workflowName, limit, cancellationToken);
        }

        try
        {
            await using var conn = await _dataSource.OpenConnectionAsync(cancellationToken);
            var hasWf = !string.IsNullOrWhiteSpace(workflowName) && !string.Equals(workflowName, "ALL", StringComparison.OrdinalIgnoreCase);
            await using var cmd = conn.CreateCommand();
            cmd.CommandText = _dialect.ListStatesSql(hasWf);

            if (hasWf)
            {
                AddParameter(cmd, workflowName!);
                AddParameter(cmd, limit);
            }
            else
            {
                AddParameter(cmd, limit);
            }

            var jobIds = new List<string>();
            await using (var reader = await cmd.ExecuteReaderAsync(cancellationToken))
            {
                while (await reader.ReadAsync(cancellationToken))
                {
                    jobIds.Add(reader.GetString(0));
                }
            }

            var list = new List<ExecutionState>();
            foreach (var id in jobIds)
            {
                var st = await GetStateAsync(id, cancellationToken);
                if (st != null) list.Add(st);
            }

            return list;
        }
        catch (Exception ex)
        {
            Console.WriteLine($"⚠️ [{_dialect.Name.ToUpperInvariant()} LIST ERROR] {ex.Message}");
            return await _fallback.ListStatesAsync(workflowName, limit, cancellationToken);
        }
    }

    public Task PublishEventAsync(string jobId, LiveEvent liveEvent, CancellationToken cancellationToken = default)
    {
        return _fallback.PublishEventAsync(jobId, liveEvent, cancellationToken);
    }

    public IAsyncEnumerable<LiveEvent> SubscribeEventsAsync(string jobId, CancellationToken cancellationToken = default)
    {
        return _fallback.SubscribeEventsAsync(jobId, cancellationToken);
    }
}
