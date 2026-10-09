package inbound

import (
	"context"
	"crypto/subtle"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"reflect"
	"regexp"
	"strconv"
	"strings"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/octopus-workflow/octopus_live_engine/go/pkg/domain/models"
	"github.com/octopus-workflow/octopus_live_engine/go/pkg/domain/ports"
	"github.com/octopus-workflow/octopus_live_engine/go/pkg/domain/services"
)

type EndpointExecutor interface {
	ExecuteEndpointSync(ctx context.Context, workflowName string, endpoint *models.APIEndpointConfig, route string, method string, payload map[string]interface{}) (*models.ExecutionState, error)
}

// ToolingAuthorizer decides whether a request may use the tooling routes
// (visualizer, execution history, live streams, diagrams, API docs).
type ToolingAuthorizer func(c *gin.Context) bool

// RouteConfig configures RegisterLiveRoutesWithConfig.
type RouteConfig struct {
	APIKey            string
	Options           models.EngineOptions
	ToolingAuthorizer ToolingAuthorizer
}

// RegisterLiveRoutes mounts the engine's routes with secure default options.
func RegisterLiveRoutes(
	router *gin.Engine,
	workflowUseCase ports.ILiveWorkflowUseCase,
	visualizerUseCase ports.IVisualizerUseCase,
	apiKey string,
	registeredModels ...map[string]interface{},
) {
	RegisterLiveRoutesWithConfig(router, workflowUseCase, visualizerUseCase, RouteConfig{APIKey: apiKey}, registeredModels...)
}

// clientJobIDPattern limits client-chosen job ids (when allowed) to safe characters.
var clientJobIDPattern = regexp.MustCompile(`^[A-Za-z0-9._:-]{1,128}$`)

func RegisterLiveRoutesWithConfig(
	router *gin.Engine,
	workflowUseCase ports.ILiveWorkflowUseCase,
	visualizerUseCase ports.IVisualizerUseCase,
	cfg RouteConfig,
	registeredModels ...map[string]interface{},
) {
	apiKey, opts := cfg.APIKey, cfg.Options

	// checkAuth requires the engine API key, if one is configured. Keys are
	// only read from the X-API-Key header unless AllowAPIKeyInQuery is set.
	checkAuth := func(c *gin.Context) bool {
		if apiKey == "" {
			return true
		}
		reqKey := c.GetHeader("X-API-Key")
		if reqKey == "" && opts.AllowAPIKeyInQuery {
			reqKey = c.Query("api_key")
		}
		if subtle.ConstantTimeCompare([]byte(reqKey), []byte(apiKey)) != 1 {
			c.AbortWithStatusJSON(http.StatusUnauthorized, gin.H{"error": "Unauthorized API Key"})
			return false
		}
		return true
	}

	// tooling guards the visualizer, history, streams, diagrams and docs.
	tooling := func(c *gin.Context) {
		deny := func(detail string) {
			c.AbortWithStatusJSON(http.StatusForbidden, gin.H{"error": "ToolingAccessDenied", "detail": detail})
		}
		switch {
		case opts.Tooling == models.ToolingDisabled:
			c.AbortWithStatus(http.StatusNotFound)
			return
		case cfg.ToolingAuthorizer != nil:
			if !cfg.ToolingAuthorizer(c) {
				if !c.IsAborted() {
					deny("Not allowed to use the engine tooling.")
				}
				return
			}
		case opts.Tooling == models.ToolingOpen:
		case opts.Tooling == models.ToolingAPIKey || apiKey != "":
			if apiKey == "" {
				deny("Tooling requires an API key, but none is configured.")
				return
			}
			if !checkAuth(c) {
				return
			}
		default: // local only
			if ip := net.ParseIP(c.ClientIP()); ip == nil || !ip.IsLoopback() {
				deny("The visualizer and execution history are only available from this machine. Configure an API key, a ToolingAuthorizer or EngineOptions.Tooling to allow remote access.")
				return
			}
		}
		c.Next()
	}

	extractOrCreateJobID := func(c *gin.Context, payload map[string]interface{}) string {
		if opts.AllowClientJobIDs {
			candidates := []string{c.GetHeader("X-Job-ID"), c.GetHeader("X-Request-ID"), c.GetHeader("X-Correlation-ID")}
			if payload != nil {
				if j, ok := payload["job_id"].(string); ok {
					candidates = append(candidates, j)
				}
			}
			candidates = append(candidates, c.Query("job_id"))
			for _, id := range candidates {
				if id != "" && clientJobIDPattern.MatchString(id) {
					return id
				}
			}
		}
		return services.NewJobID()
	}

	// extractInjections merges values set by trusted middlewares (e.g. the
	// authenticated user). They always win over request parameters, and
	// reserved keys can't be supplied by the client.
	extractInjections := func(c *gin.Context, payload map[string]interface{}) {
		for k := range payload {
			if strings.HasPrefix(k, "_injected_") || k == "_auth_claims" {
				delete(payload, k)
			}
		}
		if injVal, exists := c.Get("workflow_injections"); exists {
			if injMap, ok := injVal.(map[string]interface{}); ok {
				for k, v := range injMap {
					payload[k] = v
					payload["_injected_"+k] = v
				}
			}
		}
		if claimsVal, exists := c.Get("auth_claims"); exists {
			if claimsMap, ok := claimsVal.(map[string]interface{}); ok {
				payload["_auth_claims"] = claimsMap
			}
		}
	}

	// bindJSON reads a JSON body; an empty body is fine, malformed JSON is a 400.
	bindJSON := func(c *gin.Context, payload *map[string]interface{}) bool {
		err := c.ShouldBindJSON(payload)
		if err == nil || errors.Is(err, io.EOF) {
			if *payload == nil {
				*payload = make(map[string]interface{})
			}
			return true
		}
		if !strings.Contains(c.ContentType(), "json") {
			*payload = make(map[string]interface{})
			return true // non-JSON bodies are left to middlewares/injections
		}
		c.AbortWithStatusJSON(http.StatusBadRequest, gin.H{
			"error": "MalformedJsonException", "detail": "Malformed JSON syntax in request body.", "status_code": http.StatusBadRequest,
		})
		return false
	}

	// fail answers a failed execution. Errors the workflow mapped on purpose
	// (exceptions, OctopusMiddlewareError) keep their message; unexpected
	// errors get a generic message unless ExposeErrorDetails is set.
	fail := func(c *gin.Context, state *models.ExecutionState, err error) {
		code, errType, jobID := http.StatusInternalServerError, "WorkflowExecutionError", ""
		var status interface{}
		intentional := false
		if state != nil {
			jobID, status = state.JobID, state.Status
			if state.ErrorCode != nil && *state.ErrorCode > 0 {
				code = *state.ErrorCode
			}
			if state.ErrorType != nil && *state.ErrorType != "" {
				errType, intentional = *state.ErrorType, true
			}
		}
		msg := "An internal error occurred."
		if jobID != "" {
			msg = fmt.Sprintf("An internal error occurred (reference %s).", jobID)
		}
		switch {
		case (intentional || opts.ExposeErrorDetails) && state != nil && state.Error != nil && *state.Error != "":
			msg = *state.Error
		case opts.ExposeErrorDetails && err != nil:
			msg = err.Error()
		}
		c.JSON(code, gin.H{"error": errType, "detail": msg, "status_code": code, "job_id": jobID, "status": status})
	}

	// publicData is what /execute returns: masked, without injected values.
	publicData := func(bp models.WorkflowBlueprint, data map[string]interface{}) map[string]interface{} {
		fields, maskValue := bp.AllMaskFields()
		out := services.MaskPayload(data, fields, maskValue)
		for k := range out {
			if strings.HasPrefix(k, "_injected_") || k == "_auth_claims" {
				delete(out, k)
			}
		}
		return out
	}

	// 1. System Health
	router.GET("/health", func(c *gin.Context) {
		c.JSON(http.StatusOK, gin.H{
			"status":  "UP",
			"engine":  "OCTOPUS Live In-Memory Engine (Go)",
			"mode":    "sync",
			"version": "1.0.0",
		})
	})

	// 2. Swagger UI & OpenAPI Spec
	router.GET("/openapi.json", tooling, func(c *gin.Context) {
		blueprints, _ := visualizerUseCase.ListBlueprints(c.Request.Context())
		paths := make(map[string]map[string]interface{})

		allModels := make(map[string]interface{})
		for _, m := range registeredModels {
			for k, v := range m {
				allModels[k] = v
			}
		}

		schemas := make(map[string]interface{})
		for name, m := range allModels {
			var t reflect.Type
			if rt, ok := m.(reflect.Type); ok {
				t = rt
			} else if m != nil {
				t = reflect.TypeOf(m)
			}
			if t != nil {
				schemas[name] = generateOpenAPISchemaFromGoType(t, schemas)
			}
		}

		securitySchemes := make(map[string]interface{})
		var bearerExcludes []string
		var bearerIncludes []string
		hasBearer := false

		for _, bp := range blueprints {
			for _, mw := range bp.Middlewares {
				if strings.Contains(mw.Name, "Bearer") {
					hasBearer = true
					bearerExcludes = append(bearerExcludes, mw.Excludes...)
					bearerIncludes = append(bearerIncludes, mw.Includes...)
				}
			}
		}

		if hasBearer {
			securitySchemes["HTTPBearer"] = gin.H{
				"type":         "http",
				"scheme":       "bearer",
				"bearerFormat": "JWT",
				"description":  "Enter your Bearer / JWT token (e.g. obtained via /auth/login)",
			}
		}
		if apiKey != "" {
			securitySchemes["ApiKeyAuth"] = gin.H{
				"type":        "apiKey",
				"in":          "header",
				"name":        "X-API-Key",
				"description": "API Key header authentication",
			}
		}

		routeMatcher := &services.RouteMatcher{}
		for _, bp := range blueprints {
			for _, ep := range bp.API {
				methodLower := strings.ToLower(ep.Method)
				if paths[ep.Route] == nil {
					paths[ep.Route] = make(map[string]interface{})
				}

				// 1. Path & Header Parameters
				endpointParams := extractOpenAPIPathParams(ep.Route)
				endpointParams = append(endpointParams, gin.H{
					"name":        "X-Job-ID",
					"in":          "header",
					"required":    false,
					"description": "Optional custom Job ID for idempotency and live tracing",
					"schema":      gin.H{"type": "string"},
				})

				op := gin.H{
					"summary":     ep.Summary,
					"description": ep.Description,
					"tags":        ep.Tags,
					"parameters":  endpointParams,
				}

				// 2. Request Body
				if methodLower == "post" || methodLower == "put" || methodLower == "patch" {
					if ep.InputModel != "" {
						if _, ok := schemas[ep.InputModel]; !ok {
							schemas[ep.InputModel] = gin.H{
								"type":        "object",
								"description": fmt.Sprintf("Auto-declared schema for %s", ep.InputModel),
							}
						}
						op["requestBody"] = gin.H{
							"required":    true,
							"description": fmt.Sprintf("Request payload for %s", ep.InputModel),
							"content": gin.H{
								"application/json": gin.H{
									"schema": gin.H{
										"$ref": fmt.Sprintf("#/components/schemas/%s", ep.InputModel),
									},
								},
							},
						}
					}
				}

				// 3. Responses
				statusCode := ep.StatusCode
				if statusCode == 0 {
					statusCode = 200
				}
				respObj := gin.H{
					"description": "Successful operation",
				}
				if ep.OutputModel != "" {
					if _, ok := schemas[ep.OutputModel]; !ok {
						schemas[ep.OutputModel] = gin.H{
							"type":        "object",
							"description": fmt.Sprintf("Auto-declared schema for %s", ep.OutputModel),
						}
					}
					respObj["content"] = gin.H{
						"application/json": gin.H{
							"schema": gin.H{
								"$ref": fmt.Sprintf("#/components/schemas/%s", ep.OutputModel),
							},
						},
					}
				}
				op["responses"] = gin.H{
					fmt.Sprintf("%d", statusCode): respObj,
				}

				// 4. Security Requirements
				if hasBearer {
					if !routeMatcher.ShouldSkip(ep.Route, bearerExcludes, bearerIncludes) {
						op["security"] = []gin.H{
							{"HTTPBearer": []string{}},
						}
					} else {
						op["security"] = []gin.H{}
					}
				} else if apiKey != "" {
					op["security"] = []gin.H{
						{"ApiKeyAuth": []string{}},
					}
				}

				paths[ep.Route][methodLower] = op
			}
		}

		components := gin.H{
			"schemas": schemas,
		}
		if len(securitySchemes) > 0 {
			components["securitySchemes"] = securitySchemes
		}

		c.JSON(http.StatusOK, gin.H{
			"openapi": "3.0.0",
			"info": gin.H{
				"title":       "OCTOPUS Live Engine API",
				"version":     "1.0.0",
				"description": "Declarative Model-Driven In-Memory Synchronous API Framework",
			},
			"paths":      paths,
			"components": components,
		})
	})

	router.GET("/docs", tooling, func(c *gin.Context) {
		html := `<!DOCTYPE html>
<html>
<head>
    <title>OCTOPUS Live Engine API Docs</title>
    <link rel="stylesheet" type="text/css" href="https://unpkg.com/swagger-ui-dist@5/swagger-ui.css">
</head>
<body>
    <div id="swagger-ui"></div>
    <script src="https://unpkg.com/swagger-ui-dist@5/swagger-ui-bundle.js"></script>
    <script>
        window.onload = () => {
            window.ui = SwaggerUIBundle({
                url: '/openapi.json',
                dom_id: '#swagger-ui',
            });
        };
    </script>
</body>
</html>`
		c.Data(http.StatusOK, "text/html; charset=utf-8", []byte(html))
	})

	// 3. Visualizer UI
	router.GET("/visualizer", tooling, func(c *gin.Context) {
		blueprints, _ := visualizerUseCase.ListBlueprints(c.Request.Context())
		selectedDomain := c.Query("domain")
		if selectedDomain == "" && len(blueprints) > 0 {
			selectedDomain = blueprints[0].WorkflowName
		}
		prefix := (&models.WorkflowBlueprint{WorkflowName: selectedDomain}).APIPrefix()
		for i := range blueprints {
			if blueprints[i].WorkflowName == selectedDomain {
				prefix = blueprints[i].APIPrefix()
			}
		}
		c.Data(http.StatusOK, "text/html; charset=utf-8", []byte(RenderVisualizerHTML(selectedDomain, prefix)))
	})

	router.GET("/api/visualizer/domains", tooling, func(c *gin.Context) {
		blueprints, _ := visualizerUseCase.ListBlueprints(c.Request.Context())
		res := make([]gin.H, 0, len(blueprints))
		for _, bp := range blueprints {
			endpointsList := make([]gin.H, 0, len(bp.API))
			for _, ep := range bp.API {
				endpointsList = append(endpointsList, gin.H{
					"route":        ep.Route,
					"method":       ep.Method,
					"input_model":  ep.InputModel,
					"output_model": ep.OutputModel,
					"status_code":  ep.StatusCode,
					"summary":      ep.Summary,
					"tags":         ep.Tags,
				})
			}

			res = append(res, gin.H{
				"domain":        bp.WorkflowName,
				"namespace":     bp.Namespace,
				"prefix":        bp.APIPrefix(),
				"workflow_name": bp.WorkflowName,
				"version":       bp.Version,
				"mode":          bp.Mode,
				"description":   bp.Description,
				"endpoints":     endpointsList,
			})
		}
		c.JSON(http.StatusOK, res)
	})

	formatJobItem := func(s *models.ExecutionState) gin.H {
		statusStr := string(s.Status)
		isCompleted := s.Status == models.StatusCompleted
		isFailed := s.Status == models.StatusFailed || s.Status == models.StatusRolledBack
		statusCode := 200
		if isFailed {
			if s.ErrorCode != nil && *s.ErrorCode > 0 {
				statusCode = *s.ErrorCode
			} else {
				statusCode = 500
			}
		} else if !isCompleted {
			statusCode = 0
		}
		clientHost := s.ClientHost
		if clientHost == "" {
			if ch, ok := s.Data["client_host"].(string); ok && ch != "" {
				clientHost = ch
			} else if ci, ok := s.Data["client_ip"].(string); ok && ci != "" {
				clientHost = ci
			} else {
				clientHost = "127.0.0.1"
			}
		}
		method := s.Method
		if method == "" {
			method = "POST"
		}
		route := s.Route
		if route == "" {
			route = "-"
		}
		return gin.H{
			"job_id":        s.JobID,
			"workflow_name": s.WorkflowName,
			"workflow":      s.WorkflowName,
			"status":        statusStr,
			"status_code":   statusCode,
			"route":         route,
			"method":        strings.ToUpper(method),
			"source_ip":     clientHost,
			"client_host":   clientHost,
			"duration_ms":   s.DurationMs,
			"started_at":    s.StartedAt,
			"finished_at":   s.FinishedAt,
			"error":         s.Error,
			"error_type":    s.ErrorType,
		}
	}

	router.GET("/api/visualizer/jobs", tooling, func(c *gin.Context) {
		wf := c.Query("workflow")
		limit := 100
		if lStr := c.Query("limit"); lStr != "" {
			if parsed, err := strconv.Atoi(lStr); err == nil && parsed > 0 {
				limit = parsed
			}
		}
		states, err := workflowUseCase.ListExecutionStates(c.Request.Context(), wf, limit)
		if err != nil {
			c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
			return
		}
		res := make([]gin.H, 0, len(states))
		for _, s := range states {
			res = append(res, formatJobItem(s))
		}
		c.JSON(http.StatusOK, res)
	})

	router.GET("/api/jobs", tooling, func(c *gin.Context) {
		wf := c.Query("workflow")
		limit := 100
		if lStr := c.Query("limit"); lStr != "" {
			if parsed, err := strconv.Atoi(lStr); err == nil && parsed > 0 {
				limit = parsed
			}
		}
		states, err := workflowUseCase.ListExecutionStates(c.Request.Context(), wf, limit)
		if err != nil {
			c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
			return
		}
		res := make([]gin.H, 0, len(states))
		for _, s := range states {
			res = append(res, formatJobItem(s))
		}
		c.JSON(http.StatusOK, res)
	})

	prefixFor := func(workflow string) string {
		blueprints, _ := visualizerUseCase.ListBlueprints(context.Background())
		for i := range blueprints {
			if blueprints[i].WorkflowName == workflow {
				return blueprints[i].APIPrefix()
			}
		}
		return (&models.WorkflowBlueprint{WorkflowName: workflow}).APIPrefix()
	}

	router.GET("/api/visualizer/resolve-job", tooling, func(c *gin.Context) {
		jobID := c.Query("job_id")
		state, err := workflowUseCase.GetExecutionState(c.Request.Context(), jobID)
		if err != nil || state == nil {
			c.JSON(http.StatusOK, gin.H{"exists": false, "job_id": jobID})
			return
		}
		c.JSON(http.StatusOK, gin.H{
			"exists":             true,
			"job_id":             jobID,
			"domain":             state.WorkflowName,
			"status":             state.Status,
			"route":              state.Route,
			"method":             state.Method,
			"prefix":             prefixFor(state.WorkflowName),
			"belongs_to_service": true,
		})
	})

	// 4. Register dynamic RPC endpoints & declared custom routes
	blueprints, _ := visualizerUseCase.ListBlueprints(context.Background())
	for _, bp := range blueprints {
		wfName := bp.WorkflowName
		prefix := bp.APIPrefix()
		blueprintObj := bp

		// Execute Sync (Standard RPC)
		router.POST(fmt.Sprintf("%s/execute", prefix), func(wName string, bpObj models.WorkflowBlueprint) gin.HandlerFunc {
			return func(c *gin.Context) {
				if !checkAuth(c) {
					return
				}

				var payload map[string]interface{}
				if !bindJSON(c, &payload) {
					return
				}
				jobID := extractOrCreateJobID(c, payload)
				payload["job_id"] = jobID
				clientIP := c.ClientIP()
				if clientIP == "" {
					clientIP = "127.0.0.1"
				}
				payload["client_host"] = clientIP
				payload["client_ip"] = clientIP
				extractInjections(c, payload)

				startT := time.Now()
				state, err := workflowUseCase.ExecuteSync(c.Request.Context(), wName, payload)
				durationMs := float64(time.Since(startT).Microseconds()) / 1000.0

				if state != nil {
					c.Header("X-Job-ID", state.JobID)
					c.Header("X-Execution-Status", string(state.Status))
					c.Header("X-Execution-Duration-Ms", fmt.Sprintf("%.2f", durationMs))
				}

				if err != nil || (state != nil && (state.Status == models.StatusFailed || state.Status == models.StatusRolledBack)) {
					fail(c, state, err)
					return
				}

				if c.Query("raw") == "true" || c.Query("raw") == "1" {
					c.JSON(http.StatusOK, gin.H{
						"job_id":        state.JobID,
						"workflow_name": state.WorkflowName,
						"status":        state.Status,
						"data":          publicData(bpObj, state.Data),
						"traces":        state.Traces,
					})
					return
				}

				c.JSON(http.StatusOK, publicData(bpObj, state.Data))
			}
		}(wfName, blueprintObj))

		// Signal: resume a workflow paused at an await_signal step. It changes
		// a running workflow, so it is guarded like the tooling (the visualizer
		// approval UI uses it). Body = signal data (untrusted, under
		// signal.data); identity comes from trusted injections/claims only.
		router.POST(fmt.Sprintf("%s/signal/:job_id", prefix), tooling, func(c *gin.Context) {
			signaller, ok := workflowUseCase.(ports.ISignalUseCase)
			if !ok {
				c.JSON(http.StatusNotImplemented, gin.H{"error": "SignalsNotSupported"})
				return
			}
			var body map[string]interface{}
			if !bindJSON(c, &body) {
				return
			}
			name := c.Query("signal")
			if n, ok := body["signal"].(string); ok && name == "" {
				name = n
			}
			var by map[string]interface{}
			if claims, ok := c.Get("auth_claims"); ok {
				by, _ = claims.(map[string]interface{})
			}
			state, err := signaller.Signal(c.Request.Context(), c.Param("job_id"), name, body, by)
			switch {
			case errors.Is(err, services.ErrNoSuspension):
				c.JSON(http.StatusNotFound, gin.H{"error": "NoSuspension", "detail": err.Error()})
				return
			case errors.Is(err, services.ErrSignalMismatch):
				c.JSON(http.StatusConflict, gin.H{"error": "SignalMismatch", "detail": err.Error()})
				return
			case err != nil || (state != nil && (state.Status == models.StatusFailed || state.Status == models.StatusRolledBack)):
				fail(c, state, err)
				return
			}
			c.Header("X-Job-ID", state.JobID)
			c.Header("X-Execution-Status", string(state.Status))
			c.JSON(http.StatusOK, gin.H{"job_id": state.JobID, "status": state.Status, "output": publicData(blueprintObj, state.OutputData)})
		})

		// Initiate Async (Standard RPC)
		router.POST(fmt.Sprintf("%s/initiate", prefix), func(wName string) gin.HandlerFunc {
			return func(c *gin.Context) {
				if !checkAuth(c) {
					return
				}
				var payload map[string]interface{}
				if !bindJSON(c, &payload) {
					return
				}
				jobID := extractOrCreateJobID(c, payload)
				payload["job_id"] = jobID

				state, err := workflowUseCase.InitiateAsync(c.Request.Context(), wName, payload)
				if err != nil {
					c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
					return
				}
				c.Header("X-Job-ID", state.JobID)
				c.JSON(http.StatusOK, gin.H{
					"status":        "INITIATED",
					"job_id":        state.JobID,
					"workflow_name": state.WorkflowName,
				})
			}
		}(wfName))

		// Status
		router.GET(fmt.Sprintf("%s/status/:job_id", prefix), tooling, func(c *gin.Context) {
			jobID := c.Param("job_id")
			state, err := workflowUseCase.GetExecutionState(c.Request.Context(), jobID)
			if err != nil {
				c.JSON(http.StatusNotFound, gin.H{"error": fmt.Sprintf("Job '%s' not found", jobID)})
				return
			}
			c.JSON(http.StatusOK, gin.H{
				"job_id":          state.JobID,
				"workflow_name":   state.WorkflowName,
				"lifecycle_state": state.Status,
				"status":          state.Status,
				"route":           state.Route,
				"method":          state.Method,
				"current_step_id": state.CurrentStepID,
				"current_index":   state.CurrentIndex,
				"total_steps":     state.TotalSteps,
				"started_at":      state.StartedAt,
				"finished_at":     state.FinishedAt,
			})
		})

		// Timeline
		router.GET(fmt.Sprintf("%s/timeline/:job_id", prefix), tooling, func(c *gin.Context) {
			jobID := c.Param("job_id")
			state, err := workflowUseCase.GetExecutionState(c.Request.Context(), jobID)
			if err != nil {
				c.JSON(http.StatusNotFound, gin.H{"error": fmt.Sprintf("Job '%s' not found", jobID)})
				return
			}
			c.JSON(http.StatusOK, gin.H{
				"job_id":        state.JobID,
				"workflow_name": state.WorkflowName,
				"status":        state.Status,
				"route":         state.Route,
				"method":        state.Method,
				"data":          state.Data,
				"steps":         state.Traces,
			})
		})

		// SSE Stream
		router.GET(fmt.Sprintf("%s/stream/:job_id", prefix), tooling, func(c *gin.Context) {
			jobID := c.Param("job_id")
			ch, err := workflowUseCase.StreamEvents(c.Request.Context(), jobID)
			if err != nil {
				c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
				return
			}

			c.Stream(func(w io.Writer) bool {
				event, ok := <-ch
				if !ok {
					return false
				}
				dataBytes, _ := json.Marshal(event.Data)
				c.SSEvent(event.Event, string(dataBytes))
				return true
			})
		})

		// Diagram X6
		router.GET(fmt.Sprintf("%s/diagram/x6", prefix), tooling, func(wName string) gin.HandlerFunc {
			return func(c *gin.Context) {
				jobID := c.Query("job_id")
				var jobIDPtr *string
				if jobID != "" {
					jobIDPtr = &jobID
				}
				route := c.Query("route")
				var routePtr *string
				if route != "" {
					routePtr = &route
				}
				method := c.Query("method")
				var methodPtr *string
				if method != "" {
					methodPtr = &method
				}
				diagram, err := visualizerUseCase.CompileX6Diagram(c.Request.Context(), wName, jobIDPtr, routePtr, methodPtr)
				if err != nil {
					c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
					return
				}
				c.JSON(http.StatusOK, diagram)
			}
		}(wfName))

		// Jobs endpoint per workflow
		router.GET(fmt.Sprintf("%s/jobs", prefix), tooling, func(wName string) gin.HandlerFunc {
			return func(c *gin.Context) {
				limit := 100
				if lStr := c.Query("limit"); lStr != "" {
					if parsed, err := strconv.Atoi(lStr); err == nil && parsed > 0 {
						limit = parsed
					}
				}
				states, err := workflowUseCase.ListExecutionStates(c.Request.Context(), wName, limit)
				if err != nil {
					c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
					return
				}
				res := make([]gin.H, 0, len(states))
				for _, s := range states {
					res = append(res, formatJobItem(s))
				}
				c.JSON(http.StatusOK, res)
			}
		}(wfName))

		// 4B. Mount Custom Declared API Endpoints
		for _, ep := range bp.API {
			epCopy := ep
			epMethod := strings.ToUpper(epCopy.Method)
			if epMethod == "" {
				epMethod = "POST"
			}
			epRoute := epCopy.Route
			if !strings.HasPrefix(epRoute, "/") {
				epRoute = "/" + epRoute
			}
			ginRoute := convertOpenApiRouteToGin(epRoute)
			statusCode := epCopy.StatusCode
			if statusCode == 0 {
				statusCode = 200
			}

			router.Handle(epMethod, ginRoute, func(wName string, endpoint models.APIEndpointConfig, sc int, methodStr string, routeStr string) gin.HandlerFunc {
				return func(c *gin.Context) {
					if !checkAuth(c) {
						return
					}

					payload := make(map[string]interface{})
					if methodStr == "POST" || methodStr == "PUT" || methodStr == "PATCH" {
						if !bindJSON(c, &payload) {
							return
						}
					}

					// Ingest query params
					for k, v := range c.Request.URL.Query() {
						if len(v) > 0 {
							payload[k] = v[0]
						}
					}
					// Ingest path params
					for _, p := range c.Params {
						payload[p.Key] = p.Value
					}

					jobID := extractOrCreateJobID(c, payload)
					payload["job_id"] = jobID
					clientIP := c.ClientIP()
					if clientIP == "" {
						clientIP = "127.0.0.1"
					}
					payload["client_host"] = clientIP
					payload["client_ip"] = clientIP
					extractInjections(c, payload)

					startT := time.Now()
					var state *models.ExecutionState
					var err error

					if epExec, ok := workflowUseCase.(EndpointExecutor); ok {
						state, err = epExec.ExecuteEndpointSync(c.Request.Context(), wName, &endpoint, routeStr, methodStr, payload)
					} else {
						state, err = workflowUseCase.ExecuteSync(c.Request.Context(), wName, payload)
					}

					durationMs := float64(time.Since(startT).Microseconds()) / 1000.0

					if state != nil {
						c.Header("X-Job-ID", state.JobID)
						c.Header("X-Execution-Status", string(state.Status))
						c.Header("X-Execution-Duration-Ms", fmt.Sprintf("%.2f", durationMs))
					}

					if err != nil || (state != nil && (state.Status == models.StatusFailed || state.Status == models.StatusRolledBack)) {
						fail(c, state, err)
						return
					}

					if c.Query("raw") == "true" || c.Query("raw") == "1" {
						c.JSON(sc, gin.H{
							"job_id":        state.JobID,
							"workflow_name": state.WorkflowName,
							"route":         routeStr,
							"method":        methodStr,
							"status":        state.Status,
							"data":          publicData(blueprintObj, state.Data),
							"traces":        state.Traces,
						})
						return
					}

					outPayload := state.OutputData
					if outPayload == nil {
						outPayload = publicData(blueprintObj, state.Data)
					}
					c.JSON(sc, outPayload)
				}
			}(wfName, epCopy, statusCode, epMethod, epRoute))
		}
	}
}

func convertOpenApiRouteToGin(route string) string {
	var b strings.Builder
	inParam := false
	for i := 0; i < len(route); i++ {
		if route[i] == '{' {
			b.WriteByte(':')
			inParam = true
		} else if route[i] == '}' {
			inParam = false
		} else {
			b.WriteByte(route[i])
		}
	}
	_ = inParam
	return b.String()
}

func extractOpenAPIPathParams(route string) []gin.H {
	params := make([]gin.H, 0)
	parts := strings.Split(route, "/")
	for _, p := range parts {
		if strings.HasPrefix(p, "{") && strings.HasSuffix(p, "}") {
			name := strings.Trim(p, "{}")
			params = append(params, gin.H{
				"name":        name,
				"in":          "path",
				"required":    true,
				"description": fmt.Sprintf("Path parameter: %s", name),
				"schema":      gin.H{"type": "string"},
			})
		} else if strings.HasPrefix(p, ":") {
			name := strings.TrimPrefix(p, ":")
			params = append(params, gin.H{
				"name":        name,
				"in":          "path",
				"required":    true,
				"description": fmt.Sprintf("Path parameter: %s", name),
				"schema":      gin.H{"type": "string"},
			})
		}
	}
	return params
}

func generateOpenAPISchemaFromGoType(t reflect.Type, registeredSchemas map[string]interface{}) map[string]interface{} {
	if t == nil {
		return map[string]interface{}{"type": "object"}
	}
	for t.Kind() == reflect.Ptr {
		t = t.Elem()
	}

	switch t.Kind() {
	case reflect.Struct:
		props := make(map[string]interface{})
		required := make([]string, 0)

		for i := 0; i < t.NumField(); i++ {
			field := t.Field(i)
			if field.PkgPath != "" { // unexported
				continue
			}
			jsonTag := field.Tag.Get("json")
			if jsonTag == "-" {
				continue
			}
			fieldName := field.Name
			omitempty := false
			if jsonTag != "" {
				parts := strings.Split(jsonTag, ",")
				if parts[0] != "" {
					fieldName = parts[0]
				}
				for _, p := range parts[1:] {
					if p == "omitempty" {
						omitempty = true
					}
				}
			}

			fieldType := field.Type
			for fieldType.Kind() == reflect.Ptr {
				fieldType = fieldType.Elem()
			}

			var fieldSchema map[string]interface{}
			if fieldType.Kind() == reflect.Struct && fieldType.Name() != "" && fieldType.Name() != "Time" {
				if _, exists := registeredSchemas[fieldType.Name()]; !exists {
					registeredSchemas[fieldType.Name()] = generateOpenAPISchemaFromGoType(fieldType, registeredSchemas)
				}
				fieldSchema = map[string]interface{}{
					"$ref": fmt.Sprintf("#/components/schemas/%s", fieldType.Name()),
				}
			} else if fieldType.Kind() == reflect.Slice && fieldType.Elem().Kind() == reflect.Struct && fieldType.Elem().Name() != "" {
				elemType := fieldType.Elem()
				if _, exists := registeredSchemas[elemType.Name()]; !exists {
					registeredSchemas[elemType.Name()] = generateOpenAPISchemaFromGoType(elemType, registeredSchemas)
				}
				fieldSchema = map[string]interface{}{
					"type": "array",
					"items": map[string]interface{}{
						"$ref": fmt.Sprintf("#/components/schemas/%s", elemType.Name()),
					},
				}
			} else {
				fieldSchema = generateOpenAPISchemaFromGoType(fieldType, registeredSchemas)
			}

			if desc := field.Tag.Get("description"); desc != "" {
				fieldSchema["description"] = desc
			}
			if doc := field.Tag.Get("doc"); doc != "" {
				fieldSchema["description"] = doc
			}

			props[fieldName] = fieldSchema
			if !omitempty && field.Type.Kind() != reflect.Ptr && field.Type.Kind() != reflect.Slice && field.Type.Kind() != reflect.Map {
				required = append(required, fieldName)
			}
		}

		schema := map[string]interface{}{
			"type":       "object",
			"properties": props,
		}
		if len(required) > 0 {
			schema["required"] = required
		}
		return schema

	case reflect.Slice, reflect.Array:
		return map[string]interface{}{
			"type":  "array",
			"items": generateOpenAPISchemaFromGoType(t.Elem(), registeredSchemas),
		}

	case reflect.String:
		return map[string]interface{}{"type": "string"}

	case reflect.Int, reflect.Int8, reflect.Int16, reflect.Int32, reflect.Int64,
		reflect.Uint, reflect.Uint8, reflect.Uint16, reflect.Uint32, reflect.Uint64:
		return map[string]interface{}{"type": "integer"}

	case reflect.Float32, reflect.Float64:
		return map[string]interface{}{"type": "number"}

	case reflect.Bool:
		return map[string]interface{}{"type": "boolean"}

	case reflect.Map:
		return map[string]interface{}{"type": "object"}

	default:
		return map[string]interface{}{"type": "string"}
	}
}
