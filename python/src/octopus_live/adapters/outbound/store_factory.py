import os
from typing import Any, Dict, Optional, Union
from ...domain.ports.outbound import IStateStore
from .in_memory_store import InMemoryStateStore
from .sqlite_store import SqliteStateStore
from .duckdb_store import DuckDbStateStore
from .redis_store import RedisStateStore
from .relational_store import RelationalSqlStateStore, PostgresDialect, MySqlDialect, SqlServerDialect, SqliteDialect


class StoreFactory:
    """Zero-Import State Store Factory.
    
    Dynamically creates pluggable state store adapters (InMemory, SQLite, PostgreSQL, MySQL, SQL Server, DuckDB, Redis)
    based on configuration strings or dictionaries without requiring any database driver
    imports in client user code.
    """

    @classmethod
    def create(
        cls,
        store_type: Optional[str] = None,
        config: Optional[Dict[str, Any]] = None,
        **kwargs: Any
    ) -> IStateStore:
        cfg = dict(config or {})
        cfg.update(kwargs)

        # Resolve store type from explicit param, config, or env var
        resolved_type = (store_type or cfg.get("store") or cfg.get("type") or os.getenv("OCTOPUS_STATE_STORE") or "inmemory").lower()

        if resolved_type in ("sqlite", "sqlite3"):
            db_path = cfg.get("path") or cfg.get("sqlite_path") or cfg.get("db_path") or "data/octopus_state.sqlite"
            if db_path != ":memory:":
                parent = os.path.dirname(db_path)
                if parent and not os.path.exists(parent):
                    os.makedirs(parent, exist_ok=True)
            return RelationalSqlStateStore(db_type="sqlite", db_path=db_path, dialect=SqliteDialect())

        elif resolved_type in ("postgres", "postgresql", "pgsql"):
            dsn = cfg.get("dsn") or cfg.get("url") or cfg.get("connection_string") or os.getenv("POSTGRES_DSN")
            return RelationalSqlStateStore(db_type="postgres", dsn=dsn, dialect=PostgresDialect())

        elif resolved_type in ("mysql", "mariadb"):
            dsn = cfg.get("dsn") or cfg.get("url") or cfg.get("connection_string") or os.getenv("MYSQL_DSN")
            return RelationalSqlStateStore(db_type="mysql", dsn=dsn, dialect=MySqlDialect())

        elif resolved_type in ("sqlserver", "mssql", "tsql"):
            dsn = cfg.get("dsn") or cfg.get("url") or cfg.get("connection_string") or os.getenv("SQLSERVER_DSN")
            return RelationalSqlStateStore(db_type="sqlserver", dsn=dsn, dialect=SqlServerDialect())

        elif resolved_type in ("duckdb", "duck"):
            db_path = cfg.get("path") or cfg.get("duckdb_path") or cfg.get("db_path") or "data/octopus_traces.duckdb"
            if db_path != ":memory:":
                parent = os.path.dirname(db_path)
                if parent and not os.path.exists(parent):
                    os.makedirs(parent, exist_ok=True)
            return DuckDbStateStore(db_path=db_path)

        elif resolved_type in ("redis", "redis_cluster"):
            redis_url = cfg.get("url") or cfg.get("redis_url") or cfg.get("dsn") or os.getenv("REDIS_URL") or "redis://localhost:6379"
            retention = cfg.get("retention") or {}
            ttl = int(cfg.get("ttl_seconds") or cfg.get("ttl") or 86400)
            ttl_success = int(retention.get("success_ttl_seconds") or cfg.get("ttl_success_seconds") or 3600)
            ttl_failure = int(retention.get("failure_ttl_seconds") or cfg.get("ttl_failure_seconds") or 604800)
            ttl_running = int(retention.get("running_ttl_seconds") or cfg.get("ttl_running_seconds") or 7200)
            prefix = str(cfg.get("prefix") or "octopus:live")
            return RedisStateStore(
                redis_url=redis_url,
                ttl_seconds=ttl,
                ttl_success_seconds=ttl_success,
                ttl_failure_seconds=ttl_failure,
                ttl_running_seconds=ttl_running,
                prefix=prefix
            )

        # Default fallback: pure concurrent in-memory store
        return InMemoryStateStore()
