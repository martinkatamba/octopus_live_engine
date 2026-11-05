package octopus_live

import (
	"os"

	"github.com/gin-gonic/gin"
	"github.com/martinkatamba/octopus_live_engine/go/pkg"
	"github.com/martinkatamba/octopus_live_engine/go/pkg/domain/models"
	"github.com/martinkatamba/octopus_live_engine/go/pkg/domain/ports"
	"gopkg.in/yaml.v3"
)

type OctopusLiveEngine struct {
	*pkg.OctopusLiveEngine
	ginEngine *gin.Engine
}

func NewLiveEngine(apiKey ...string) *OctopusLiveEngine {
	key := ""
	if len(apiKey) > 0 {
		key = apiKey[0]
	}
	base := pkg.NewOctopusLiveEngine(key)

	return &OctopusLiveEngine{
		OctopusLiveEngine: base,
	}
}

func (e *OctopusLiveEngine) LoadBlueprintFile(filePath string) error {
	data, err := os.ReadFile(filePath)
	if err != nil {
		return err
	}
	var bp models.WorkflowBlueprint
	if err := yaml.Unmarshal(data, &bp); err != nil {
		return err
	}
	return e.RegisterBlueprint(&bp)
}

func (e *OctopusLiveEngine) GetGinEngine() *gin.Engine {
	if e.ginEngine == nil {
		e.ginEngine = gin.Default()
		e.MountRoutes(e.ginEngine)
	}
	return e.ginEngine
}

func (e *OctopusLiveEngine) RegisterActivityFunc(name string, handler ports.ActivityFunc) {
	e.RegisterActivity(name, handler)
}
