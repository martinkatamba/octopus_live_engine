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

type YamlBlueprintRepository struct {
	blueprints map[string]*models.WorkflowBlueprint
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

	for _, entry := range entries {
		if entry.IsDir() {
			continue
		}
		ext := strings.ToLower(filepath.Ext(entry.Name()))
		if ext == ".yaml" || ext == ".yml" {
			filePath := filepath.Join(dirPath, entry.Name())
			content, err := os.ReadFile(filePath)
			if err != nil {
				continue
			}
			var bp models.WorkflowBlueprint
			if err := yaml.Unmarshal(content, &bp); err == nil && bp.WorkflowName != "" {
				_ = r.SaveBlueprint(context.Background(), &bp)
			}
		}
	}
	return nil
}

func (r *YamlBlueprintRepository) SaveBlueprint(ctx context.Context, blueprint *models.WorkflowBlueprint) error {
	r.mu.Lock()
	defer r.mu.Unlock()
	if blueprint != nil {
		blueprint.NormalizeEndpoints()
		if len(blueprint.Steps) == 0 && len(blueprint.API) > 0 && len(blueprint.API[0].Steps) > 0 {
			blueprint.Steps = blueprint.API[0].Steps
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
	list := make([]models.WorkflowBlueprint, 0, len(r.blueprints))
	for _, bp := range r.blueprints {
		list = append(list, *bp)
	}
	return list, nil
}
