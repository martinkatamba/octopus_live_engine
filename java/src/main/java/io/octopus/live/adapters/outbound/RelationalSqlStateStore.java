package io.octopus.live.adapters.outbound;

import com.fasterxml.jackson.core.type.TypeReference;
import com.fasterxml.jackson.databind.ObjectMapper;
import io.octopus.live.domain.models.ExecutionState;
import io.octopus.live.domain.models.ExecutionStatus;
import io.octopus.live.domain.models.LiveEvent;
import io.octopus.live.domain.models.StepExecutionTrace;
import io.octopus.live.domain.ports.IStateStore;

import javax.sql.DataSource;
import java.net.URI;
import java.sql.*;
import java.util.*;
import java.util.function.Consumer;

public class RelationalSqlStateStore implements IStateStore {

    public interface ISqlDialect {
        String getName();
        String getPlaceholder(int index);
        String getDdlExecutionStates();
        String getDdlStepTraces();
        String getUpsertExecutionStateSql();
        String getInsertStepTraceSql();
        String getSelectStateSql();
        String getSelectTracesSql();
        String getListStatesSql(boolean hasWorkflow);
        String getDeleteTracesSql();
    }

    public static class PostgresDialect implements ISqlDialect {
        @Override public String getName() { return "postgres"; }
        @Override public String getPlaceholder(int index) { return "$" + index; }
        @Override public String getDdlExecutionStates() {
            return """
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
        }
        @Override public String getDdlStepTraces() {
            return """
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
        }
        @Override public String getUpsertExecutionStateSql() {
            return """
                INSERT INTO execution_states (
                    job_id, workflow_name, status, route, method,
                    current_step_id, current_index, total_steps,
                    data, output_data, error, started_at, finished_at,
                    client_host, duration_ms, error_code, error_type
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
        }
        @Override public String getInsertStepTraceSql() {
            return "INSERT INTO step_traces (job_id, step_id, activity_name, status, input_payload, output_payload, error_message, started_at, finished_at, duration_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";
        }
        @Override public String getSelectStateSql() {
            return "SELECT job_id, workflow_name, status, route, method, current_step_id, current_index, total_steps, data, output_data, error, started_at, finished_at, client_host, duration_ms, error_code, error_type FROM execution_states WHERE job_id = ?";
        }
        @Override public String getSelectTracesSql() {
            return "SELECT step_id, activity_name, status, input_payload, output_payload, error_message, started_at, finished_at, duration_ms FROM step_traces WHERE job_id = ? ORDER BY id ASC";
        }
        @Override public String getListStatesSql(boolean hasWorkflow) {
            return hasWorkflow
                ? "SELECT job_id, workflow_name, status, route, method, current_step_id, current_index, total_steps, data, output_data, error, started_at, finished_at, client_host, duration_ms, error_code, error_type FROM execution_states WHERE workflow_name = ? ORDER BY started_at DESC LIMIT ?"
                : "SELECT job_id, workflow_name, status, route, method, current_step_id, current_index, total_steps, data, output_data, error, started_at, finished_at, client_host, duration_ms, error_code, error_type FROM execution_states ORDER BY started_at DESC LIMIT ?";
        }
        @Override public String getDeleteTracesSql() { return "DELETE FROM step_traces WHERE job_id = ?"; }
    }

    public static class MySqlDialect implements ISqlDialect {
        @Override public String getName() { return "mysql"; }
        @Override public String getPlaceholder(int index) { return "?"; }
        @Override public String getDdlExecutionStates() {
            return """
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
        }
        @Override public String getDdlStepTraces() {
            return """
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
        }
        @Override public String getUpsertExecutionStateSql() {
            return """
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
        }
        @Override public String getInsertStepTraceSql() {
            return "INSERT INTO step_traces (job_id, step_id, activity_name, status, input_payload, output_payload, error_message, started_at, finished_at, duration_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";
        }
        @Override public String getSelectStateSql() {
            return "SELECT job_id, workflow_name, status, route, method, current_step_id, current_index, total_steps, data, output_data, error, started_at, finished_at, client_host, duration_ms, error_code, error_type FROM execution_states WHERE job_id = ?";
        }
        @Override public String getSelectTracesSql() {
            return "SELECT step_id, activity_name, status, input_payload, output_payload, error_message, started_at, finished_at, duration_ms FROM step_traces WHERE job_id = ? ORDER BY id ASC";
        }
        @Override public String getListStatesSql(boolean hasWorkflow) {
            return hasWorkflow
                ? "SELECT job_id, workflow_name, status, route, method, current_step_id, current_index, total_steps, data, output_data, error, started_at, finished_at, client_host, duration_ms, error_code, error_type FROM execution_states WHERE workflow_name = ? ORDER BY started_at DESC LIMIT ?"
                : "SELECT job_id, workflow_name, status, route, method, current_step_id, current_index, total_steps, data, output_data, error, started_at, finished_at, client_host, duration_ms, error_code, error_type FROM execution_states ORDER BY started_at DESC LIMIT ?";
        }
        @Override public String getDeleteTracesSql() { return "DELETE FROM step_traces WHERE job_id = ?"; }
    }

    public static class SqlServerDialect implements ISqlDialect {
        @Override public String getName() { return "sqlserver"; }
        @Override public String getPlaceholder(int index) { return "?"; }
        @Override public String getDdlExecutionStates() {
            return """
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
        }
        @Override public String getDdlStepTraces() {
            return """
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
        }
        @Override public String getUpsertExecutionStateSql() {
            return """
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
                    INSERT (job_id, workflow_name, status, route, method, current_step_id, current_index, total_steps, data, output_data, error, started_at, finished_at, client_host, duration_ms, error_code, error_type)
                    VALUES (source.job_id, source.workflow_name, source.status, source.route, source.method, source.current_step_id, source.current_index, source.total_steps, source.data, source.output_data, source.error, source.started_at, source.finished_at, source.client_host, source.duration_ms, source.error_code, source.error_type);
            """;
        }
        @Override public String getInsertStepTraceSql() {
            return "INSERT INTO step_traces (job_id, step_id, activity_name, status, input_payload, output_payload, error_message, started_at, finished_at, duration_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";
        }
        @Override public String getSelectStateSql() {
            return "SELECT job_id, workflow_name, status, route, method, current_step_id, current_index, total_steps, data, output_data, error, started_at, finished_at, client_host, duration_ms, error_code, error_type FROM execution_states WHERE job_id = ?";
        }
        @Override public String getSelectTracesSql() {
            return "SELECT step_id, activity_name, status, input_payload, output_payload, error_message, started_at, finished_at, duration_ms FROM step_traces WHERE job_id = ? ORDER BY id ASC";
        }
        @Override public String getListStatesSql(boolean hasWorkflow) {
            return hasWorkflow
                ? "SELECT TOP (?) job_id, workflow_name, status, route, method, current_step_id, current_index, total_steps, data, output_data, error, started_at, finished_at, client_host, duration_ms, error_code, error_type FROM execution_states WHERE workflow_name = ? ORDER BY started_at DESC"
                : "SELECT TOP (?) job_id, workflow_name, status, route, method, current_step_id, current_index, total_steps, data, output_data, error, started_at, finished_at, client_host, duration_ms, error_code, error_type FROM execution_states ORDER BY started_at DESC";
        }
        @Override public String getDeleteTracesSql() { return "DELETE FROM step_traces WHERE job_id = ?"; }
    }

    public static class SqliteDialect implements ISqlDialect {
        @Override public String getName() { return "sqlite"; }
        @Override public String getPlaceholder(int index) { return "?"; }
        @Override public String getDdlExecutionStates() {
            return """
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
        }
        @Override public String getDdlStepTraces() {
            return """
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
        }
        @Override public String getUpsertExecutionStateSql() {
            return """
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
        }
        @Override public String getInsertStepTraceSql() {
            return "INSERT INTO step_traces (job_id, step_id, activity_name, status, input_payload, output_payload, error_message, started_at, finished_at, duration_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";
        }
        @Override public String getSelectStateSql() {
            return "SELECT job_id, workflow_name, status, route, method, current_step_id, current_index, total_steps, data, output_data, error, started_at, finished_at, client_host, duration_ms, error_code, error_type FROM execution_states WHERE job_id = ?";
        }
        @Override public String getSelectTracesSql() {
            return "SELECT step_id, activity_name, status, input_payload, output_payload, error_message, started_at, finished_at, duration_ms FROM step_traces WHERE job_id = ? ORDER BY id ASC";
        }
        @Override public String getListStatesSql(boolean hasWorkflow) {
            return hasWorkflow
                ? "SELECT job_id, workflow_name, status, route, method, current_step_id, current_index, total_steps, data, output_data, error, started_at, finished_at, client_host, duration_ms, error_code, error_type FROM execution_states WHERE workflow_name = ? ORDER BY started_at DESC LIMIT ?"
                : "SELECT job_id, workflow_name, status, route, method, current_step_id, current_index, total_steps, data, output_data, error, started_at, finished_at, client_host, duration_ms, error_code, error_type FROM execution_states ORDER BY started_at DESC LIMIT ?";
        }
        @Override public String getDeleteTracesSql() { return "DELETE FROM step_traces WHERE job_id = ?"; }
    }

    private final ISqlDialect dialect;
    private final InMemoryStateStore fallback = new InMemoryStateStore();
    private final DataSource dataSource;
    private final ObjectMapper mapper = new ObjectMapper();

    public RelationalSqlStateStore(ISqlDialect dialect) {
        this(dialect, (DataSource) null);
    }

    public RelationalSqlStateStore(ISqlDialect dialect, String connectionString) {
        this(dialect, createDataSource(dialect, connectionString));
    }

    public RelationalSqlStateStore(ISqlDialect dialect, DataSource dataSource) {
        this.dialect = dialect != null ? dialect : new SqliteDialect();
        this.dataSource = dataSource;
        if (this.dataSource != null) {
            initDb();
        }
    }

    public static DataSource createDataSource(ISqlDialect dialect, String cs) {
        if (cs == null || cs.isBlank()) return null;
        try {
            if ("postgres".equalsIgnoreCase(dialect.getName()) || cs.startsWith("postgres://") || cs.startsWith("postgresql://") || cs.startsWith("jdbc:postgresql:")) {
                org.postgresql.ds.PGSimpleDataSource ds = new org.postgresql.ds.PGSimpleDataSource();
                String url = cs;
                if (url.startsWith("postgres://") || url.startsWith("postgresql://")) {
                    String cleanUrl = url.replace("postgres://", "http://").replace("postgresql://", "http://");
                    URI uri = new URI(cleanUrl);
                    String host = uri.getHost() != null ? uri.getHost() : "localhost";
                    int port = uri.getPort() > 0 ? uri.getPort() : 5432;
                    String db = uri.getPath() != null && uri.getPath().length() > 1 ? uri.getPath().substring(1) : "postgres";
                    ds.setServerNames(new String[]{host});
                    ds.setPortNumbers(new int[]{port});
                    ds.setDatabaseName(db);
                    if (uri.getUserInfo() != null) {
                        String[] ui = uri.getUserInfo().split(":", 2);
                        ds.setUser(ui[0]);
                        if (ui.length > 1) ds.setPassword(ui[1]);
                    }
                    if (uri.getQuery() != null && uri.getQuery().contains("sslmode=disable")) {
                        ds.setSsl(false);
                    }
                } else if (url.startsWith("jdbc:postgresql:")) {
                    ds.setUrl(url);
                } else {
                    ds.setUrl("jdbc:postgresql://" + url);
                }
                return ds;
            } else if ("mysql".equalsIgnoreCase(dialect.getName()) || cs.startsWith("mysql://") || cs.startsWith("jdbc:mysql:") || cs.contains("@tcp(")) {
                com.mysql.cj.jdbc.MysqlDataSource ds = new com.mysql.cj.jdbc.MysqlDataSource();
                String url = cs;
                if (url.startsWith("mysql://")) {
                    String cleanUrl = url.replace("mysql://", "http://");
                    URI uri = new URI(cleanUrl);
                    String host = uri.getHost() != null ? uri.getHost() : "localhost";
                    int port = uri.getPort() > 0 ? uri.getPort() : 3306;
                    String db = uri.getPath() != null && uri.getPath().length() > 1 ? uri.getPath().substring(1) : "octopus_db";
                    String user = "";
                    String pass = "";
                    if (uri.getUserInfo() != null) {
                        String[] ui = uri.getUserInfo().split(":", 2);
                        user = ui[0];
                        if (ui.length > 1) pass = ui[1];
                    }
                    ds.setUrl("jdbc:mysql://" + host + ":" + port + "/" + db + "?allowPublicKeyRetrieval=true&useSSL=false&serverTimezone=UTC");
                    if (!user.isEmpty()) ds.setUser(user);
                    if (!pass.isEmpty()) ds.setPassword(pass);
                } else if (url.contains("@tcp(")) {
                    int atIdx = url.indexOf('@');
                    int tcpIdx = url.indexOf("tcp(");
                    int closeIdx = url.indexOf(')', tcpIdx);
                    int slashIdx = url.indexOf('/', closeIdx);
                    String userPass = url.substring(0, atIdx);
                    String[] up = userPass.split(":", 2);
                    String hostPort = url.substring(tcpIdx + 4, closeIdx);
                    String[] hp = hostPort.split(":", 2);
                    String host = hp[0];
                    int port = hp.length > 1 ? Integer.parseInt(hp[1]) : 3306;
                    String db = slashIdx >= 0 ? url.substring(slashIdx + 1).split("\\?")[0] : "octopus_db";
                    ds.setUrl("jdbc:mysql://" + host + ":" + port + "/" + db + "?allowPublicKeyRetrieval=true&useSSL=false&serverTimezone=UTC");
                    ds.setUser(up[0]);
                    if (up.length > 1) ds.setPassword(up[1]);
                } else if (url.startsWith("jdbc:mysql:")) {
                    ds.setUrl(url);
                } else {
                    ds.setUrl("jdbc:mysql://" + url + "?allowPublicKeyRetrieval=true&useSSL=false&serverTimezone=UTC");
                }
                return ds;
            } else if ("sqlserver".equalsIgnoreCase(dialect.getName()) || "mssql".equalsIgnoreCase(dialect.getName()) || cs.startsWith("sqlserver://") || cs.startsWith("mssql://") || cs.startsWith("jdbc:sqlserver:")) {
                com.microsoft.sqlserver.jdbc.SQLServerDataSource ds = new com.microsoft.sqlserver.jdbc.SQLServerDataSource();
                String url = cs;
                if (url.startsWith("sqlserver://") || url.startsWith("mssql://")) {
                    String cleanUrl = url.replace("sqlserver://", "http://").replace("mssql://", "http://");
                    URI uri = new URI(cleanUrl);
                    String host = uri.getHost() != null ? uri.getHost() : "localhost";
                    int port = uri.getPort() > 0 ? uri.getPort() : 1433;
                    String db = uri.getPath() != null && uri.getPath().length() > 1 ? uri.getPath().substring(1) : "octopus_db";
                    ds.setServerName(host);
                    ds.setPortNumber(port);
                    ds.setDatabaseName(db);
                    if (uri.getUserInfo() != null) {
                        String[] ui = uri.getUserInfo().split(":", 2);
                        ds.setUser(ui[0]);
                        if (ui.length > 1) ds.setPassword(ui[1]);
                    }
                    ds.setTrustServerCertificate(true);
                    ds.setEncrypt(false);
                } else if (url.startsWith("jdbc:sqlserver:")) {
                    ds.setURL(url);
                } else {
                    String jdbcUrl = url.startsWith("jdbc:") ? url : "jdbc:sqlserver://" + url;
                    if (!jdbcUrl.contains("trustServerCertificate=")) {
                        jdbcUrl += ";trustServerCertificate=true;encrypt=false;";
                    }
                    ds.setURL(jdbcUrl);
                }
                return ds;
            }
        } catch (Exception ex) {
            System.err.println("⚠️ [DATA SOURCE] Failed to configure DataSource: " + ex.getMessage());
            ex.printStackTrace();
        }
        return null;
    }

    private void initDb() {
        if (dataSource == null) return;
        try (Connection conn = dataSource.getConnection()) {
            for (String ddl : new String[]{dialect.getDdlExecutionStates(), dialect.getDdlStepTraces()}) {
                for (String stmt : ddl.split(";")) {
                    String sql = stmt.trim();
                    if (!sql.isBlank()) {
                        try (Statement st = conn.createStatement()) {
                            st.execute(sql);
                        }
                    }
                }
            }
        } catch (Exception ex) {
            System.err.println("⚠️ [SQL STATE STORE] DDL Init Warning: " + ex.getMessage());
        }
    }

    public ISqlDialect getDialect() {
        return dialect;
    }

    public DataSource getDataSource() {
        return dataSource;
    }

    private String toJson(Object obj) {
        if (obj == null) return null;
        try {
            return mapper.writeValueAsString(obj);
        } catch (Exception e) {
            return String.valueOf(obj);
        }
    }

    private Map<String, Object> parseJson(String json) {
        if (json == null || json.isBlank()) return new HashMap<>();
        try {
            return mapper.readValue(json, new TypeReference<Map<String, Object>>() {});
        } catch (Exception e) {
            return new HashMap<>();
        }
    }

    private ExecutionState mapRowToExecutionState(ResultSet rs) throws SQLException {
        ExecutionState state = new ExecutionState();
        state.setJobId(rs.getString("job_id"));
        state.setWorkflowName(rs.getString("workflow_name"));
        String statusStr = rs.getString("status");
        if (statusStr != null) {
            try { state.setStatus(ExecutionStatus.valueOf(statusStr)); } catch (Exception ignored) {}
        }
        state.setRoute(rs.getString("route"));
        state.setMethod(rs.getString("method"));
        state.setCurrentStepId(rs.getString("current_step_id"));
        state.setCurrentIndex(rs.getInt("current_index"));
        state.setTotalSteps(rs.getInt("total_steps"));
        state.setData(parseJson(rs.getString("data")));
        state.setOutputData(parseJson(rs.getString("output_data")));
        state.setError(rs.getString("error"));
        state.setStartedAt(rs.getString("started_at"));
        state.setFinishedAt(rs.getString("finished_at"));
        state.setClientHost(rs.getString("client_host"));
        double dur = rs.getDouble("duration_ms");
        if (!rs.wasNull()) state.setDurationMs(dur);
        int ec = rs.getInt("error_code");
        if (!rs.wasNull()) state.setErrorCode(ec);
        state.setErrorType(rs.getString("error_type"));
        return state;
    }

    private StepExecutionTrace mapRowToStepTrace(ResultSet rs) throws SQLException {
        StepExecutionTrace trace = new StepExecutionTrace();
        trace.setStepId(rs.getString("step_id"));
        trace.setActivityName(rs.getString("activity_name"));
        String statusStr = rs.getString("status");
        if (statusStr != null) {
            trace.setStatus(statusStr);
        }
        trace.setInputPayload(parseJson(rs.getString("input_payload")));
        trace.setOutputPayload(parseJson(rs.getString("output_payload")));
        trace.setErrorMessage(rs.getString("error_message"));
        trace.setStartedAt(rs.getString("started_at"));
        trace.setFinishedAt(rs.getString("finished_at"));
        double dur = rs.getDouble("duration_ms");
        if (!rs.wasNull()) trace.setDurationMs(dur);
        return trace;
    }

    private String toJdbcSql(String sql) {
        if (sql == null) return null;
        return sql.replaceAll("\\$\\d+", "?");
    }

    @Override
    public void saveState(ExecutionState state) {
        if (state == null) return;
        fallback.saveState(state);

        if (dataSource == null) return;
        try (Connection conn = dataSource.getConnection()) {
            conn.setAutoCommit(false);

            // 1. Upsert execution state
            try (PreparedStatement ps = conn.prepareStatement(toJdbcSql(dialect.getUpsertExecutionStateSql()))) {
                ps.setString(1, state.getJobId());
                ps.setString(2, state.getWorkflowName() != null ? state.getWorkflowName() : "default");
                ps.setString(3, state.getStatus() != null ? state.getStatus().name() : "IDLE");
                ps.setString(4, state.getRoute());
                ps.setString(5, state.getMethod());
                ps.setString(6, state.getCurrentStepId());
                ps.setInt(7, state.getCurrentIndex());
                ps.setInt(8, state.getTotalSteps());
                ps.setString(9, toJson(state.getData()));
                ps.setString(10, toJson(state.getOutputData()));
                ps.setString(11, state.getError());
                ps.setString(12, state.getStartedAt());
                ps.setString(13, state.getFinishedAt());
                ps.setString(14, state.getClientHost());
                if (state.getDurationMs() != null) ps.setDouble(15, state.getDurationMs()); else ps.setNull(15, Types.DOUBLE);
                if (state.getErrorCode() != null) ps.setInt(16, state.getErrorCode()); else ps.setNull(16, Types.INTEGER);
                ps.setString(17, state.getErrorType());
                ps.executeUpdate();
            }

            // 2. Delete existing traces
            try (PreparedStatement delPs = conn.prepareStatement(toJdbcSql(dialect.getDeleteTracesSql()))) {
                delPs.setString(1, state.getJobId());
                delPs.executeUpdate();
            }

            // 3. Insert traces
            if (state.getTraces() != null && !state.getTraces().isEmpty()) {
                try (PreparedStatement insPs = conn.prepareStatement(toJdbcSql(dialect.getInsertStepTraceSql()))) {
                    for (StepExecutionTrace trace : state.getTraces()) {
                        insPs.setString(1, state.getJobId());
                        insPs.setString(2, trace.getStepId());
                        insPs.setString(3, trace.getActivityName());
                        insPs.setString(4, trace.getStatus() != null ? trace.getStatus() : "COMPLETED");
                        insPs.setString(5, toJson(trace.getInputPayload()));
                        insPs.setString(6, toJson(trace.getOutputPayload()));
                        insPs.setString(7, trace.getErrorMessage());
                        insPs.setString(8, trace.getStartedAt());
                        insPs.setString(9, trace.getFinishedAt());
                        insPs.setDouble(10, trace.getDurationMs());
                        insPs.addBatch();
                    }
                    insPs.executeBatch();
                }
            }

            conn.commit();
        } catch (Exception ex) {
            System.err.println("⚠️ [SQL STATE STORE] saveState error: " + ex.getMessage());
        }
    }

    @Override
    public ExecutionState getState(String jobId) {
        if (jobId == null || jobId.isBlank()) return null;
        if (dataSource == null) return fallback.getState(jobId);

        try (Connection conn = dataSource.getConnection()) {
            ExecutionState state = null;
            try (PreparedStatement ps = conn.prepareStatement(toJdbcSql(dialect.getSelectStateSql()))) {
                ps.setString(1, jobId);
                try (ResultSet rs = ps.executeQuery()) {
                    if (rs.next()) {
                        state = mapRowToExecutionState(rs);
                    }
                }
            }

            if (state != null) {
                try (PreparedStatement psTraces = conn.prepareStatement(toJdbcSql(dialect.getSelectTracesSql()))) {
                    psTraces.setString(1, jobId);
                    try (ResultSet rsTraces = psTraces.executeQuery()) {
                        while (rsTraces.next()) {
                            state.getTraces().add(mapRowToStepTrace(rsTraces));
                        }
                    }
                }
                return state;
            }
        } catch (Exception ex) {
            System.err.println("⚠️ [SQL STATE STORE] getState error: " + ex.getMessage());
        }

        return fallback.getState(jobId);
    }

    @Override
    public List<ExecutionState> listStates(String workflowName, int limit) {
        if (dataSource == null) return fallback.listStates(workflowName, limit);

        List<ExecutionState> result = new ArrayList<>();
        boolean hasWf = workflowName != null && !workflowName.isBlank();
        String sql = toJdbcSql(dialect.getListStatesSql(hasWf));

        try (Connection conn = dataSource.getConnection();
             PreparedStatement ps = conn.prepareStatement(sql)) {
            if (hasWf) {
                ps.setString(1, workflowName);
                ps.setInt(2, limit > 0 ? limit : 100);
            } else {
                ps.setInt(1, limit > 0 ? limit : 100);
            }

            try (ResultSet rs = ps.executeQuery()) {
                while (rs.next()) {
                    result.add(mapRowToExecutionState(rs));
                }
            }
            return result;
        } catch (Exception ex) {
            System.err.println("⚠️ [SQL STATE STORE] listStates error: " + ex.getMessage());
        }

        return fallback.listStates(workflowName, limit);
    }

    @Override
    public void publishEvent(String jobId, LiveEvent event) {
        fallback.publishEvent(jobId, event);
    }

    @Override
    public AutoCloseable subscribeEvents(String jobId, Consumer<LiveEvent> consumer) {
        return fallback.subscribeEvents(jobId, consumer);
    }
}
