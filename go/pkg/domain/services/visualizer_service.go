package services

import (
	"context"
	"fmt"
	"path/filepath"
	"strings"

	"github.com/octopus-workflow/octopus_live_engine/go/pkg/domain/models"
	"github.com/octopus-workflow/octopus_live_engine/go/pkg/domain/ports"
)

type VisualizerService struct {
	blueprintRepo ports.IBlueprintRepository
	stateStore    ports.IStateStore
}

func NewVisualizerService(repo ports.IBlueprintRepository, store ports.IStateStore) *VisualizerService {
	return &VisualizerService{
		blueprintRepo: repo,
		stateStore:    store,
	}
}

func (v *VisualizerService) SetStateStore(store ports.IStateStore) {
	v.stateStore = store
}

func (v *VisualizerService) ListBlueprints(ctx context.Context) ([]models.WorkflowBlueprint, error) {
	return v.blueprintRepo.ListBlueprints(ctx)
}

type prevItem struct {
	ID       string
	Label    string
	Color    string
	Executed bool
}

type excConfigItem struct {
	Activity     string
	Exception    string
	Code         int
	IsAsync      bool
	IsMiddleware bool
	SourceStepID string
	SourceName   string
	Message      string
}

func (v *VisualizerService) CompileX6Diagram(
	ctx context.Context,
	workflowName string,
	jobID *string,
	route *string,
	method *string,
) (map[string]interface{}, error) {
	bp, err := v.blueprintRepo.GetBlueprint(ctx, workflowName)
	if err != nil {
		return nil, fmt.Errorf("blueprint '%s' not found: %w", workflowName, err)
	}

	var state *models.ExecutionState
	if jobID != nil && *jobID != "" {
		state, _ = v.stateStore.GetState(ctx, *jobID)
		if state == nil {
			if strings.HasPrefix(*jobID, "wf-"+workflowName+"-") {
				cleanID := (*jobID)[len("wf-"+workflowName+"-"):]
				state, _ = v.stateStore.GetState(ctx, cleanID)
			} else if strings.HasPrefix(*jobID, "wf-") {
				parts := strings.SplitN(*jobID, "-", 3)
				if len(parts) == 3 {
					state, _ = v.stateStore.GetState(ctx, parts[2])
				}
			}
		}
	}

	statusMap := make(map[string]string)
	durationMap := make(map[string]float64)
	inputMap := make(map[string]map[string]interface{})
	outputMap := make(map[string]map[string]interface{})
	errorMap := make(map[string]string)
	if state != nil {
		for _, trace := range state.Traces {
			if trace.StepID != "" {
				statusMap[trace.StepID] = trace.Status
				durationMap[trace.StepID] = trace.DurationMs
				if trace.InputPayload != nil {
					inputMap[trace.StepID] = trace.InputPayload
				}
				if trace.OutputPayload != nil {
					outputMap[trace.StepID] = trace.OutputPayload
				}
				if trace.ErrorMessage != "" {
					errorMap[trace.StepID] = trace.ErrorMessage
				}
			}
			if trace.ActivityName != "" {
				statusMap[trace.ActivityName] = trace.Status
				if trace.InputPayload != nil {
					inputMap[trace.ActivityName] = trace.InputPayload
				}
				if trace.OutputPayload != nil {
					outputMap[trace.ActivityName] = trace.OutputPayload
				}
			}
		}
	}

	routesList := make([]map[string]interface{}, 0, len(bp.API))
	for _, ep := range bp.API {
		m := ep.Method
		if m == "" {
			m = "POST"
		}
		routesList = append(routesList, map[string]interface{}{
			"route":       ep.Route,
			"method":      strings.ToUpper(m),
			"summary":     ep.Summary,
			"description": ep.Description,
			"step_count":  len(ep.Steps),
		})
	}

	var selectedEp *models.APIEndpointConfig
	var selectedRoute string
	var selectedMethod string

	// 1. If we have a live job execution state, match by state.Route / state.Method or trace step IDs
	if state != nil {
		if state.Route != "" && len(bp.API) > 0 {
			for i := range bp.API {
				ep := &bp.API[i]
				if strings.EqualFold(ep.Route, state.Route) || strings.HasSuffix(state.Route, ep.Route) || strings.HasSuffix(ep.Route, state.Route) {
					selectedEp = ep
					break
				}
			}
		}

		if selectedEp == nil && len(bp.API) > 0 && len(state.Traces) > 0 {
			traceStepIDs := make(map[string]bool)
			for _, t := range state.Traces {
				if t.StepID != "" {
					traceStepIDs[strings.ToLower(t.StepID)] = true
				}
				if t.ActivityName != "" {
					traceStepIDs[strings.ToLower(t.ActivityName)] = true
				}
			}
			for i := range bp.API {
				ep := &bp.API[i]
				for _, s := range ep.Steps {
					if traceStepIDs[strings.ToLower(s.ID)] || traceStepIDs[strings.ToLower(s.Activity)] {
						selectedEp = ep
						break
					}
				}
				if selectedEp != nil {
					break
				}
			}
		}
	}

	// 2. If no job match, check explicit requested route / method from query parameter
	if selectedEp == nil && route != nil && *route != "" && len(bp.API) > 0 {
		for i := range bp.API {
			ep := &bp.API[i]
			matchRoute := strings.EqualFold(ep.Route, *route) || strings.HasSuffix(*route, ep.Route) || strings.HasSuffix(ep.Route, *route)
			matchMethod := method == nil || *method == "" || strings.EqualFold(ep.Method, *method)
			if matchRoute && matchMethod {
				selectedEp = ep
				break
			}
		}
	}

	// 3. Resolve the step list to render
	var stepsToVisualize []models.WorkflowStep
	if selectedEp != nil && len(selectedEp.Steps) > 0 {
		stepsToVisualize = selectedEp.Steps
		selectedRoute = selectedEp.Route
		selectedMethod = selectedEp.Method
		if selectedMethod == "" {
			selectedMethod = "POST"
		}
		selectedMethod = strings.ToUpper(selectedMethod)
	} else if len(bp.Steps) > 0 {
		stepsToVisualize = bp.Steps
		selectedRoute = "default"
		selectedMethod = "EXECUTE"
	} else if len(bp.API) > 0 && len(bp.API[0].Steps) > 0 {
		selectedEp = &bp.API[0]
		stepsToVisualize = selectedEp.Steps
		selectedRoute = selectedEp.Route
		selectedMethod = selectedEp.Method
		if selectedMethod == "" {
			selectedMethod = "POST"
		}
		selectedMethod = strings.ToUpper(selectedMethod)
	} else {
		stepsToVisualize = make([]models.WorkflowStep, 0)
	}

	hasLiveExecution := state != nil
	startStatus := "PENDING"
	if hasLiveExecution {
		startStatus = "COMPLETED"
	}

	nodes := make([]map[string]interface{}, 0)
	edges := make([]map[string]interface{}, 0)

	// 1. Start Node
	startNode := map[string]interface{}{
		"id":     "node__start",
		"shape":  "octopus-start-node",
		"label":  "START",
		"width":  120,
		"height": 50,
		"data": map[string]interface{}{
			"step_id":  "start",
			"type":     "start",
			"title":    "Workflow Start",
			"status":   startStatus,
			"executed": hasLiveExecution,
		},
	}
	if hasLiveExecution {
		startNode["attrs"] = map[string]interface{}{
			"body": map[string]interface{}{
				"rx":          25,
				"ry":          25,
				"fill":        "#064e3b",
				"stroke":      "#10b981",
				"strokeWidth": 3,
				"filter":      "drop-shadow(0 0 14px rgba(16, 185, 129, 0.75))",
				"opacity":     1.0,
			},
			"label": map[string]interface{}{
				"fill":       "#6ee7b7",
				"fontSize":   13,
				"fontWeight": 700,
			},
		}
	}
	nodes = append(nodes, startNode)

	// 2. Ingress Middleware Guards Node (Filtered by route excludes / includes)
	var preMiddlewares []models.MiddlewareConfig
	var postMiddlewares []models.MiddlewareConfig
	for _, m := range bp.Middlewares {
		if isMiddlewareActiveForRoute(m, selectedRoute) {
			if strings.EqualFold(m.Type, "post-request") {
				postMiddlewares = append(postMiddlewares, m)
			} else {
				preMiddlewares = append(preMiddlewares, m)
			}
		}
	}

	hasIngressGuards := len(preMiddlewares) > 0
	hasEgressGuards := len(postMiddlewares) > 0
	var initialPrev []prevItem

	isIngressFailed := false
	if hasIngressGuards {
		mwExcNames := make(map[string]bool)
		for _, m := range preMiddlewares {
			for _, e := range m.Exceptions {
				if e.Exception != "" {
					mwExcNames[strings.ToLower(e.Exception)] = true
				}
			}
		}

		if state != nil && state.Status == models.StatusFailed {
			stateErr := ""
			if state.Error != nil {
				stateErr = strings.ToLower(*state.Error)
			}
			for me := range mwExcNames {
				if strings.Contains(stateErr, me) {
					isIngressFailed = true
					break
				}
			}
			if !isIngressFailed {
				for _, t := range state.Traces {
					tErr := strings.ToLower(t.ErrorMessage)
					for me := range mwExcNames {
						if strings.Contains(tErr, me) {
							isIngressFailed = true
							break
						}
					}
					if isIngressFailed {
						break
					}
				}
			}
		}

		ingressStatus := "PENDING"
		if isIngressFailed {
			ingressStatus = "FAILED"
		} else if hasLiveExecution {
			ingressStatus = "COMPLETED"
		}

		mwNamesList := make([]string, 0, len(preMiddlewares))
		for _, m := range preMiddlewares {
			if m.Name != "" {
				mwNamesList = append(mwNamesList, m.Name)
			}
		}

		ingressNode := map[string]interface{}{
			"id":     "node__ingress_guards",
			"shape":  "octopus-activity-node",
			"label":  "🛡️ Ingress Middleware Guards",
			"width":  220,
			"height": 55,
			"data": map[string]interface{}{
				"step_id":     "ingress_guards",
				"type":        "ingress_guards",
				"title":       "🛡️ Ingress Middleware Guards",
				"description": "Pre-execution security filters, JWT authentication, IP whitelisting, and rate-limiting guards executed prior to workflow pipeline dispatch.",
				"status":      ingressStatus,
				"executed":    hasLiveExecution,
				"middlewares": mwNamesList,
			},
		}

		if isIngressFailed {
			ingressNode["attrs"] = map[string]interface{}{
				"body": map[string]interface{}{
					"stroke":      "#ef4444",
					"strokeWidth": 3,
					"fill":        "#2d1215",
					"filter":      "drop-shadow(0 0 16px rgba(239, 68, 68, 0.75))",
					"opacity":     1.0,
				},
				"label": map[string]interface{}{
					"fill":       "#fca5a5",
					"fontWeight": 700,
				},
			}
		} else if hasLiveExecution {
			ingressNode["attrs"] = map[string]interface{}{
				"body": map[string]interface{}{
					"stroke":      "#10b981",
					"strokeWidth": 3,
					"fill":        "#0f2e22",
					"filter":      "drop-shadow(0 0 16px rgba(16, 185, 129, 0.75))",
					"opacity":     1.0,
				},
				"label": map[string]interface{}{
					"fill":       "#6ee7b7",
					"fontWeight": 600,
				},
			}
		}
		nodes = append(nodes, ingressNode)

		startToIngressStroke := "#6366f1"
		if isIngressFailed {
			startToIngressStroke = "#ef4444"
		} else if hasLiveExecution {
			startToIngressStroke = "#10b981"
		}

		startToIngressEdge := map[string]interface{}{
			"id":     "edge__node__start__to__node__ingress_guards",
			"source": "node__start",
			"target": "node__ingress_guards",
			"attrs": map[string]interface{}{
				"line": map[string]interface{}{
					"stroke":       startToIngressStroke,
					"strokeWidth":  2,
					"targetMarker": map[string]interface{}{"name": "block", "width": 8, "height": 6, "fill": startToIngressStroke},
				},
			},
		}
		edges = append(edges, startToIngressEdge)

		initialPrev = []prevItem{
			{
				ID:       "node__ingress_guards",
				Label:    "✅ Quota & Security OK",
				Color:    "#10b981",
				Executed: hasLiveExecution && !isIngressFailed,
			},
		}
	} else {
		initialPrev = []prevItem{{ID: "node__start", Color: "#6366f1", Executed: hasLiveExecution}}
	}

	var processStepList func(stepList []models.WorkflowStep, prevHeads []prevItem, scopePrefix string, isBranchActive bool) []prevItem
	processStepList = func(stepList []models.WorkflowStep, prevHeads []prevItem, scopePrefix string, isBranchActive bool) []prevItem {
		currPrev := prevHeads

		for idx, step := range stepList {
			rawStepID := step.ID
			if rawStepID == "" {
				rawStepID = fmt.Sprintf("step_%d", idx)
			}
			nodeID := fmt.Sprintf("node__%s", rawStepID)
			if scopePrefix != "" {
				nodeID = fmt.Sprintf("node__%s__%s", scopePrefix, rawStepID)
			}

			executed := false
			stepStatus := "PENDING"
			if st, ok := statusMap[rawStepID]; ok {
				if st == "ASYNC_DISPATCHED" || st == "BACKGROUND_DISPATCHED" || st == "DISPATCHED" {
					stepStatus = "COMPLETED"
					executed = true
				} else {
					stepStatus = st
					executed = (st == "COMPLETED" || st == "RUNNING" || st == "FAILED" || st == "ROLLED_BACK")
				}
			} else if step.Activity != "" && statusMap[step.Activity] != "" {
				st := statusMap[step.Activity]
				if st == "ASYNC_DISPATCHED" || st == "BACKGROUND_DISPATCHED" || st == "DISPATCHED" {
					stepStatus = "COMPLETED"
					executed = true
				} else {
					stepStatus = st
					executed = (st == "COMPLETED" || st == "RUNNING" || st == "FAILED" || st == "ROLLED_BACK")
				}
			} else if state != nil && state.CurrentStepID == rawStepID {
				stepStatus = "RUNNING"
				executed = true
			} else if hasLiveExecution {
				stepStatus = "SKIPPED"
			}

			isLoop := step.Loop != nil || step.ForEach != nil
			isAsync := step.Async || step.IsAsync
			hasBranches := len(step.GetThenSteps()) > 0 || len(step.GetElseSteps()) > 0 || step.BranchingStep != "" || step.Condition != nil || step.Conditions != nil

			shape := "octopus-activity-node"
			typeStr := "activity"
			nodeLabel := rawStepID

			if isLoop {
				shape = "octopus-loop-node"
				typeStr = "loop"
				nodeLabel = fmt.Sprintf("⚏ %s\n[Loop Iteration]", rawStepID)
			} else if isAsync {
				shape = "octopus-activity-node"
				typeStr = "async"
				nodeLabel = fmt.Sprintf("⚡ %s\n[Async Background]", rawStepID)
			} else if hasBranches {
				shape = "octopus-branch-node"
				typeStr = "branch"
				nodeLabel = fmt.Sprintf("⚏ %s", rawStepID)
			} else if step.HumanStep != "" {
				shape = "octopus-human-node"
				typeStr = "human"
			}

			nodeData := map[string]interface{}{
				"step_id":         rawStepID,
				"title":           rawStepID,
				"activity":        step.Activity,
				"status":          stepStatus,
				"executed":        executed,
				"type":            typeStr,
				"is_loop":         isLoop,
				"loop":            step.Loop,
				"for_each":        step.ForEach,
				"is_async":        isAsync,
				"condition":       step.Condition,
				"conditions":      step.Conditions,
				"compensation":    step.Compensation,
				"description":     step.Description,
				"timeout_seconds": step.TimeoutSeconds,
				"index":           idx + 1,
				"input_payload": func() map[string]interface{} {
					var raw map[string]interface{}
					if p, ok := inputMap[rawStepID]; ok {
						raw = p
					} else {
						raw = inputMap[step.Activity]
					}
					fields, maskVal := bp.GetMaskConfig(&step)
					return MaskPayload(raw, fields, maskVal)
				}(),
				"output_payload": func() map[string]interface{} {
					var raw map[string]interface{}
					if p, ok := outputMap[rawStepID]; ok {
						raw = p
					} else {
						raw = outputMap[step.Activity]
					}
					fields, maskVal := bp.GetMaskConfig(&step)
					return MaskPayload(raw, fields, maskVal)
				}(),
				"error":           errorMap[rawStepID],
			}

			if hasBranches {
				thenStepIds := make([]string, 0)
				for _, s := range step.GetThenSteps() {
					if s.ID != "" {
						thenStepIds = append(thenStepIds, s.ID)
					} else {
						thenStepIds = append(thenStepIds, s.Activity)
					}
				}
				elseStepIds := make([]string, 0)
				for _, s := range step.GetElseSteps() {
					if s.ID != "" {
						elseStepIds = append(elseStepIds, s.ID)
					} else {
						elseStepIds = append(elseStepIds, s.Activity)
					}
				}

				thenExecuted := false
				elseExecuted := false
				for _, ts := range step.GetThenSteps() {
					if statusMap[ts.ID] == "COMPLETED" || statusMap[ts.Activity] == "COMPLETED" || statusMap[ts.ID] == "ASYNC_DISPATCHED" || statusMap[ts.Activity] == "ASYNC_DISPATCHED" {
						thenExecuted = true
						break
					}
				}
				for _, es := range step.GetElseSteps() {
					if statusMap[es.ID] == "COMPLETED" || statusMap[es.Activity] == "COMPLETED" || statusMap[es.ID] == "ASYNC_DISPATCHED" || statusMap[es.Activity] == "ASYNC_DISPATCHED" {
						elseExecuted = true
						break
					}
				}

				if thenExecuted || elseExecuted {
					executed = true
					stepStatus = "COMPLETED"
					nodeData["executed"] = true
					nodeData["status"] = "COMPLETED"
				}

				branchInfo := map[string]interface{}{
					"has_branches":  true,
					"condition":     step.Condition,
					"conditions":    step.Conditions,
					"then_step_ids": thenStepIds,
					"else_step_ids": elseStepIds,
				}

				if hasLiveExecution && (thenExecuted || elseExecuted || executed) {
					evalInfo := map[string]interface{}{
						"evaluated": true,
						"verdict": func() string {
							if thenExecuted {
								return "TRUE"
							} else if elseExecuted {
								return "FALSE"
							}
							return "UNKNOWN"
						}(),
						"verdict_label": func() string {
							if thenExecuted {
								return "TRUE (Match)"
							} else if elseExecuted {
								return "FALSE (Else)"
							}
							return "EXECUTED"
						}(),
						"branch_taken": func() string {
							if thenExecuted {
								return "THEN"
							} else if elseExecuted {
								return "ELSE"
							}
							return "NONE"
						}(),
					}

					if step.Condition != nil {
						varName := step.Condition.Variable
						if varName == "" {
							varName = step.Condition.Field
						}
						if varName != "" {
							evalInfo["variable_name"] = varName
							evalInfo["runtime_value"] = v.extractRuntimeVariable(state, varName)
							evalInfo["expected_value"] = step.Condition.Value
							op := step.Condition.Operator
							if op == "" {
								op = "=="
							}
							evalInfo["operator"] = op
						}
						if step.Condition.Rule != "" {
							evalInfo["rule"] = step.Condition.Rule
						}
					}
					branchInfo["evaluation"] = evalInfo
				}
				nodeData["branch_info"] = branchInfo
			}

			node := map[string]interface{}{
				"id":     nodeID,
				"shape":  shape,
				"width":  220,
				"height": 75,
				"label":  nodeLabel,
				"data":   nodeData,
			}

			// Path Highlighting
			if stepStatus == "FAILED" || stepStatus == "ROLLED_BACK" {
				node["attrs"] = map[string]interface{}{
					"body": map[string]interface{}{
						"stroke":      "#ef4444",
						"strokeWidth": 3,
						"fill":        "#2d1215",
						"filter":      "drop-shadow(0 0 16px rgba(239, 68, 68, 0.75))",
						"opacity":     1.0,
					},
					"label": map[string]interface{}{
						"fill":       "#fca5a5",
						"fontWeight": 700,
					},
				}
			} else if stepStatus == "TIMED_OUT" {
				node["attrs"] = map[string]interface{}{
					"body": map[string]interface{}{
						"stroke":      "#f97316",
						"strokeWidth": 3,
						"fill":        "#2b180d",
						"filter":      "drop-shadow(0 0 16px rgba(249, 115, 22, 0.75))",
						"opacity":     1.0,
					},
					"label": map[string]interface{}{
						"fill":       "#fdba74",
						"fontWeight": 700,
					},
				}
			} else if stepStatus == "AWAITING_APPROVAL" || stepStatus == "RUNNING" || stepStatus == "AWAITING_SIGNAL" {
				node["attrs"] = map[string]interface{}{
					"body": map[string]interface{}{
						"stroke":      "#f59e0b",
						"strokeWidth": 3,
						"fill":        "#2a1d08",
						"filter":      "drop-shadow(0 0 14px rgba(245, 158, 11, 0.85))",
						"opacity":     1.0,
					},
					"label": map[string]interface{}{
						"fill":       "#fde68a",
						"fontWeight": 700,
					},
				}
			} else if executed {
				bodyAttrs := map[string]interface{}{
					"stroke":      "#10b981",
					"strokeWidth": 3,
					"fill":        "#0f2e22",
					"filter":      "drop-shadow(0 0 16px rgba(16, 185, 129, 0.75))",
					"opacity":     1.0,
				}
				if isLoop {
					bodyAttrs["strokeDasharray"] = "5 4"
				}
				node["attrs"] = map[string]interface{}{
					"body": bodyAttrs,
					"label": map[string]interface{}{
						"fill":       "#6ee7b7",
						"fontWeight": 600,
					},
				}
			} else if isLoop {
				node["attrs"] = map[string]interface{}{
					"body": map[string]interface{}{
						"stroke":          "#38bdf8",
						"strokeWidth":     2,
						"strokeDasharray": "5 4",
						"fill":            "#0c2333",
						"filter":          "drop-shadow(0 0 12px rgba(56, 189, 248, 0.45))",
						"opacity":         func() float64 { if hasLiveExecution { return 0.35 }; return 1.0 }(),
					},
					"label": map[string]interface{}{
						"fill":       "#7dd3fc",
						"fontWeight": 600,
						"opacity":    func() float64 { if hasLiveExecution { return 0.45 }; return 1.0 }(),
					},
				}
			} else if hasLiveExecution {
				node["attrs"] = map[string]interface{}{
					"body": map[string]interface{}{
						"stroke":      "#334155",
						"strokeWidth": 1.5,
						"fill":        "#111827",
						"opacity":     0.35,
					},
					"label": map[string]interface{}{
						"fill":    "#64748b",
						"opacity": 0.45,
					},
				}
			}

			nodes = append(nodes, node)

			// Connect predecessors to this node
			for _, p := range currPrev {
				var edgeAttrs map[string]interface{}
				if p.Executed && executed {
					edgeAttrs = map[string]interface{}{
						"line": map[string]interface{}{
							"stroke":       "#10b981",
							"strokeWidth":  3.5,
							"targetMarker": map[string]interface{}{"name": "block", "width": 10, "height": 8, "fill": "#10b981"},
							"filter":       "drop-shadow(0 0 8px rgba(16, 185, 129, 0.53))",
							"opacity":      1.0,
						},
					}
				} else if p.Executed && (stepStatus == "RUNNING" || stepStatus == "AWAITING_APPROVAL") {
					edgeAttrs = map[string]interface{}{
						"line": map[string]interface{}{
							"stroke":          "#f59e0b",
							"strokeWidth":     3.5,
							"strokeDasharray": "6 4",
							"targetMarker":    map[string]interface{}{"name": "block", "width": 10, "height": 8, "fill": "#f59e0b"},
							"filter":          "drop-shadow(0 0 10px rgba(245, 158, 11, 0.75))",
							"opacity":         1.0,
						},
					}
				} else if hasLiveExecution {
					edgeAttrs = map[string]interface{}{
						"line": map[string]interface{}{
							"stroke":          "#1e293b",
							"strokeWidth":     1.5,
							"strokeDasharray": "4 4",
							"targetMarker":    map[string]interface{}{"name": "block", "width": 6, "height": 4, "fill": "#334155"},
							"opacity":         0.25,
						},
					}
				} else {
					edgeAttrs = map[string]interface{}{
						"line": map[string]interface{}{
							"stroke":       "#64748b",
							"strokeWidth":  2,
							"targetMarker": map[string]interface{}{"name": "block", "width": 8, "height": 6, "fill": "#64748b"},
							"opacity":      1.0,
						},
					}
				}

				edgeObj := map[string]interface{}{
					"id":     fmt.Sprintf("edge_%s_to_%s", p.ID, nodeID),
					"source": p.ID,
					"target": nodeID,
					"attrs":  edgeAttrs,
				}
				if p.Label != "" {
					labelColor := p.Color
					if p.Executed && executed {
						labelColor = "#10b981"
					}
					edgeObj["labels"] = []map[string]interface{}{
						{
							"attrs": map[string]interface{}{
								"text": map[string]interface{}{
									"text":       p.Label,
									"fill":       labelColor,
									"fontSize":   11,
									"fontWeight": "bold",
								},
							},
						},
					}
				}
				edges = append(edges, edgeObj)
			}

			if hasBranches {
				var branchOuts []prevItem
				thenSteps := step.GetThenSteps()
				elseSteps := step.GetElseSteps()

				if len(thenSteps) > 0 {
					thenScope := fmt.Sprintf("%s_then", rawStepID)
					if scopePrefix != "" {
						thenScope = fmt.Sprintf("%s_%s_then", scopePrefix, rawStepID)
					}
					thenExecuted := false
					for _, ts := range thenSteps {
						if statusMap[ts.ID] == "COMPLETED" || statusMap[ts.Activity] == "COMPLETED" || statusMap[ts.ID] == "ASYNC_DISPATCHED" || statusMap[ts.Activity] == "ASYNC_DISPATCHED" {
							thenExecuted = true
							break
						}
					}
					thenHeads := []prevItem{{ID: nodeID, Label: "TRUE (Match)", Color: "#10b981", Executed: (executed && thenExecuted)}}
					thenEnds := processStepList(thenSteps, thenHeads, thenScope, isBranchActive)
					branchOuts = append(branchOuts, thenEnds...)
				}
				if len(elseSteps) > 0 {
					elseScope := fmt.Sprintf("%s_else", rawStepID)
					if scopePrefix != "" {
						elseScope = fmt.Sprintf("%s_%s_else", scopePrefix, rawStepID)
					}
					elseExecuted := false
					for _, es := range elseSteps {
						if statusMap[es.ID] == "COMPLETED" || statusMap[es.Activity] == "COMPLETED" || statusMap[es.ID] == "ASYNC_DISPATCHED" || statusMap[es.Activity] == "ASYNC_DISPATCHED" {
							elseExecuted = true
							break
						}
					}
					elseHeads := []prevItem{{ID: nodeID, Label: "FALSE (Else)", Color: "#38bdf8", Executed: (executed && elseExecuted)}}
					elseEnds := processStepList(elseSteps, elseHeads, elseScope, isBranchActive)
					branchOuts = append(branchOuts, elseEnds...)
				}
				currPrev = branchOuts
			} else {
				currPrev = []prevItem{{ID: nodeID, Color: "#64748b", Executed: executed}}
			}
		}

		return currPrev
	}

	finalHeads := processStepList(stepsToVisualize, initialPrev, "", true)

	// 3. Exception Handlers & Failure Terminal Nodes Collection
	var excConfigs []excConfigItem
	if bp.ErrorHandling != nil && bp.ErrorHandling.GlobalHandler != nil && bp.ErrorHandling.GlobalHandler.Activity != "" {
		gh := bp.ErrorHandling.GlobalHandler
		gCode := gh.Code
		if gCode <= 0 {
			gCode = 500
		}
		gMsg := gh.Message
		if gMsg == "" {
			gMsg = "Internal Server Error"
		}
		excConfigs = append(excConfigs, excConfigItem{
			Activity:     gh.Activity,
			Exception:    "*",
			Code:         gCode,
			IsAsync:      gh.IsAsync,
			IsMiddleware: false,
			SourceName:   "GlobalExceptionHandler",
			Message:      gMsg,
		})
	}

	for _, mw := range preMiddlewares {
		for _, exc := range mw.Exceptions {
			if exc.Activity != "" {
				excConfigs = append(excConfigs, excConfigItem{
					Activity:     exc.Activity,
					Exception:    exc.Exception,
					Code:         exc.Code,
					IsAsync:      exc.IsAsync,
					IsMiddleware: true,
					SourceName:   mw.Name,
					Message:      exc.Message,
				})
			}
		}
	}

	collectStepExceptions(stepsToVisualize, &excConfigs)

	if state != nil && len(state.Traces) > 0 {
		for _, trace := range state.Traces {
			sID := trace.StepID
			aName := trace.ActivityName
			isAlreadyPresent := false
			for _, e := range excConfigs {
				if strings.EqualFold(e.Activity, aName) {
					isAlreadyPresent = true
					break
				}
			}
			if (strings.HasPrefix(sID, "exc_") || strings.Contains(strings.ToLower(aName), "audit")) && !isAlreadyPresent {
				errM := trace.ErrorMessage
				if errM == "" {
					errM = "Exception"
				}
				excConfigs = append(excConfigs, excConfigItem{
					Activity:  aName,
					Exception: errM,
					Code:      400,
					IsAsync:   true,
				})
			}
		}
	}

	processedExcIDs := make(map[string]bool)
	for _, exc := range excConfigs {
		excName := exc.Exception
		if excName == "" {
			excName = "MiddlewareException"
		}
		excCode := exc.Code
		if excCode <= 0 {
			excCode = 400
		}
		excActName := exc.Activity
		isGlobalExc := excName == "*" || excCode == 500 || strings.Contains(strings.ToLower(excActName), "global")
		excNodeID := fmt.Sprintf("node__exc_%s", strings.ToLower(excName))
		if isGlobalExc {
			excNodeID = fmt.Sprintf("node__exc_%s", strings.ToLower(excActName))
		}

		srcID := "node__ingress_guards"
		if exc.SourceStepID != "" {
			srcID = "node__" + exc.SourceStepID
		} else if excName == "*" || excCode == 500 || strings.Contains(strings.ToLower(excActName), "global") {
			if state != nil {
				for _, t := range state.Traces {
					if t.Status == "FAILED" && !strings.HasPrefix(t.StepID, "exc_") {
						srcID = "node__" + t.StepID
						break
					}
				}
			}
			if srcID == "node__ingress_guards" && len(stepsToVisualize) > 0 {
				srcID = "node__" + stepsToVisualize[0].ID
			}
		} else if !hasIngressGuards {
			srcID = "node__start"
		}

		srcExists := false
		for _, n := range nodes {
			if n["id"] == srcID {
				srcExists = true
				break
			}
		}
		if !srcExists {
			if hasIngressGuards {
				srcID = "node__ingress_guards"
			} else {
				srcID = "node__start"
			}
		}

		if processedExcIDs[excNodeID] {
			continue
		}
		processedExcIDs[excNodeID] = true

		isExcExecuted := false
		if state != nil {
			sErr := ""
			if state.Error != nil {
				sErr = strings.ToLower(*state.Error)
			}
			excSrcStep := strings.ToLower(exc.SourceStepID)

			if isGlobalExc {
				if state.Status == models.StatusFailed {
					for _, t := range state.Traces {
						if strings.EqualFold(t.ActivityName, excActName) || strings.EqualFold(t.StepID, "exc_"+strings.ToLower(excActName)) {
							isExcExecuted = true
							break
						}
					}
				}
			} else {
				// 1. Check exact trace matching for this specific exception type or step
				for _, t := range state.Traces {
					tErr := strings.ToLower(t.ErrorMessage)
					tStep := strings.ToLower(t.StepID)
					var inpExc, inpErr, inpStep string
					var inpCode int
					if t.InputPayload != nil {
						if v, ok := t.InputPayload["exception"].(string); ok {
							inpExc = strings.ToLower(v)
						}
						if v, ok := t.InputPayload["error"].(string); ok {
							inpErr = strings.ToLower(v)
						}
						if v, ok := t.InputPayload["step_id"].(string); ok {
							inpStep = strings.ToLower(v)
						} else if v, ok := t.InputPayload["failing_step_id"].(string); ok {
							inpStep = strings.ToLower(v)
						}
						if v, ok := t.InputPayload["status_code"].(int); ok {
							inpCode = v
						} else if v, ok := t.InputPayload["status_code"].(float64); ok {
							inpCode = int(v)
						}
					}

					if excSrcStep != "" {
						// Step-Level Exception (e.g. step_verify_credentials -> InvalidCredentials)
						if strings.EqualFold(t.ActivityName, excActName) || tStep == "exc_"+excSrcStep+"_"+strings.ToLower(excName) || tStep == "exc_"+strings.ToLower(excActName) {
							if inpStep == excSrcStep || inpExc == strings.ToLower(excName) || inpErr == strings.ToLower(excName) || strings.Contains(inpExc, strings.ToLower(excName)) || (inpCode > 0 && inpCode == excCode) {
								isExcExecuted = true
								break
							}
						}
					} else {
						// Middleware Exception (e.g. Ingress Guards -> RateLimitExceeded, IpAccessDenied, UserNotAuthenticated)
						if strings.EqualFold(t.ActivityName, excActName) || tStep == "exc_"+strings.ToLower(excName) || tStep == "exc_"+strings.ToLower(excActName) {
							if inpStep != "" && inpStep != "ingress_guards" && inpStep != "egress_guards" && inpStep != "middleware" {
								// Triggered by a workflow step, so not this middleware exception
								continue
							}
							if inpExc == strings.ToLower(excName) || inpErr == strings.ToLower(excName) || strings.Contains(tErr, strings.ToLower(excName)) || tStep == "exc_"+strings.ToLower(excName) {
								isExcExecuted = true
								break
							}
						}
					}
				}

				// 2. Fallback check on state.Data or state.Error
				if !isExcExecuted {
					if state.Data != nil {
						if dExc, ok := state.Data["exception"].(string); ok && strings.EqualFold(dExc, excName) {
							isExcExecuted = true
						} else if dErr, ok := state.Data["error"].(string); ok && strings.EqualFold(dErr, excName) {
							isExcExecuted = true
						}
					}
					if !isExcExecuted && strings.Contains(sErr, strings.ToLower(excName)) {
						isExcExecuted = true
					}
				}
			}
		}

		excStatus := "PENDING"
		if isExcExecuted {
			excStatus = "COMPLETED"
		} else if hasLiveExecution {
			excStatus = "SKIPPED"
		}

		excEdgeLbl := fmt.Sprintf("On %s (%d)", excName, excCode)
		termLbl := fmt.Sprintf("🛑 FAILED (%d)", excCode)
		termID := fmt.Sprintf("node__failed_term_%d", excCode)

		excLower := strings.ToLower(excName)
		if excName == "*" || excCode == 500 || strings.Contains(strings.ToLower(excActName), "global") {
			excEdgeLbl = "💥 500 Unhandled Exception\n(Catch-All / Runtime Crash)"
			termLbl = "🛑 500 INTERNAL SERVER ERROR"
			termID = "node__failed_term_500"
		} else if strings.Contains(excLower, "rate") {
			excEdgeLbl = "⚠️ 429 RateLimitExceeded\n(Quota > 60 req/min)"
			termLbl = "🛑 429 TOO MANY REQUESTS"
			termID = "node__failed_term_429"
		} else if strings.Contains(excLower, "ip") || strings.Contains(excLower, "denied") {
			excEdgeLbl = "🚫 403 IpAccessDenied\n(Blocked IP / Untrusted Subnet)"
			termLbl = "🛑 403 FORBIDDEN"
			termID = "node__failed_term_403"
		} else if strings.Contains(excLower, "authorized") {
			excEdgeLbl = "🚫 403 UserNotAuthorized\n(Forbidden Role/Permissions)"
			termLbl = "🛑 403 FORBIDDEN"
			termID = "node__failed_term_403"
		} else if strings.Contains(excLower, "invalidcredentials") || strings.Contains(excLower, "credentials") {
			excEdgeLbl = "🔒 401 InvalidCredentials\n(Invalid Username/Password)"
			termLbl = "🛑 401 UNAUTHORIZED"
			termID = "node__failed_term_401"
		} else if strings.Contains(excLower, "authenticated") {
			excEdgeLbl = "🔒 401 UserNotAuthenticated\n(Missing/Invalid Token)"
			termLbl = "🛑 401 UNAUTHORIZED"
			termID = "node__failed_term_401"
		} else if excCode == 400 || strings.Contains(excLower, "badrequest") || strings.Contains(excLower, "validation") || strings.Contains(excLower, "malformed") || strings.Contains(excLower, "field") {
			excEdgeLbl = fmt.Sprintf("🛑 400 %s\n(Bad Request / Schema Violation)", excName)
			termLbl = "🛑 400 BAD REQUEST"
			termID = "node__failed_term_400"
		} else if excCode == 413 || strings.Contains(excLower, "toolarge") || strings.Contains(excLower, "payload") {
			excEdgeLbl = "🛑 413 PayloadTooLarge\n(Payload Size Exceeded)"
			termLbl = "🛑 413 PAYLOAD TOO LARGE"
			termID = "node__failed_term_413"
		}

		originName := "Workflow Exception Handler"
		desc := fmt.Sprintf("Invoked upon exception %s (%d).", excName, excCode)
		if exc.IsMiddleware {
			originName = fmt.Sprintf("Middleware Pipeline (%s)", exc.SourceName)
			desc = fmt.Sprintf("Invoked by %s upon %s (%d).", exc.SourceName, excName, excCode)
		} else if exc.SourceStepID != "" {
			originName = fmt.Sprintf("Workflow Step (%s)", exc.SourceStepID)
			desc = fmt.Sprintf("Invoked when step '%s' encounters exception %s (%d).", exc.SourceStepID, excName, excCode)
		}
		if exc.Message != "" {
			desc += " Policy: " + exc.Message
		}

		excNode := map[string]interface{}{
			"id":     excNodeID,
			"shape":  "octopus-activity-node",
			"label":  "🚨 " + excActName,
			"width":  220,
			"height": 55,
			"data": map[string]interface{}{
				"step_id":         "exc_" + strings.ToLower(excActName),
				"activity":        excActName,
				"type":            "exception_handler",
				"is_async":        exc.IsAsync,
				"exception_type":  excName,
				"status_code":     excCode,
				"source_type":     func() string { if exc.IsMiddleware { return "Middleware Exception Pipeline" }; return "Workflow Step Exception Handler" }(),
				"source_location": originName,
				"source_name":     exc.SourceName,
				"source_step_id":  exc.SourceStepID,
				"is_middleware":   exc.IsMiddleware,
				"title":           "🚨 Exception Handler: " + excActName,
				"description":     desc,
				"status":          excStatus,
				"executed":        isExcExecuted,
				"input_payload": func() map[string]interface{} {
					var raw map[string]interface{}
					if p, ok := inputMap["exc_"+strings.ToLower(excActName)]; ok {
						raw = p
					} else {
						raw = inputMap[excActName]
					}
					fields, maskVal := bp.GetMaskConfig(nil)
					return MaskPayload(raw, fields, maskVal)
				}(),
				"output_payload": func() map[string]interface{} {
					var raw map[string]interface{}
					if p, ok := outputMap["exc_"+strings.ToLower(excActName)]; ok {
						raw = p
					} else {
						raw = outputMap[excActName]
					}
					fields, maskVal := bp.GetMaskConfig(nil)
					return MaskPayload(raw, fields, maskVal)
				}(),
			},
		}

		if isExcExecuted {
			excNode["attrs"] = map[string]interface{}{
				"body": map[string]interface{}{
					"stroke":      "#10b981",
					"strokeWidth": 3,
					"fill":        "#0f2e22",
					"filter":      "drop-shadow(0 0 16px rgba(16, 185, 129, 0.75))",
					"opacity":     1.0,
				},
				"label": map[string]interface{}{
					"fill":       "#6ee7b7",
					"fontWeight": 600,
				},
			}
		} else if hasLiveExecution {
			excNode["attrs"] = map[string]interface{}{
				"body": map[string]interface{}{
					"stroke":      "#334155",
					"strokeWidth": 1.5,
					"fill":        "#111827",
					"opacity":     0.35,
				},
				"label": map[string]interface{}{
					"fill":    "#64748b",
					"opacity": 0.45,
				},
			}
		}
		nodes = append(nodes, excNode)

		strokeColor := "#f59e0b"
		strokeWidth := 1.5
		strokeDash := "5 5"
		opacity := 0.8
		markerColor := "#f59e0b"
		if isExcExecuted {
			strokeColor = "#ef4444"
			strokeWidth = 2.5
			opacity = 1.0
			markerColor = "#ef4444"
		} else if hasLiveExecution {
			strokeColor = "#1e293b"
			opacity = 0.25
			markerColor = "#334155"
		}

		textFill := "#fbbf24"
		rectFill := "#0f172a"
		rectStroke := "#f59e0b"
		labelOpacity := 1.0
		if isExcExecuted {
			textFill = "#f87171"
			rectFill = "#2d1215"
			rectStroke = "#ef4444"
		} else if hasLiveExecution {
			textFill = "#475569"
			rectFill = "#090d16"
			rectStroke = "#1e293b"
			labelOpacity = 0.3
		}

		excEdge := map[string]interface{}{
			"id":     fmt.Sprintf("edge__%s__to__%s", srcID, excNodeID),
			"source": srcID,
			"target": excNodeID,
			"attrs": map[string]interface{}{
				"line": map[string]interface{}{
					"stroke":          strokeColor,
					"strokeWidth":     strokeWidth,
					"strokeDasharray": strokeDash,
					"opacity":         opacity,
					"targetMarker":    map[string]interface{}{"name": "block", "width": 8, "height": 6, "fill": markerColor},
				},
			},
			"labels": []map[string]interface{}{
				{
					"position": 0.4,
					"attrs": map[string]interface{}{
						"text": map[string]interface{}{
							"text":       excEdgeLbl,
							"fill":       textFill,
							"fontSize":   10,
							"fontWeight": func() string { if isExcExecuted { return "bold" }; return "normal" }(),
							"opacity":    labelOpacity,
						},
						"rect": map[string]interface{}{
							"fill":        rectFill,
							"rx":          4,
							"ry":          4,
							"stroke":      rectStroke,
							"strokeWidth": 1,
							"opacity":     labelOpacity,
						},
					},
				},
			},
		}
		edges = append(edges, excEdge)

		// Create Terminal Failure Node if not already existing
		var existingTermNode map[string]interface{}
		for _, n := range nodes {
			if n["id"] == termID {
				existingTermNode = n
				break
			}
		}

		if existingTermNode == nil {
			failedTermStatus := "PENDING"
			if isExcExecuted {
				failedTermStatus = "FAILED"
			} else if hasLiveExecution {
				failedTermStatus = "SKIPPED"
			}
			termWidth := len(termLbl)*10 + 40
			if termWidth < 160 {
				termWidth = 160
			}

			termNode := map[string]interface{}{
				"id":     termID,
				"shape":  "octopus-end-node",
				"label":  termLbl,
				"width":  termWidth,
				"height": 50,
				"data": map[string]interface{}{
					"step_id":  "failed",
					"type":     "end_failed",
					"title":    "Failed: " + excName,
					"status":   failedTermStatus,
					"executed": isExcExecuted,
				},
			}
			if isExcExecuted {
				termNode["attrs"] = map[string]interface{}{
					"body": map[string]interface{}{
						"rx":          25,
						"ry":          25,
						"fill":        "#450a0a",
						"stroke":      "#ef4444",
						"strokeWidth": 3,
						"filter":      "drop-shadow(0 0 14px rgba(239, 68, 68, 0.75))",
						"opacity":     1.0,
					},
					"label": map[string]interface{}{
						"fill":       "#fca5a5",
						"fontSize":   13,
						"fontWeight": 700,
					},
				}
			} else if hasLiveExecution {
				termNode["attrs"] = map[string]interface{}{
					"body": map[string]interface{}{
						"rx":          25,
						"ry":          25,
						"fill":        "#0f172a",
						"stroke":      "#334155",
						"strokeWidth": 1.5,
						"opacity":     0.35,
					},
					"label": map[string]interface{}{
						"fill":       "#64748b",
						"fontSize":   13,
						"fontWeight": 600,
						"opacity":    0.45,
					},
				}
			}
			nodes = append(nodes, termNode)
		} else if isExcExecuted {
			if tData, ok := existingTermNode["data"].(map[string]interface{}); ok {
				tData["status"] = "FAILED"
				tData["executed"] = true
			}
			existingTermNode["attrs"] = map[string]interface{}{
				"body": map[string]interface{}{
					"rx":          25,
					"ry":          25,
					"fill":        "#450a0a",
					"stroke":      "#ef4444",
					"strokeWidth": 3,
					"filter":      "drop-shadow(0 0 14px rgba(239, 68, 68, 0.75))",
					"opacity":     1.0,
				},
				"label": map[string]interface{}{
					"fill":       "#fca5a5",
					"fontSize":   13,
					"fontWeight": 700,
				},
			}
		}

		excToTermEdge := map[string]interface{}{
			"id":     fmt.Sprintf("edge__%s__to__%s", excNodeID, termID),
			"source": excNodeID,
			"target": termID,
			"attrs": map[string]interface{}{
				"line": map[string]interface{}{
					"stroke":          strokeColor,
					"strokeWidth":     strokeWidth,
					"strokeDasharray": "5 5",
					"opacity":         opacity,
					"targetMarker":    map[string]interface{}{"name": "block", "width": 8, "height": 6, "fill": markerColor},
				},
			},
		}
		edges = append(edges, excToTermEdge)
	}

	// 4. Egress Post-Execution Middleware Guards Node
	if hasEgressGuards {
		hasPredExec := false
		for _, p := range finalHeads {
			if p.Executed {
				hasPredExec = true
				break
			}
		}
		egressExecuted := state != nil && state.Status == models.StatusCompleted && hasPredExec
		egressStatus := "PENDING"
		if egressExecuted {
			egressStatus = "COMPLETED"
		} else if hasLiveExecution {
			egressStatus = "SKIPPED"
		}

		postMwNamesList := make([]string, 0, len(postMiddlewares))
		for _, m := range postMiddlewares {
			if m.Name != "" {
				postMwNamesList = append(postMwNamesList, m.Name)
			}
		}

		egressNode := map[string]interface{}{
			"id":     "node__egress_guards",
			"shape":  "octopus-activity-node",
			"label":  "📊 Egress Middleware Guards",
			"width":  220,
			"height": 55,
			"data": map[string]interface{}{
				"step_id":     "egress_guards",
				"type":        "egress_guards",
				"title":       "📊 Egress Middleware Guards",
				"description": "Post-execution response auditing, latency logging, and metrics dispatch executed following workflow pipeline completion.",
				"status":      egressStatus,
				"executed":    egressExecuted,
				"middlewares": postMwNamesList,
			},
		}

		if egressExecuted {
			egressNode["attrs"] = map[string]interface{}{
				"body": map[string]interface{}{
					"stroke":      "#10b981",
					"strokeWidth": 3,
					"fill":        "#0f2e22",
					"filter":      "drop-shadow(0 0 16px rgba(16, 185, 129, 0.75))",
					"opacity":     1.0,
				},
				"label": map[string]interface{}{
					"fill":       "#6ee7b7",
					"fontWeight": 600,
				},
			}
		} else {
			opacityVal := 0.6
			labelFill := "#64748b"
			if hasLiveExecution {
				opacityVal = 0.35
				labelFill = "#475569"
			}
			egressNode["attrs"] = map[string]interface{}{
				"body": map[string]interface{}{
					"stroke":      "#334155",
					"strokeWidth": 1.5,
					"fill":        "#111827",
					"opacity":     opacityVal,
				},
				"label": map[string]interface{}{
					"fill":       labelFill,
					"fontWeight": 600,
				},
			}
		}
		nodes = append(nodes, egressNode)

		for _, p := range finalHeads {
			var edgeAttrs map[string]interface{}
			if p.Executed && egressExecuted {
				edgeAttrs = map[string]interface{}{
					"line": map[string]interface{}{
						"stroke":       "#10b981",
						"strokeWidth":  3.5,
						"targetMarker": map[string]interface{}{"name": "block", "width": 10, "height": 8, "fill": "#10b981"},
						"filter":       "drop-shadow(0 0 8px rgba(16, 185, 129, 0.53))",
						"opacity":      1.0,
					},
				}
			} else if hasLiveExecution {
				edgeAttrs = map[string]interface{}{
					"line": map[string]interface{}{
						"stroke":       "#334155",
						"strokeWidth":  1.5,
						"targetMarker": map[string]interface{}{"name": "block", "width": 6, "height": 5, "fill": "#334155"},
						"opacity":      0.35,
					},
				}
			} else {
				edgeAttrs = map[string]interface{}{
					"line": map[string]interface{}{
						"stroke":       "#6366f1",
						"strokeWidth":  2,
						"targetMarker": map[string]interface{}{"name": "block", "width": 8, "height": 6, "fill": "#6366f1"},
					},
				}
			}
			edges = append(edges, map[string]interface{}{
				"id":     fmt.Sprintf("edge__%s__to__node__egress_guards", p.ID),
				"source": p.ID,
				"target": "node__egress_guards",
				"attrs":  edgeAttrs,
			})
		}

		finalHeads = []prevItem{{ID: "node__egress_guards", Color: "#10b981", Executed: egressExecuted}}
	}

	// 5. End Node
	hasExecPred := false
	for _, p := range finalHeads {
		if p.Executed {
			hasExecPred = true
			break
		}
	}
	endExecuted := state != nil && state.Status == models.StatusCompleted && hasExecPred
	endStatus := "PENDING"
	if endExecuted {
		endStatus = "COMPLETED"
	} else if hasLiveExecution {
		endStatus = "SKIPPED"
	}

	var successStatusCode *int
	if selectedEp != nil && selectedEp.StatusCode > 0 {
		code := selectedEp.StatusCode
		successStatusCode = &code
	} else if len(bp.API) > 0 {
		code := 200
		successStatusCode = &code
	}

	endLabel := "✅ COMPLETED"
	endTitle := "Workflow Completed"
	if successStatusCode != nil {
		switch *successStatusCode {
		case 200:
			endLabel = "✅ 200 OK"
		case 201:
			endLabel = "✅ 201 CREATED"
		case 202:
			endLabel = "✅ 202 ACCEPTED"
		case 204:
			endLabel = "✅ 204 NO CONTENT"
		default:
			endLabel = fmt.Sprintf("✅ %d OK", *successStatusCode)
		}
		endTitle = fmt.Sprintf("Completed: %d", *successStatusCode)
	}

	endWidth := len(endLabel)*10 + 36
	if endWidth < 140 {
		endWidth = 140
	}

	endNode := map[string]interface{}{
		"id":     "node__end",
		"shape":  "octopus-end-node",
		"width":  endWidth,
		"height": 50,
		"label":  endLabel,
		"data": map[string]interface{}{
			"step_id":  "end",
			"type":     "end",
			"title":    endTitle,
			"status":   endStatus,
			"executed": endExecuted,
		},
	}
	if successStatusCode != nil {
		endNode["data"].(map[string]interface{})["status_code"] = *successStatusCode
	}

	if endExecuted {
		endNode["attrs"] = map[string]interface{}{
			"body": map[string]interface{}{
				"rx":          25,
				"ry":          25,
				"fill":        "#064e3b",
				"stroke":      "#10b981",
				"strokeWidth": 3,
				"filter":      "drop-shadow(0 0 14px rgba(16, 185, 129, 0.75))",
				"opacity":     1.0,
			},
			"label": map[string]interface{}{
				"fill":       "#6ee7b7",
				"fontSize":   13,
				"fontWeight": 700,
			},
		}
	} else {
		opacityVal := 0.6
		labelFill := "#64748b"
		if hasLiveExecution {
			opacityVal = 0.35
			labelFill = "#475569"
		}
		endNode["attrs"] = map[string]interface{}{
			"body": map[string]interface{}{
				"rx":          25,
				"ry":          25,
				"fill":        "#0f172a",
				"stroke":      "#334155",
				"strokeWidth": 1.5,
				"opacity":     opacityVal,
			},
			"label": map[string]interface{}{
				"fill":       labelFill,
				"fontSize":   13,
				"fontWeight": 600,
			},
		}
	}
	nodes = append(nodes, endNode)

	for _, p := range finalHeads {
		var edgeAttrs map[string]interface{}
		if p.Executed && endStatus == "COMPLETED" {
			edgeAttrs = map[string]interface{}{
				"line": map[string]interface{}{
					"stroke":       "#10b981",
					"strokeWidth":  3.5,
					"targetMarker": map[string]interface{}{"name": "block", "width": 10, "height": 8, "fill": "#10b981"},
					"filter":       "drop-shadow(0 0 8px rgba(16, 185, 129, 0.53))",
					"opacity":      1.0,
				},
			}
		} else if hasLiveExecution {
			edgeAttrs = map[string]interface{}{
				"line": map[string]interface{}{
					"stroke":          "#1e293b",
					"strokeWidth":     1.5,
					"strokeDasharray": "4 4",
					"targetMarker":    map[string]interface{}{"name": "block", "width": 6, "height": 4, "fill": "#334155"},
					"opacity":         0.25,
				},
			}
		} else {
			edgeAttrs = map[string]interface{}{
				"line": map[string]interface{}{
					"stroke":       "#64748b",
					"strokeWidth":  2,
					"targetMarker": map[string]interface{}{"name": "block", "width": 8, "height": 6, "fill": "#64748b"},
					"opacity":      1.0,
				},
			}
		}

		edges = append(edges, map[string]interface{}{
			"id":     fmt.Sprintf("edge_%s_to_node__end", p.ID),
			"source": p.ID,
			"target": "node__end",
			"attrs":  edgeAttrs,
		})
	}

	statusStr := "IDLE"
	overallStatusStr := ""
	if state != nil {
		statusStr = string(state.Status)
		overallStatusStr = string(state.Status)
	}

	jobExists := (jobID == nil || *jobID == "" || state != nil)

	apiList := make([]map[string]interface{}, 0, len(bp.API))
	for _, ep := range bp.API {
		apiList = append(apiList, map[string]interface{}{
			"route":        ep.Route,
			"method":       ep.Method,
			"input_model":  ep.InputModel,
			"output_model": ep.OutputModel,
			"status_code":  ep.StatusCode,
			"summary":      ep.Summary,
			"tags":         ep.Tags,
		})
	}

	result := map[string]interface{}{
		"workflow_name":  workflowName,
		"version":        bp.Version,
		"description":    bp.Description,
		"namespace":      bp.Namespace,
		"mode":           bp.Mode,
		"job_id":         func() string { if jobID != nil { return *jobID }; return "" }(),
		"job_exists":     jobExists,
		"status":         statusStr,
		"overall_status": overallStatusStr,
		"nodes":          nodes,
		"edges":          edges,
		"api":            apiList,
		"api_endpoints":  apiList,
		"routes":         routesList,
		"middlewares":    bp.Middlewares,
		"persistence":    bp.Persistence,
	}

	if selectedRoute != "" {
		result["selected_route"] = selectedRoute
	}
	if selectedMethod != "" {
		result["selected_method"] = selectedMethod
	}

	return result, nil
}

func collectStepExceptions(steps []models.WorkflowStep, excConfigs *[]excConfigItem) {
	if len(steps) == 0 {
		return
	}
	for _, step := range steps {
		sID := step.ID
		if sID == "" {
			sID = step.Activity
		}
		for _, exc := range step.Exceptions {
			if exc.Activity != "" {
				*excConfigs = append(*excConfigs, excConfigItem{
					Activity:     exc.Activity,
					Exception:    exc.Exception,
					Code:         exc.Code,
					IsAsync:      exc.IsAsync,
					IsMiddleware: false,
					SourceStepID: sID,
					SourceName:   sID,
					Message:      exc.Message,
				})
			}
		}
		if len(step.GetThenSteps()) > 0 {
			collectStepExceptions(step.GetThenSteps(), excConfigs)
		}
		if len(step.GetElseSteps()) > 0 {
			collectStepExceptions(step.GetElseSteps(), excConfigs)
		}
	}
}

func isPathExcluded(route string, excludes []string) bool {
	if route == "" || len(excludes) == 0 {
		return false
	}
	cleanRoute := strings.TrimRight(route, "/")
	for _, raw := range excludes {
		pattern := strings.TrimSpace(raw)
		if pattern == "" {
			continue
		}
		cleanPattern := strings.TrimRight(pattern, "/")

		if cleanRoute == cleanPattern {
			return true
		}

		if strings.HasSuffix(pattern, "*") {
			prefix := pattern[:len(pattern)-1]
			if strings.HasPrefix(route, prefix) || strings.Contains(route, prefix) {
				return true
			}
		}

		if strings.Contains(pattern, "*") || strings.Contains(pattern, "?") {
			if matched, _ := filepath.Match(pattern, route); matched {
				return true
			}
			if matched, _ := filepath.Match(cleanPattern, cleanRoute); matched {
				return true
			}
		}

		if strings.EqualFold(route, pattern) || strings.HasSuffix(route, pattern) || strings.HasSuffix(pattern, route) {
			return true
		}
	}
	return false
}

func isMiddlewareActiveForRoute(m models.MiddlewareConfig, route string) bool {
	if route == "" || strings.EqualFold(route, "default") {
		return true
	}
	if isPathExcluded(route, m.Excludes) {
		return false
	}
	if len(m.Includes) > 0 {
		for _, inc := range m.Includes {
			if strings.HasSuffix(inc, "*") {
				prefix := inc[:len(inc)-1]
				if strings.HasPrefix(route, prefix) || strings.Contains(route, prefix) {
					return true
				}
			}
			if strings.EqualFold(route, inc) || strings.HasSuffix(route, inc) || strings.HasSuffix(inc, route) {
				return true
			}
		}
		return false
	}
	return true
}

func (v *VisualizerService) extractRuntimeVariable(state *models.ExecutionState, varName string) interface{} {
	if state == nil || varName == "" {
		return nil
	}
	eval := NewConditionEvaluator()
	if state.Data != nil {
		if val := eval.getNestedField(state.Data, varName); val != nil {
			return val
		}
	}
	if state.Traces != nil {
		for i := len(state.Traces) - 1; i >= 0; i-- {
			t := state.Traces[i]
			if t.OutputPayload != nil {
				if val := eval.getNestedField(t.OutputPayload, varName); val != nil {
					return val
				}
			}
			if t.InputPayload != nil {
				if val := eval.getNestedField(t.InputPayload, varName); val != nil {
					return val
				}
			}
		}
	}
	return nil
}


