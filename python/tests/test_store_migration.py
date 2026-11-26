"""A SQLite database created before the newer job columns still works (it gets them)."""

import asyncio
import sqlite3

from octopus_live.adapters.outbound.relational_store import RelationalSqlStateStore, SqliteDialect
from octopus_live.domain.models.models import ExecutionState, ExecutionStatus, StepExecutionTrace

OLD_SCHEMA = """
CREATE TABLE execution_states (job_id TEXT PRIMARY KEY, workflow_name TEXT, status TEXT, route TEXT,
    method TEXT, current_step_id TEXT, current_index INTEGER, total_steps INTEGER, data TEXT,
    output_data TEXT, error TEXT, started_at TEXT, finished_at TEXT);
CREATE TABLE step_traces (id INTEGER PRIMARY KEY AUTOINCREMENT, job_id TEXT, step_id TEXT,
    activity_name TEXT, status TEXT, input_payload TEXT, output_payload TEXT, error_message TEXT,
    started_at TEXT, finished_at TEXT);
"""


def test_an_old_sqlite_database_gets_the_newer_columns(tmp_path):
    path = str(tmp_path / "old.sqlite")
    with sqlite3.connect(path) as conn:
        conn.executescript(OLD_SCHEMA)

    store = RelationalSqlStateStore(db_type="sqlite", db_path=path, dialect=SqliteDialect())
    state = ExecutionState(job_id="job-old", workflow_name="wf", status=ExecutionStatus.COMPLETED)
    state.client_host = "10.0.0.7"
    state.duration_ms = 12.5
    state.error_type = "Boom"
    state.traces.append(StepExecutionTrace(step_id="a", activity_name="A", status="COMPLETED", duration_ms=3.0))
    asyncio.run(store.save_state(state))

    back = asyncio.run(store.get_state("job-old"))
    assert back is not None and back.client_host == "10.0.0.7" and back.error_type == "Boom"
    assert asyncio.run(store.list_states("wf"))[0].job_id == "job-old"
    with sqlite3.connect(path) as conn:
        assert conn.execute("SELECT duration_ms FROM step_traces WHERE job_id = 'job-old'").fetchone() == (3.0,)

    # Opening it again changes nothing.
    RelationalSqlStateStore(db_type="sqlite", db_path=path, dialect=SqliteDialect())
