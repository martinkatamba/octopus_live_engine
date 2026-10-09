package tests

import (
	"context"
	"encoding/json"
	"sync"
	"testing"
	"time"

	"github.com/octopus-workflow/octopus_live_engine/go/pkg"
	"github.com/octopus-workflow/octopus_live_engine/go/pkg/domain/models"
	"github.com/octopus-workflow/octopus_live_engine/go/pkg/domain/services"
)

// R1: stored state must not share memory with the running execution. Readers
// (visualizer, history, streams) serialise stored states while flows run;
// with a shared live pointer this is a data race ("concurrent map read and
// map write"). Run with -race.
func TestStoredStateIsIsolatedFromRunningExecution(t *testing.T) {
	engine := pkg.NewOctopusLiveEngine("")
	engine.RegisterActivity("Slow", func(ctx context.Context, in map[string]interface{}) (map[string]interface{}, error) {
		out := map[string]interface{}{}
		for i := 0; i < 50; i++ {
			out["k"+string(rune('a'+i%26))] = i
			time.Sleep(200 * time.Microsecond)
		}
		return out, nil
	})
	_ = engine.RegisterBlueprint(&models.WorkflowBlueprint{
		WorkflowName: "race", Version: "1.0.0",
		Steps: []models.WorkflowStep{{ID: "a", Activity: "Slow"}, {ID: "b", Activity: "Slow"}, {ID: "c", Activity: "Slow"}},
	})

	ctx := context.Background()
	stop := make(chan struct{})
	var readers sync.WaitGroup
	for r := 0; r < 4; r++ {
		readers.Add(1)
		go func() {
			defer readers.Done()
			for {
				select {
				case <-stop:
					return
				default:
				}
				states, _ := engine.ListExecutionStates(ctx, "race", 10)
				for _, st := range states {
					_, _ = json.Marshal(st)
				}
			}
		}()
	}
	var runs sync.WaitGroup
	for i := 0; i < 4; i++ {
		runs.Add(1)
		go func() {
			defer runs.Done()
			if _, err := engine.ExecuteSync(ctx, "race", map[string]interface{}{"n": 1}); err != nil {
				t.Error(err)
			}
		}()
	}
	runs.Wait()
	close(stop)
	readers.Wait()
}

// S7: stored state and traces mask declared fields, including fields of Go
// structs passed between steps.
func TestStoredStateMasksStructFields(t *testing.T) {
	type Account struct {
		Owner string
		PIN   string
	}
	engine := pkg.NewOctopusLiveEngine("")
	engine.RegisterActivity("Load", func(ctx context.Context, in map[string]interface{}) (map[string]interface{}, error) {
		return map[string]interface{}{"account": &Account{Owner: "grace", PIN: "2468"}}, nil
	})
	_ = engine.RegisterBlueprint(&models.WorkflowBlueprint{
		WorkflowName: "masking", Version: "1.0.0",
		Masking: &models.MaskingConfig{Enabled: true, Fields: []string{"pin", "phone"}},
		Steps:   []models.WorkflowStep{{ID: "load", Activity: "Load"}},
	})
	st, err := engine.ExecuteSync(context.Background(), "masking", map[string]interface{}{"phone": "0772123456"})
	if err != nil {
		t.Fatal(err)
	}
	stored, err := engine.GetExecutionState(context.Background(), st.JobID)
	if err != nil {
		t.Fatal(err)
	}
	b, _ := json.Marshal(stored)
	for _, secret := range []string{"2468", "0772123456"} {
		if json.Valid(b) && contains(string(b), secret) {
			t.Errorf("stored state leaks %q: %s", secret, b)
		}
	}
	if !contains(string(b), "grace") {
		t.Errorf("non-sensitive struct field lost: %s", b)
	}
}

func contains(s, sub string) bool {
	return len(sub) > 0 && len(s) >= len(sub) && (stringIndex(s, sub) >= 0)
}

func stringIndex(s, sub string) int {
	for i := 0; i+len(sub) <= len(s); i++ {
		if s[i:i+len(sub)] == sub {
			return i
		}
	}
	return -1
}

// The engine injects request fields again as "_injected_<field>": masking a
// field must cover that copy too (it reaches traces and the visualizer).
func TestMaskingCoversInjectedCopies(t *testing.T) {
	out := services.MaskPayload(map[string]interface{}{
		"pin": "1234", "_injected_pin": "1234", "_INJECTED_Api_Key": "k", "nested": map[string]interface{}{"_injected_pin": "1234"},
		"name": "ok",
	}, []string{"pin", "api_key"}, "")
	for _, k := range []string{"pin", "_injected_pin", "_INJECTED_Api_Key"} {
		if out[k] != "********" {
			t.Errorf("%s = %v, want masked", k, out[k])
		}
	}
	if out["nested"].(map[string]interface{})["_injected_pin"] != "********" || out["name"] != "ok" {
		t.Errorf("nested/unrelated: %v", out)
	}
}
