package pkg

import (
	"context"
	"fmt"
	"log"
	"os"
	"os/signal"
	"syscall"

	"github.com/gin-gonic/gin"
	"github.com/martinkatamba/octopus_live_engine/go/pkg/adapters/inbound"
	"github.com/martinkatamba/octopus_live_engine/go/pkg/adapters/outbound"
	"github.com/martinkatamba/octopus_live_engine/go/pkg/domain/models"
	"github.com/martinkatamba/octopus_live_engine/go/pkg/domain/ports"
	"github.com/martinkatamba/octopus_live_engine/go/pkg/domain/services"
)

type OctopusLiveEngine struct {
	BlueprintRepo            *outbound.YamlBlueprintRepository
	ActivityExecutor         *outbound.CallableActivityExecutor
	StateStore               ports.IStateStore
	PipelineManager          *services.PipelineManager
	VisualizerService        *services.VisualizerService
	ApiKey                   string
	AsyncDispatcher          ports.IAsyncActivityDispatcher
	models                   map[string]interface{}
	middlewares              []gin.HandlerFunc
	customMiddlewareRegistry map[string]services.CustomMiddlewareFactory
	options                  models.EngineOptions
	toolingAuthorizer        inbound.ToolingAuthorizer
}

// SetOptions sets security and runtime options (zero value = secure defaults).
// Call it before MountRoutes.
func (e *OctopusLiveEngine) SetOptions(opts models.EngineOptions) {
	e.options = opts
	e.PipelineManager.SetOptions(opts)
	if mem, ok := e.StateStore.(*outbound.InMemoryStateStore); ok {
		mem.SetLimit(opts.MaxStoredExecutions)
	}
}

func (e *OctopusLiveEngine) Options() models.EngineOptions { return e.options }

// Signal resumes a workflow paused at an await_signal step (see
// PipelineManager.Signal). Use it from activities or handlers, e.g. when a
// payment provider calls back.
func (e *OctopusLiveEngine) Signal(ctx context.Context, jobID, name string, data, by map[string]interface{}) (*models.ExecutionState, error) {
	return e.PipelineManager.Signal(ctx, jobID, name, data, by)
}

// SetSuspensionStore keeps paused workflows somewhere durable (default:
// in-memory); RestoreSuspensions re-arms their timeouts after a restart.
func (e *OctopusLiveEngine) SetSuspensionStore(store ports.ISuspensionStore) {
	e.PipelineManager.SetSuspensionStore(store)
}

func (e *OctopusLiveEngine) RestoreSuspensions(ctx context.Context) error {
	return e.PipelineManager.RestoreSuspensions(ctx)
}

// SetToolingAuthorizer lets the host decide who may use the visualizer,
// execution history, live streams, diagrams and docs (e.g. its own session
// and permission checks). It takes precedence over EngineOptions.Tooling.
func (e *OctopusLiveEngine) SetToolingAuthorizer(fn func(c *gin.Context) bool) {
	e.toolingAuthorizer = fn
}

func NewOctopusLiveEngine(apiKey string) *OctopusLiveEngine {
	repo := outbound.NewYamlBlueprintRepository()
	executor := outbound.NewCallableActivityExecutor()
	store := outbound.NewInMemoryStateStore()
	dispatcher := outbound.NewInMemoryAsyncActivityDispatcher()

	pipeline := services.NewPipelineManagerWithDispatcher(repo, executor, store, dispatcher)
	visualizer := services.NewVisualizerService(repo, store)

	return &OctopusLiveEngine{
		BlueprintRepo:            repo,
		ActivityExecutor:         executor,
		StateStore:               store,
		AsyncDispatcher:          dispatcher,
		PipelineManager:          pipeline,
		VisualizerService:        visualizer,
		ApiKey:                   apiKey,
		models:                   make(map[string]interface{}),
		middlewares:              make([]gin.HandlerFunc, 0),
		customMiddlewareRegistry: make(map[string]services.CustomMiddlewareFactory),
	}
}

func (e *OctopusLiveEngine) SetStateStore(store ports.IStateStore) {
	e.StateStore = store
	e.PipelineManager.SetStateStore(store)
	e.VisualizerService.SetStateStore(store)
}

func (e *OctopusLiveEngine) SetAsyncDispatcher(dispatcher ports.IAsyncActivityDispatcher) {
	e.AsyncDispatcher = dispatcher
	e.PipelineManager.SetAsyncDispatcher(dispatcher)
}

func (e *OctopusLiveEngine) RegisterModel(name string, model interface{}) {
	e.models[name] = model
}

func (e *OctopusLiveEngine) RegisterActivity(name string, handler ports.ActivityFunc) {
	e.ActivityExecutor.RegisterActivity(name, handler)
}

func (e *OctopusLiveEngine) RegisterCustomMiddleware(name string, factory services.CustomMiddlewareFactory) {
	e.customMiddlewareRegistry[name] = factory
}

func (e *OctopusLiveEngine) Use(mw ...gin.HandlerFunc) {
	e.middlewares = append(e.middlewares, mw...)
}

func (e *OctopusLiveEngine) AddMiddleware(mw gin.HandlerFunc) {
	e.middlewares = append(e.middlewares, mw)
}

func (e *OctopusLiveEngine) SetGlobalExceptionHandler(activity string, code int, message string, isAsync bool) {
	cfg := &models.GlobalExceptionHandlerConfig{
		Activity: activity,
		Code:     code,
		Message:  message,
		IsAsync:  isAsync,
	}
	blueprints, _ := e.BlueprintRepo.ListBlueprints(context.Background())
	for _, bp := range blueprints {
		bp.ErrorHandling = &models.ErrorHandlingConfig{GlobalHandler: cfg}
		bp.GlobalExceptionHandler = cfg
		_ = e.BlueprintRepo.SaveBlueprint(context.Background(), &bp)
	}
}

func (e *OctopusLiveEngine) RegisterBlueprint(bp *models.WorkflowBlueprint) error {
	e.configurePersistenceIfPresent(bp)
	return e.BlueprintRepo.SaveBlueprint(context.Background(), bp)
}

func (e *OctopusLiveEngine) LoadBlueprintsFromDirectory(dirPath string) error {
	if err := e.BlueprintRepo.LoadFromDirectory(dirPath); err != nil {
		return err
	}
	bps, _ := e.BlueprintRepo.ListBlueprints(context.Background())
	for _, bp := range bps {
		e.configurePersistenceIfPresent(&bp)
	}
	return nil
}

func (e *OctopusLiveEngine) configurePersistenceIfPresent(bp *models.WorkflowBlueprint) {
	if bp == nil || bp.Persistence == nil {
		return
	}
	factory := outbound.NewStoreFactory()
	storeType := ""
	if s, ok := bp.Persistence["store"].(string); ok {
		storeType = s
	}
	if storeType != "" && storeType != "inmemory" {
		st := factory.Create(storeType, bp.Persistence)
		if st != nil {
			e.SetStateStore(st)
		}
	}
	e.configureTemporalIfPresent(bp)
}

func (e *OctopusLiveEngine) configureTemporalIfPresent(bp *models.WorkflowBlueprint) {
	if bp == nil || bp.Persistence == nil {
		return
	}
	if tempRaw, ok := bp.Persistence["temporal"]; ok {
		if tempMap, ok := tempRaw.(map[string]interface{}); ok {
			host, _ := tempMap["host"].(string)
			ns, _ := tempMap["namespace"].(string)
			tq, _ := tempMap["task_queue"].(string)
			if e.AsyncDispatcher == nil || e.AsyncDispatcher.GetDispatcherType() == "IN_MEMORY" {
				temporalDisp := outbound.NewTemporalAsyncActivityDispatcher(host, ns, tq)
				e.SetAsyncDispatcher(temporalDisp)
			}
		}
	}
}

func (e *OctopusLiveEngine) StartEmbeddedWorker() error {
	if e.AsyncDispatcher != nil {
		if tempDisp, ok := e.AsyncDispatcher.(*outbound.TemporalAsyncActivityDispatcher); ok {
			return tempDisp.StartWorker(e.ActivityExecutor)
		}
	}
	return nil
}

func (e *OctopusLiveEngine) StopEmbeddedWorker() {
	if e.AsyncDispatcher != nil {
		if tempDisp, ok := e.AsyncDispatcher.(*outbound.TemporalAsyncActivityDispatcher); ok {
			tempDisp.StopWorker()
		}
	}
}

func (e *OctopusLiveEngine) StartWorker() error {
	if e.AsyncDispatcher == nil || e.AsyncDispatcher.GetDispatcherType() != "TEMPORAL" {
		return fmt.Errorf("cannot start standalone worker: Temporal persistence is not configured in loaded blueprints")
	}
	tempDisp, ok := e.AsyncDispatcher.(*outbound.TemporalAsyncActivityDispatcher)
	if !ok {
		return fmt.Errorf("unexpected dispatcher type: %T", e.AsyncDispatcher)
	}

	log.Printf("👷 Starting standalone Temporal Worker for TaskQueue: %s (Host: %s, Namespace: %s)...", tempDisp.TaskQueue, tempDisp.Host, tempDisp.Namespace)
	if err := tempDisp.StartWorker(e.ActivityExecutor); err != nil {
		return fmt.Errorf("failed to start standalone Temporal worker: %w", err)
	}

	sigChan := make(chan os.Signal, 1)
	signal.Notify(sigChan, os.Interrupt, syscall.SIGTERM)
	log.Printf("✨ Standalone Temporal Worker running. Press CTRL+C to terminate.")
	<-sigChan
	log.Printf("🛑 Shutting down standalone Temporal Worker...")
	tempDisp.StopWorker()
	return nil
}

func (e *OctopusLiveEngine) StartServer(port int) error {
	_ = e.StartEmbeddedWorker()
	defer e.StopEmbeddedWorker()

	router := gin.Default()
	e.MountRoutes(router)

	addr := fmt.Sprintf(":%d", port)
	log.Printf("🚀 Octopus Live Engine server listening on http://0.0.0.0%s", addr)
	return router.Run(addr)
}

func (e *OctopusLiveEngine) ExecuteSync(
	ctx context.Context,
	workflowName string,
	payload map[string]interface{},
) (*models.ExecutionState, error) {
	return e.PipelineManager.ExecuteSync(ctx, workflowName, payload)
}

func (e *OctopusLiveEngine) ExecuteEndpointSync(
	ctx context.Context,
	workflowName string,
	endpoint *models.APIEndpointConfig,
	route string,
	method string,
	payload map[string]interface{},
) (*models.ExecutionState, error) {
	return e.PipelineManager.ExecuteEndpointSync(ctx, workflowName, endpoint, route, method, payload)
}

func (e *OctopusLiveEngine) InitiateAsync(
	ctx context.Context,
	workflowName string,
	payload map[string]interface{},
) (*models.ExecutionState, error) {
	return e.PipelineManager.InitiateAsync(ctx, workflowName, payload)
}

func (e *OctopusLiveEngine) GetExecutionState(ctx context.Context, jobID string) (*models.ExecutionState, error) {
	return e.PipelineManager.GetExecutionState(ctx, jobID)
}

func (e *OctopusLiveEngine) ListExecutionStates(ctx context.Context, workflowName string, limit int) ([]*models.ExecutionState, error) {
	return e.PipelineManager.ListExecutionStates(ctx, workflowName, limit)
}

func (e *OctopusLiveEngine) MountRoutes(router *gin.Engine) {
	// 1. Resolve and mount declarative blueprint middlewares
	blueprints, _ := e.BlueprintRepo.ListBlueprints(context.Background())
	seenMiddlewares := make(map[string]bool)

	// Middlewares are global: they mount once, in blueprint registration
	// order then declaration order; a name declared again is ignored.
	for _, bp := range blueprints {
		for _, mwCfg := range bp.Middlewares {
			mwKey := mwCfg.Name
			if seenMiddlewares[mwKey] {
				log.Printf("⚠️ middleware %q declared again in blueprint %q is ignored (middlewares are global; declare each once)", mwKey, bp.WorkflowName)
				continue
			}
			if !seenMiddlewares[mwKey] {
				handler := services.BuildMiddlewareFromConfig(mwCfg, e.ActivityExecutor, e.AsyncDispatcher, e.customMiddlewareRegistry)
				if handler != nil {
					router.Use(handler)
					seenMiddlewares[mwKey] = true
				}
			}
		}
	}

	// 2. Mount custom programmatic middlewares
	for _, mw := range e.middlewares {
		router.Use(mw)
	}

	// 3. Only configured proxies may set X-Forwarded-For / X-Real-IP; by
	// default the TCP peer is the client, so IP-based rules can't be spoofed.
	if err := router.SetTrustedProxies(e.options.TrustedProxies); err != nil {
		log.Printf("⚠️ invalid TrustedProxies %v: %v (trusting no proxies)", e.options.TrustedProxies, err)
		_ = router.SetTrustedProxies(nil)
	}

	// 4. Register standard and declared routes
	inbound.RegisterLiveRoutesWithConfig(router, e.PipelineManager, e.VisualizerService, inbound.RouteConfig{
		APIKey: e.ApiKey, Options: e.options, ToolingAuthorizer: e.toolingAuthorizer,
	}, e.models)
}
