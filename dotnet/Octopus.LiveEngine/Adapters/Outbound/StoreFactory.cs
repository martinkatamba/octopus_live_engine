using Octopus.LiveEngine.Domain.Ports;

namespace Octopus.LiveEngine.Adapters.Outbound;

public static class StoreFactory
{
    public static IStateStore Create(string? storeType = null, Dictionary<string, object>? config = null)
    {
        var cfg = config ?? new Dictionary<string, object>();
        var type = storeType;

        if (string.IsNullOrWhiteSpace(type))
        {
            if (cfg.TryGetValue("store", out var s) && s != null)
                type = s.ToString();
            else if (cfg.TryGetValue("type", out var t) && t != null)
                type = t.ToString();
            else
                type = Environment.GetEnvironmentVariable("OCTOPUS_STATE_STORE") ?? "inmemory";
        }

        var normalized = type?.Trim().ToLowerInvariant() ?? "inmemory";

        string? GetDsn(string envKey)
        {
            if (cfg.TryGetValue("dsn", out var d) && d != null && !string.IsNullOrWhiteSpace(d.ToString()))
                return d.ToString();
            if (cfg.TryGetValue("url", out var u) && u != null && !string.IsNullOrWhiteSpace(u.ToString()))
                return u.ToString();
            if (cfg.TryGetValue("connection_string", out var cs) && cs != null && !string.IsNullOrWhiteSpace(cs.ToString()))
                return cs.ToString();
            return Environment.GetEnvironmentVariable(envKey);
        }

        if (normalized == "redis" || normalized == "redis_cluster")
        {
            var redisUrl = GetDsn("REDIS_URL") ?? "localhost:6379";
            // Normalize redis:// prefix if present for StackExchange.Redis
            if (redisUrl.StartsWith("redis://", StringComparison.OrdinalIgnoreCase))
            {
                redisUrl = redisUrl.Substring("redis://".Length);
                var slashIdx = redisUrl.IndexOf('/');
                if (slashIdx != -1) redisUrl = redisUrl.Substring(0, slashIdx);
            }

            TimeSpan? ttlSuccess = null;
            TimeSpan? ttlFailure = null;
            TimeSpan? ttlRunning = null;
            string prefix = "octopus:live";

            if (cfg.TryGetValue("prefix", out var pr) && pr != null)
                prefix = pr.ToString()!;

            if (cfg.TryGetValue("retention", out var retObj) && retObj is System.Collections.IDictionary retDict)
            {
                foreach (System.Collections.DictionaryEntry entry in retDict)
                {
                    var k = entry.Key?.ToString()?.ToLowerInvariant() ?? "";
                    if ((k == "success_ttl_seconds" || k == "ttl_success_seconds" || k == "success") && double.TryParse(entry.Value?.ToString(), out var sSec))
                        ttlSuccess = TimeSpan.FromSeconds(sSec);
                    else if ((k == "failure_ttl_seconds" || k == "ttl_failure_seconds" || k == "failure") && double.TryParse(entry.Value?.ToString(), out var fSec))
                        ttlFailure = TimeSpan.FromSeconds(fSec);
                    else if ((k == "running_ttl_seconds" || k == "ttl_running_seconds" || k == "running") && double.TryParse(entry.Value?.ToString(), out var rSec))
                        ttlRunning = TimeSpan.FromSeconds(rSec);
                }
            }
            else if (cfg.TryGetValue("success_ttl_seconds", out var s) && double.TryParse(s?.ToString(), out var sSec))
            {
                ttlSuccess = TimeSpan.FromSeconds(sSec);
            }

            return new RedisStateStore(redisUrl, ttlSuccess, ttlFailure, ttlRunning, prefix);
        }

        return normalized switch
        {
            "postgres" or "postgresql" or "pgsql" => new RelationalSqlStateStore(new PostgresDialect(), GetDsn("POSTGRES_DSN")),
            "mysql" or "mariadb" => new RelationalSqlStateStore(new MySqlDialect(), GetDsn("MYSQL_DSN")),
            "sqlserver" or "mssql" or "tsql" => new RelationalSqlStateStore(new SqlServerDialect(), GetDsn("SQLSERVER_DSN")),
            "sqlite" or "sqlite3" => new RelationalSqlStateStore(new SqliteDialect(), GetDsn("SQLITE_PATH")),
            _ => new InMemoryStateStore()
        };
    }
}
