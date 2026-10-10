package tests

import (
	"context"
	"encoding/json"
	"strings"
	"testing"

	"github.com/octopus-workflow/octopus_live_engine/go/pkg"
	"github.com/octopus-workflow/octopus_live_engine/go/pkg/domain/models"
)

// A false condition without else skips straight to the next step; the diagram
// must show that path (it used to draw only the TRUE edge, so the executed
// path looked broken), and conditions show as evaluated.
func TestDiagramShowsSkippedBranch(t *testing.T) {
	ctx := context.Background()
	e := pkg.NewOctopusLiveEngine("")
	noop := func(context.Context, map[string]interface{}) (map[string]interface{}, error) {
		return map[string]interface{}{"handled": true}, nil
	}
	e.RegisterActivity("Find", noop)
	e.RegisterActivity("Check", noop)
	e.RegisterActivity("Respond", noop)
	if err := e.RegisterBlueprint(&models.WorkflowBlueprint{WorkflowName: "cb", Version: "1.0.0", Steps: []models.WorkflowStep{
		{ID: "find", Activity: "Find"},
		{ID: "check_here", Condition: &models.StepCondition{Variable: "handled", Operator: "!=", Value: true},
			ThenSteps: []models.WorkflowStep{{ID: "check", Activity: "Check"}}},
		{ID: "respond", Activity: "Respond"},
	}}); err != nil {
		t.Fatal(err)
	}
	st, err := e.ExecuteSync(ctx, "cb", nil)
	if err != nil {
		t.Fatal(err)
	}
	jobID := st.JobID
	d, err := e.VisualizerService.CompileX6Diagram(ctx, "cb", &jobID, nil, nil)
	if err != nil {
		t.Fatal(err)
	}
	var skip, toCheck map[string]interface{}
	for _, edge := range d["edges"].([]map[string]interface{}) {
		switch {
		case edge["source"] == "node__check_here" && edge["target"] == "node__respond":
			skip = edge
		case edge["source"] == "node__check_here" && strings.HasSuffix(edge["target"].(string), "__check"):
			toCheck = edge
		}
	}
	if skip == nil {
		t.Fatal("no edge from the false condition to the next step")
	}
	b, _ := json.Marshal(skip)
	if !strings.Contains(string(b), "FALSE (Skip)") || !strings.Contains(string(b), `"stroke":"#10b981"`) {
		t.Fatalf("skip edge not labelled/highlighted as taken: %s", b)
	}
	b, _ = json.Marshal(toCheck)
	if strings.Contains(string(b), `"stroke":"#10b981"`) {
		t.Fatalf("untaken TRUE edge highlighted: %s", b)
	}
	for _, n := range d["nodes"].([]map[string]interface{}) {
		if n["id"] == "node__check_here" {
			b, _ := json.Marshal(n)
			if !strings.Contains(string(b), `"executed":true`) {
				t.Fatalf("condition not shown as evaluated: %s", b)
			}
		}
	}
}

// await_signal waits for a system event (e.g. a payment callback) and must not
// be drawn as a human approval; human_step is the step a person decides.
func TestDiagramTellsSignalWaitsFromHumanSteps(t *testing.T) {
	ctx := context.Background()
	e := pkg.NewOctopusLiveEngine("")
	if err := e.RegisterBlueprint(&models.WorkflowBlueprint{WorkflowName: "waits", Version: "1.0.0", Steps: []models.WorkflowStep{
		{ID: "wait_for_payment", AwaitSignal: "payment_result", TimeoutSeconds: 200},
		{ID: "manager_ok", HumanStep: "approval"},
	}}); err != nil {
		t.Fatal(err)
	}
	d, err := e.VisualizerService.CompileX6Diagram(ctx, "waits", nil, nil, nil)
	if err != nil {
		t.Fatal(err)
	}
	types := map[string]interface{}{}
	for _, n := range d["nodes"].([]map[string]interface{}) {
		if data, ok := n["data"].(map[string]interface{}); ok {
			types[n["id"].(string)] = data["type"]
		}
	}
	if types["node__wait_for_payment"] != "signal" || types["node__manager_ok"] != "human" {
		t.Fatalf("node types: %v", types)
	}
}
