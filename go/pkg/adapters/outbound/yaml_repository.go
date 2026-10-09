package outbound

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"

	"github.com/octopus-workflow/octopus_live_engine/go/pkg/domain/models"
	"gopkg.in/yaml.v3"
)

// YamlBlueprintRepository keeps blueprints in registration order, so
// anything derived from the list (middleware mounting, routes) is
// deterministic.
type YamlBlueprintRepository struct {
	blueprints map[string]*models.WorkflowBlueprint
	order      []string
	mu         sync.RWMutex
}

func NewYamlBlueprintRepository() *YamlBlueprintRepository {
	return &YamlBlueprintRepository{
		blueprints: make(map[string]*models.WorkflowBlueprint),
	}
}

func (r *YamlBlueprintRepository) LoadFromDirectory(dirPath string) error {
	entries, err := os.ReadDir(dirPath)
	if err != nil {
		return err
	}

	for _, entry := range entries { // os.ReadDir sorts by name
		if entry.IsDir() {
			continue
		}
		ext := strings.ToLower(filepath.Ext(entry.Name()))
		if ext != ".yaml" && ext != ".yml" {
			continue
		}
		filePath := filepath.Join(dirPath, entry.Name())
		content, err := os.ReadFile(filePath)
		if err != nil {
			return fmt.Errorf("reading blueprint %s: %w", filePath, err)
		}
		var bp models.WorkflowBlueprint
		if err := yaml.Unmarshal(content, &bp); err != nil {
			return fmt.Errorf("parsing blueprint %s: %w", filePath, err)
		}
		if bp.WorkflowName == "" {
			return fmt.Errorf("blueprint %s: missing workflow_name", filePath)
		}
		if err := r.SaveBlueprint(context.Background(), &bp); err != nil {
			return err
		}
	}
	return nil
}

func (r *YamlBlueprintRepository) SaveBlueprint(ctx context.Context, blueprint *models.WorkflowBlueprint) error {
	r.mu.Lock()
	defer r.mu.Unlock()
	if blueprint != nil {
		if err := blueprint.Validate(); err != nil {
			return err
		}
		blueprint.NormalizeEndpoints()
		if len(blueprint.Steps) == 0 && len(blueprint.API) > 0 && len(blueprint.API[0].Steps) > 0 {
			blueprint.Steps = blueprint.API[0].Steps
		}
		if _, exists := r.blueprints[blueprint.WorkflowName]; !exists {
			r.order = append(r.order, blueprint.WorkflowName)
		}
		r.blueprints[blueprint.WorkflowName] = blueprint
	}
	return nil
}

func (r *YamlBlueprintRepository) GetBlueprint(ctx context.Context, workflowName string) (*models.WorkflowBlueprint, error) {
	r.mu.RLock()
	defer r.mu.RUnlock()
	bp, exists := r.blueprints[workflowName]
	if !exists {
		return nil, fmt.Errorf("blueprint '%s' not found", workflowName)
	}
	return bp, nil
}

func (r *YamlBlueprintRepository) ListBlueprints(ctx context.Context) ([]models.WorkflowBlueprint, error) {
	r.mu.RLock()
	defer r.mu.RUnlock()
	list := make([]models.WorkflowBlueprint, 0, len(r.order))
	for _, name := range r.order {
		list = append(list, *r.blueprints[name])
	}
	return list, nil
}
