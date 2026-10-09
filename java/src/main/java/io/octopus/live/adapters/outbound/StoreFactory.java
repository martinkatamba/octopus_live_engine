package io.octopus.live.adapters.outbound;

import io.octopus.live.domain.ports.IStateStore;
import java.util.Collections;
import java.util.Map;

public class StoreFactory {

    public static IStateStore create(String storeType, Map<String, Object> config) {
        Map<String, Object> cfg = config != null ? config : Collections.emptyMap();
        String type = storeType;
        if (type == null || type.isBlank()) {
            if (cfg.containsKey("store") && cfg.get("store") != null) {
                type = cfg.get("store").toString();
            } else if (cfg.containsKey("type") && cfg.get("type") != null) {
                type = cfg.get("type").toString();
            } else {
                type = System.getenv("OCTOPUS_STATE_STORE");
                if (type == null) type = "inmemory";
            }
        }

        String dsn = null;
        if (cfg.containsKey("dsn") && cfg.get("dsn") != null && !cfg.get("dsn").toString().isBlank()) {
            dsn = cfg.get("dsn").toString();
        } else if (cfg.containsKey("url") && cfg.get("url") != null && !cfg.get("url").toString().isBlank()) {
            dsn = cfg.get("url").toString();
        } else if (cfg.containsKey("connection_string") && cfg.get("connection_string") != null && !cfg.get("connection_string").toString().isBlank()) {
            dsn = cfg.get("connection_string").toString();
        }

        String normalized = type.trim().toLowerCase();
        return switch (normalized) {
            case "redis", "redis_cluster" -> {
                String redisUrl = dsn != null ? dsn : System.getenv("REDIS_URL");
                String prefix = cfg.containsKey("prefix") && cfg.get("prefix") != null ? cfg.get("prefix").toString() : "octopus:live";
                Long ttlSuccess = null;
                Long ttlFailure = null;
                Long ttlRunning = null;

                if (cfg.get("retention") instanceof Map<?, ?> retMap) {
                    if (retMap.get("success_ttl_seconds") != null) {
                        ttlSuccess = Long.parseLong(retMap.get("success_ttl_seconds").toString());
                    }
                    if (retMap.get("failure_ttl_seconds") != null) {
                        ttlFailure = Long.parseLong(retMap.get("failure_ttl_seconds").toString());
                    }
                    if (retMap.get("running_ttl_seconds") != null) {
                        ttlRunning = Long.parseLong(retMap.get("running_ttl_seconds").toString());
                    }
                }
                yield new RedisStateStore(redisUrl, ttlSuccess, ttlFailure, ttlRunning, prefix);
            }
            case "postgres", "postgresql", "pgsql" -> new RelationalSqlStateStore(
                new RelationalSqlStateStore.PostgresDialect(),
                dsn != null ? dsn : System.getenv("POSTGRES_DSN")
            );
            case "mysql", "mariadb" -> new RelationalSqlStateStore(
                new RelationalSqlStateStore.MySqlDialect(),
                dsn != null ? dsn : System.getenv("MYSQL_DSN")
            );
            case "sqlserver", "mssql", "tsql" -> new RelationalSqlStateStore(
                new RelationalSqlStateStore.SqlServerDialect(),
                dsn != null ? dsn : System.getenv("SQLSERVER_DSN")
            );
            case "sqlite", "sqlite3" -> new RelationalSqlStateStore(
                new RelationalSqlStateStore.SqliteDialect(),
                dsn != null ? dsn : (cfg.containsKey("sqlite_path") && cfg.get("sqlite_path") != null ? cfg.get("sqlite_path").toString() : System.getenv("SQLITE_PATH"))
            );
            default -> new InMemoryStateStore();
        };
    }
}
