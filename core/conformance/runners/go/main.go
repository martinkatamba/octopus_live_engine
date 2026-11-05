// Runs the conformance cases on the Go live engine and prints the results as JSON.
package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"sync"
	"time"

	octopus "github.com/martinkatamba/octopus_live_engine/go/pkg"
	"github.com/martinkatamba/octopus_live_engine/go/pkg/domain/models"
	"github.com/martinkatamba/octopus_live_engine/go/pkg/domain/services"
)

type entry struct {
	DelayMs    int                    `json:"delay_ms"`
	Fail       *string                `json:"fail"`
	ErrorType  string                 `json:"error_type"`
	ErrorBases []string               `json:"error_bases"`
	Echo       map[string]string      `json:"echo"`
	Return     map[string]interface{} `json:"return"`
}

type caseFile struct {
	Workflow     string             `json:"workflow"`
	SharedEngine bool               `json:"shared_engine"`
	Activities   map[string][]entry `json:"activities"`
	Runs         []struct {
		Input     map[string]interface{} `json:"input"`
		WaitAsync bool                   `json:"wait_async"`
		Events    bool                   `json:"events"`
		Diagram   bool                   `json:"diagram"`
	} `json:"runs"`
}

const asyncWait = 5 * time.Second

// storedState is the job as the state store holds it; with waitAsync, once
// no step is still ASYNC_DISPATCHED.
func storedState(engine *octopus.OctopusLiveEngine, jobID string, waitAsync bool) interface{} {
	deadline := time.Now().Add(asyncWait)
	for {
		st, err := engine.GetExecutionState(context.Background(), jobID)
		if err != nil || st == nil {
			return nil
		}
		pending := false
		for _, t := range st.Traces {
			pending = pending || t.Status == "ASYNC_DISPATCHED"
		}
		if !waitAsync || !pending || time.Now().After(deadline) {
			return st
		}
		time.Sleep(20 * time.Millisecond)
	}
}

func newEngine(dir string, c *caseFile, mu *sync.Mutex, calls map[string]int, sequence *[]string) (*octopus.OctopusLiveEngine, error) {
	engine := octopus.NewOctopusLiveEngine("")
	if err := engine.LoadBlueprintsFromDirectory(dir); err != nil {
		return nil, err
	}
	for name, script := range c.Activities {
		name, script := name, script
		engine.RegisterActivity(name, func(_ context.Context, input map[string]interface{}) (map[string]interface{}, error) {
			mu.Lock()
			n := calls[name]
			calls[name] = n + 1
			*sequence = append(*sequence, name)
			mu.Unlock()
			e := script[min(n, len(script)-1)]
			if e.DelayMs > 0 {
				time.Sleep(time.Duration(e.DelayMs) * time.Millisecond)
			}
			if e.Fail != nil {
				return nil, typedError(e)
			}
			out := map[string]interface{}{}
			for k, src := range e.Echo {
				out[k] = input[src]
			}
			for k, v := range e.Return {
				out[k] = v
			}
			return out, nil
		})
	}
	return engine, nil
}

// typedError is the script's failure. With error_type, it is an
// OctopusMiddlewareError whose Cause chain holds the error_bases (D14).
func typedError(e entry) error {
	if e.ErrorType == "" {
		return errors.New(*e.Fail)
	}
	var cause error
	for i := len(e.ErrorBases) - 1; i >= 0; i-- {
		cause = &services.OctopusMiddlewareError{Name: e.ErrorBases[i], Message: *e.Fail, Cause: cause}
	}
	return &services.OctopusMiddlewareError{Name: e.ErrorType, Message: *e.Fail, Cause: cause}
}

// diagram is the visualizer graph with nothing named: {"nodes": [node ids]}.
func diagram(engine *octopus.OctopusLiveEngine, workflow string) map[string]interface{} {
	ids := []interface{}{}
	d, err := engine.VisualizerService.CompileX6Diagram(context.Background(), workflow, nil, nil, nil)
	if err == nil {
		raw, _ := json.Marshal(d["nodes"])
		var nodes []map[string]interface{}
		_ = json.Unmarshal(raw, &nodes)
		for _, n := range nodes {
			ids = append(ids, n["id"])
		}
	}
	return map[string]interface{}{"nodes": ids}
}

func runCase(dir string) ([]map[string]interface{}, error) {
	raw, err := os.ReadFile(filepath.Join(dir, "case.json"))
	if err != nil {
		return nil, err
	}
	var c caseFile
	if err := json.Unmarshal(raw, &c); err != nil {
		return nil, err
	}
	var runs []map[string]interface{}
	var engine *octopus.OctopusLiveEngine
	var mu sync.Mutex
	calls := map[string]int{}
	sequence := []string{}
	for _, run := range c.Runs {
		if engine == nil || !c.SharedEngine {
			calls = map[string]int{}
			sequence = []string{}
			engine, err = newEngine(dir, &c, &mu, calls, &sequence)
			if err != nil {
				runs = append(runs, map[string]interface{}{"load_error": err.Error()})
				engine = nil
				continue
			}
		}
		if run.Diagram {
			runs = append(runs, map[string]interface{}{"diagram": diagram(engine, c.Workflow)})
			continue
		}
		input := map[string]interface{}{}
		for k, v := range run.Input {
			input[k] = v
		}
		var events <-chan models.LiveEvent
		if run.Events {
			jobID, _ := input["job_id"].(string)
			ch, unsubscribe, err := engine.StateStore.SubscribeEvents(context.Background(), jobID)
			if err == nil {
				events = ch
				defer unsubscribe()
			}
		}
		st, err := engine.ExecuteSync(context.Background(), c.Workflow, input)
		res := map[string]interface{}{"state": st}
		if events != nil {
			got := []interface{}{}
		drain:
			for {
				select {
				case ev := <-events:
					got = append(got, map[string]interface{}{"event": ev.Event, "data": ev.Data})
				default:
					break drain
				}
			}
			res["events"] = got
		}
		if err != nil {
			res["error"] = err.Error()
		}
		if st != nil && st.JobID != "" {
			res["stored"] = storedState(engine, st.JobID, run.WaitAsync)
		}
		mu.Lock()
		snapshot := map[string]int{}
		for k, v := range calls {
			snapshot[k] = v
		}
		res["sequence"] = append([]string{}, sequence...)
		mu.Unlock()
		res["calls"] = snapshot
		runs = append(runs, res)
	}
	return runs, nil
}

func main() {
	dir := filepath.Join("..", "..", "cases")
	if len(os.Args) > 1 {
		dir = os.Args[1]
	}
	entries, err := os.ReadDir(dir)
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	var names []string
	for _, e := range entries {
		if _, err := os.Stat(filepath.Join(dir, e.Name(), "case.json")); err == nil {
			names = append(names, e.Name())
		}
	}
	sort.Strings(names)
	cases := map[string]interface{}{}
	for _, name := range names {
		runs, err := runCase(filepath.Join(dir, name))
		if err != nil {
			runs = []map[string]interface{}{{"load_error": err.Error()}}
		}
		cases[name] = runs
	}
	out, _ := json.Marshal(map[string]interface{}{"engine": "go", "cases": cases})
	fmt.Println(string(out))
}
