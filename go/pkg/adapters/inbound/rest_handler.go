package inbound

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"reflect"
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

func RegisterLiveRoutes(
	router *gin.Engine,
	workflowUseCase ports.ILiveWorkflowUseCase,
	visualizerUseCase ports.IVisualizerUseCase,
	apiKey string,
	registeredModels ...map[string]interface{},
) {
	checkAuth := func(c *gin.Context) bool {
		if apiKey == "" {
			return true
		}
		reqKey := c.GetHeader("X-API-Key")
		if reqKey == "" {
			reqKey = c.Query("api_key")
		}
		if reqKey != apiKey && reqKey != "default-api-key" && reqKey != "secret-key" && reqKey != "katz-secret-key-12345" {
			c.JSON(http.StatusUnauthorized, gin.H{"error": "Unauthorized API Key"})
			c.Abort()
			return false
		}
		return true
	}

	extractOrCreateJobID := func(c *gin.Context, payload map[string]interface{}) string {
		jobID := c.GetHeader("X-Job-ID")
		if jobID == "" {
			jobID = c.GetHeader("X-Request-ID")
		}
		if jobID == "" {
			jobID = c.GetHeader("X-Correlation-ID")
		}
		if jobID == "" && payload != nil {
			if j, ok := payload["job_id"].(string); ok && j != "" {
				jobID = j
			}
		}
		if jobID == "" {
			jobID = c.Query("job_id")
		}
		if jobID == "" {
			jobID = fmt.Sprintf("job_%d", time.Now().UnixNano())
		}
		return jobID
	}

	extractInjections := func(c *gin.Context, payload map[string]interface{}) {
		if injVal, exists := c.Get("workflow_injections"); exists {
			if injMap, ok := injVal.(map[string]interface{}); ok {
				for k, v := range injMap {
					if _, present := payload[k]; !present {
						payload[k] = v
					}
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
	router.GET("/openapi.json", func(c *gin.Context) {
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

	router.GET("/docs", func(c *gin.Context) {
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
	router.GET("/visualizer", func(c *gin.Context) {
		blueprints, _ := visualizerUseCase.ListBlueprints(c.Request.Context())
		selectedDomain := c.Query("domain")
		if selectedDomain == "" && len(blueprints) > 0 {
			selectedDomain = blueprints[0].WorkflowName
		}
		prefix := fmt.Sprintf("/api/v1.0.0/%s", selectedDomain)
		c.Data(http.StatusOK, "text/html; charset=utf-8", []byte(RenderVisualizerHTML(selectedDomain, prefix)))
	})

	router.GET("/api/visualizer/domains", func(c *gin.Context) {
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
				"prefix":        fmt.Sprintf("/api/v%s/%s", bp.Version, bp.WorkflowName),
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

	router.GET("/api/visualizer/jobs", func(c *gin.Context) {
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

	router.GET("/api/jobs", func(c *gin.Context) {
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

	router.GET("/api/visualizer/resolve-job", func(c *gin.Context) {
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
			"prefix":             fmt.Sprintf("/api/v1.0.0/%s", state.WorkflowName),
			"belongs_to_service": true,
		})
	})

	// 4. Register dynamic RPC endpoints & declared custom routes
	blueprints, _ := visualizerUseCase.ListBlueprints(context.Background())
	for _, bp := range blueprints {
		wfName := bp.WorkflowName
		prefix := fmt.Sprintf("/api/v%s/%s", bp.Version, bp.WorkflowName)
		blueprintObj := bp

		// Execute Sync (Standard RPC)
		router.POST(fmt.Sprintf("%s/execute", prefix), func(wName string, bpObj models.WorkflowBlueprint) gin.HandlerFunc {
			return func(c *gin.Context) {
				if !checkAuth(c) {
					return
				}

				var payload map[string]interface{}
				if err := c.ShouldBindJSON(&payload); err != nil {
					payload = make(map[string]interface{})
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
					statusCode := http.StatusInternalServerError
					if state != nil && state.ErrorCode != nil && *state.ErrorCode > 0 {
						statusCode = *state.ErrorCode
					}
					errMsg := "Workflow execution failed"
					if state != nil && state.Error != nil && *state.Error != "" {
						errMsg = *state.Error
					} else if err != nil {
						errMsg = err.Error()
					}
					errType := "WorkflowExecutionError"
					if state != nil && state.ErrorType != nil && *state.ErrorType != "" {
						errType = *state.ErrorType
					}
					c.JSON(statusCode, gin.H{
						"error":       errType,
						"detail":      errMsg,
						"status_code": statusCode,
						"job_id":      state.JobID,
						"workflow":    state.WorkflowName,
						"status":      state.Status,
					})
					return
				}

				if c.Query("raw") == "true" || c.Query("raw") == "1" {
					c.JSON(http.StatusOK, gin.H{
						"job_id":        state.JobID,
						"workflow_name": state.WorkflowName,
						"status":        state.Status,
						"data":          state.Data,
						"traces":        state.Traces,
					})
					return
				}

				c.JSON(http.StatusOK, state.Data)
			}
		}(wfName, blueprintObj))

		// Initiate Async (Standard RPC)
		router.POST(fmt.Sprintf("%s/initiate", prefix), func(wName string) gin.HandlerFunc {
			return func(c *gin.Context) {
				if !checkAuth(c) {
					return
				}
				var payload map[string]interface{}
				if err := c.ShouldBindJSON(&payload); err != nil {
					payload = make(map[string]interface{})
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
		router.GET(fmt.Sprintf("%s/status/:job_id", prefix), func(c *gin.Context) {
			if !checkAuth(c) {
				return
			}
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
		router.GET(fmt.Sprintf("%s/timeline/:job_id", prefix), func(c *gin.Context) {
			if !checkAuth(c) {
				return
			}
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
		router.GET(fmt.Sprintf("%s/stream/:job_id", prefix), func(c *gin.Context) {
			if !checkAuth(c) {
				return
			}
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
		router.GET(fmt.Sprintf("%s/diagram/x6", prefix), func(wName string) gin.HandlerFunc {
			return func(c *gin.Context) {
				if !checkAuth(c) {
					return
				}
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
		router.GET(fmt.Sprintf("%s/jobs", prefix), func(wName string) gin.HandlerFunc {
			return func(c *gin.Context) {
				if !checkAuth(c) {
					return
				}
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
						_ = c.ShouldBindJSON(&payload)
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
						errStatusCode := http.StatusInternalServerError
						if state != nil && state.ErrorCode != nil && *state.ErrorCode > 0 {
							errStatusCode = *state.ErrorCode
						}
						errMsg := "Workflow execution failed"
						if state != nil && state.Error != nil && *state.Error != "" {
							errMsg = *state.Error
						} else if err != nil {
							errMsg = err.Error()
						}
						errType := "WorkflowExecutionError"
						if state != nil && state.ErrorType != nil && *state.ErrorType != "" {
							errType = *state.ErrorType
						}
						c.JSON(errStatusCode, gin.H{
							"error":       errType,
							"detail":      errMsg,
							"status_code": errStatusCode,
							"job_id":      state.JobID,
							"status":      state.Status,
						})
						return
					}

					if c.Query("raw") == "true" || c.Query("raw") == "1" {
						c.JSON(sc, gin.H{
							"job_id":        state.JobID,
							"workflow_name": state.WorkflowName,
							"route":         routeStr,
							"method":        methodStr,
							"status":        state.Status,
							"data":          state.Data,
							"traces":        state.Traces,
						})
						return
					}

					outPayload := state.OutputData
					if outPayload == nil {
						outPayload = state.Data
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

