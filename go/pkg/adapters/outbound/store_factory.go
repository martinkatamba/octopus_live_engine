package outbound

import (
	"database/sql"
	"os"
	"strings"
	"time"

	_ "github.com/go-sql-driver/mysql"
	_ "github.com/lib/pq"
	"github.com/martinkatamba/octopus_live_engine/go/pkg/domain/ports"
	_ "github.com/microsoft/go-mssqldb"
)

// StoreFactory provides zero-import dynamic instantiation of state store adapters.
type StoreFactory struct{}

func NewStoreFactory() *StoreFactory {
	return &StoreFactory{}
}

func normalizeMySqlDSN(raw string) string {
	raw = strings.TrimSpace(raw)
	if strings.HasPrefix(raw, "mysql://") {
		trimmed := strings.TrimPrefix(raw, "mysql://")
		// format: user:pass@host:port/dbname
		parts := strings.SplitN(trimmed, "@", 2)
		if len(parts) == 2 {
			userPass := parts[0]
			hostAndRest := parts[1]
			slashIdx := strings.Index(hostAndRest, "/")
			if slashIdx != -1 {
				hostPort := hostAndRest[:slashIdx]
				dbAndParams := hostAndRest[slashIdx+1:]
				if !strings.Contains(dbAndParams, "parseTime=") {
					if strings.Contains(dbAndParams, "?") {
						dbAndParams += "&parseTime=true"
					} else {
						dbAndParams += "?parseTime=true"
					}
				}
				return userPass + "@tcp(" + hostPort + ")/" + dbAndParams
			}
		}
	}
	return raw
}

func normalizeSqlServerDSN(raw string) string {
	raw = strings.TrimSpace(raw)
	if raw == "" {
		return raw
	}
	if strings.HasPrefix(raw, "sqlserver://") || strings.HasPrefix(raw, "mssql://") {
		// Ensure trustServerCertificate=true or encrypt=disable is present if not specified
		if !strings.Contains(raw, "trustServerCertificate=") && !strings.Contains(raw, "encrypt=") {
			if strings.Contains(raw, "?") {
				raw += "&trustServerCertificate=true"
			} else {
				raw += "?trustServerCertificate=true"
			}
		}
	}
	return raw
}

// Create resolves and instantiates the appropriate IStateStore implementation.
func (f *StoreFactory) Create(storeType string, config map[string]interface{}) ports.IStateStore {
	st := strings.ToLower(strings.TrimSpace(storeType))

	if config == nil {
		config = make(map[string]interface{})
	}

	if st == "" {
		if val, ok := config["store"].(string); ok && val != "" {
			st = strings.ToLower(strings.TrimSpace(val))
		} else if val, ok := config["type"].(string); ok && val != "" {
			st = strings.ToLower(strings.TrimSpace(val))
		} else {
			st = "inmemory"
		}
	}

	getDSN := func(envKeys ...string) string {
		if val, ok := config["dsn"].(string); ok && val != "" {
			return val
		}
		if val, ok := config["url"].(string); ok && val != "" {
			return val
		}
		if val, ok := config["connection_string"].(string); ok && val != "" {
			return val
		}
		for _, k := range envKeys {
			if v := os.Getenv(k); v != "" {
				return v
			}
		}
		return ""
	}

	switch st {
	case "postgres", "postgresql", "pgsql":
		dsn := getDSN("POSTGRES_DSN")
		if dsn != "" {
			if db, err := sql.Open("postgres", dsn); err == nil {
				return NewRelationalStateStore(db, &PostgresDialect{})
			}
		}
		return NewRelationalStateStore(nil, &PostgresDialect{})
	case "mysql", "mariadb":
		dsn := getDSN("MYSQL_DSN")
		if dsn != "" {
			norm := normalizeMySqlDSN(dsn)
			if db, err := sql.Open("mysql", norm); err == nil {
				return NewRelationalStateStore(db, &MySqlDialect{})
			}
		}
		return NewRelationalStateStore(nil, &MySqlDialect{})
	case "sqlserver", "mssql", "tsql":
		dsn := getDSN("SQLSERVER_DSN", "MSSQL_DSN")
		if dsn != "" {
			norm := normalizeSqlServerDSN(dsn)
			if db, err := sql.Open("sqlserver", norm); err == nil {
				return NewRelationalStateStore(db, &SqlServerDialect{})
			}
		}
		return NewRelationalStateStore(nil, &SqlServerDialect{})
	case "redis", "redis_cluster":
		redisURL := getDSN("REDIS_URL")
		if redisURL == "" {
			redisURL = "redis://localhost:6379"
		}
		prefix := "octopus:live"
		if p, ok := config["prefix"].(string); ok && p != "" {
			prefix = p
		}
		ttlSuccess := 1 * time.Hour
		ttlFailure := 7 * 24 * time.Hour
		ttlRunning := 2 * time.Hour

		if retMap, ok := config["retention"].(map[string]interface{}); ok {
			if s, ok := retMap["success_ttl_seconds"].(int); ok && s > 0 {
				ttlSuccess = time.Duration(s) * time.Second
			} else if sf, ok := retMap["success_ttl_seconds"].(float64); ok && sf > 0 {
				ttlSuccess = time.Duration(sf) * time.Second
			}
			if f, ok := retMap["failure_ttl_seconds"].(int); ok && f > 0 {
				ttlFailure = time.Duration(f) * time.Second
			} else if ff, ok := retMap["failure_ttl_seconds"].(float64); ok && ff > 0 {
				ttlFailure = time.Duration(ff) * time.Second
			}
			if r, ok := retMap["running_ttl_seconds"].(int); ok && r > 0 {
				ttlRunning = time.Duration(r) * time.Second
			} else if rf, ok := retMap["running_ttl_seconds"].(float64); ok && rf > 0 {
				ttlRunning = time.Duration(rf) * time.Second
			}
		}

		if rStore, err := NewRedisStateStore(redisURL, ttlSuccess, ttlFailure, ttlRunning, prefix); err == nil {
			return rStore
		}
		return NewInMemoryStateStore()
	case "sqlite", "sqlite3":
		return NewRelationalStateStore(nil, &SqliteDialect{})
	case "inmemory", "memory":
		return NewInMemoryStateStore()
	default:
		return NewInMemoryStateStore()
	}
}

// CreateWithDB instantiates a RelationalStateStore with an active *sql.DB instance.
func (f *StoreFactory) CreateWithDB(db *sql.DB, storeType string) ports.IStateStore {
	st := strings.ToLower(strings.TrimSpace(storeType))
	var dialect SQLDialect

	switch st {
	case "postgres", "postgresql", "pgsql":
		dialect = &PostgresDialect{}
	case "mysql", "mariadb":
		dialect = &MySqlDialect{}
	case "sqlserver", "mssql", "tsql":
		dialect = &SqlServerDialect{}
	default:
		dialect = &SqliteDialect{}
	}

	return NewRelationalStateStore(db, dialect)
}
