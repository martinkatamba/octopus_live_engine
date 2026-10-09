using Octopus.LiveEngine;
using Octopus.LiveEngine.Adapters.Outbound;
using Octopus.LiveEngine.Domain.Models;
using Xunit;

namespace Octopus.LiveEngine.Tests;

public class LiveEngineTests
{
    [Fact]
    public async Task TestDotnetLiveEngineSyncExecution()
    {
        var engine = new OctopusLiveEngine();

        engine.RegisterActivity("ValidateOrder", input =>
        {
            return new Dictionary<string, object>
            {
                ["is_valid"] = true,
                ["order_id"] = input.GetValueOrDefault("order_id") ?? "ORD-001"
            };
        });

        engine.RegisterActivity("CalculateTax", input =>
        {
            var amount = Convert.ToDouble(input.GetValueOrDefault("amount") ?? 100.0);
            var tax = amount * 0.15;
            return new Dictionary<string, object>
            {
                ["tax_amount"] = tax,
                ["total"] = amount + tax
            };
        });

        var blueprint = new WorkflowBlueprint
        {
            WorkflowName = "order_tax_pipeline",
            Version = "1.0.0",
            Mode = "sync",
            Steps = new List<WorkflowStep>
            {
                new() { Id = "validate", Activity = "ValidateOrder" },
                new() { Id = "calc_tax", Activity = "CalculateTax" }
            }
        };

        await engine.RegisterBlueprintAsync(blueprint);

        var state = await engine.ExecuteSyncAsync("order_tax_pipeline", new Dictionary<string, object>
        {
            ["order_id"] = "ORD-999",
            ["amount"] = 200.0
        });

        Assert.Equal(ExecutionStatus.COMPLETED, state.Status);
        Assert.Equal(2, state.Traces.Count);
        Assert.Equal(230.0, Convert.ToDouble(state.Data["total"]));

        // Test Visualizer Diagram
        var diagram = await engine.VisualizerService.CompileX6DiagramAsync("order_tax_pipeline", state.JobId);
        var nodes = Assert.IsAssignableFrom<IEnumerable<object>>(diagram["nodes"]);
        Assert.Equal(4, nodes.Count());
    }

    [Fact]
    public async Task TestDotnetLiveEngineLoopAndAsync()
    {
        var engine = new OctopusLiveEngine();

        bool asyncCalled = false;
        engine.RegisterActivity("ProcessItem", input =>
        {
            var item = input.GetValueOrDefault("item") as Dictionary<string, object>;
            var price = item != null && item.TryGetValue("price", out var p) ? Convert.ToDouble(p) : 0.0;
            return new Dictionary<string, object> { ["processed_price"] = price * 1.1 };
        });

        engine.RegisterActivity("AsyncAuditLog", input =>
        {
            asyncCalled = true;
            return new Dictionary<string, object> { ["audited"] = true };
        });

        var blueprint = new WorkflowBlueprint
        {
            WorkflowName = "batch_loop_pipeline",
            Version = "1.0.0",
            Mode = "sync",
            Steps = new List<WorkflowStep>
            {
                new()
                {
                    Id = "step_loop",
                    Activity = "ProcessItem",
                    Loop = new Dictionary<string, object>
                    {
                        ["items_field"] = "items",
                        ["item_variable"] = "item"
                    }
                },
                new()
                {
                    Id = "step_async_log",
                    Activity = "AsyncAuditLog",
                    IsAsync = true
                },
                new()
                {
                    Id = "step_settle",
                    Settle = true,
                    Reason = "Batch settled successfully"
                },
                new()
                {
                    Id = "step_unreachable",
                    Activity = "ProcessItem"
                }
            }
        };

        await engine.RegisterBlueprintAsync(blueprint);

        var state = await engine.ExecuteSyncAsync("batch_loop_pipeline", new Dictionary<string, object>
        {
            ["items"] = new List<object>
            {
                new Dictionary<string, object> { ["name"] = "Item A", ["price"] = 10.0 },
                new Dictionary<string, object> { ["name"] = "Item B", ["price"] = 20.0 }
            }
        });

        Assert.Equal(ExecutionStatus.COMPLETED, state.Status);
        Assert.True(state.Data.ContainsKey("step_loop_results"));
        var results = state.Data["step_loop_results"] as List<object>;
        Assert.NotNull(results);
        Assert.Equal(2, results.Count);
        Assert.DoesNotContain(state.Traces, t => t.StepId == "step_unreachable");
    }

    [Fact]
    public async Task TestDotnetLiveEngineSagaCompensation()
    {
        var engine = new OctopusLiveEngine();
        bool compensated = false;

        engine.RegisterActivity("ReserveFunds", input =>
        {
            return new Dictionary<string, object> { ["hold_id"] = "HOLD-999" };
        });

        engine.RegisterActivity("ReleaseFunds", input =>
        {
            compensated = true;
            return new Dictionary<string, object> { ["released"] = true };
        });

        engine.RegisterActivity("FailingActivity", input =>
        {
            throw new InvalidOperationException("External API unavailable");
        });

        var blueprint = new WorkflowBlueprint
        {
            WorkflowName = "saga_flow",
            CompensationMode = "sync",
            Steps = new List<WorkflowStep>
            {
                new()
                {
                    Id = "reserve",
                    Activity = "ReserveFunds",
                    Compensation = "ReleaseFunds"
                },
                new()
                {
                    Id = "failing_step",
                    Activity = "FailingActivity"
                }
            }
        };

        await engine.RegisterBlueprintAsync(blueprint);

        await Assert.ThrowsAsync<InvalidOperationException>(async () =>
        {
            await engine.ExecuteSyncAsync("saga_flow", new Dictionary<string, object> { ["amount"] = 500.0 });
        });

        Assert.True(compensated);
    }

    [Fact]
    public async Task TestDotnetDeclarativeMultiRouteAPI()
    {
        var engine = new OctopusLiveEngine();

        engine.RegisterActivity("CalculateSubtotal", input =>
        {
            return new Dictionary<string, object> { ["subtotal"] = 100.0 };
        });

        engine.RegisterActivity("CalculateTax", input =>
        {
            var amount = Convert.ToDouble(input.GetValueOrDefault("subtotal") ?? input.GetValueOrDefault("amount") ?? 100.0);
            var tax = amount * 0.15;
            return new Dictionary<string, object>
            {
                ["tax_amount"] = tax,
                ["total"] = amount + tax
            };
        });

        engine.RegisterActivity("PackageTaxResponse", input =>
        {
            return new Dictionary<string, object>
            {
                ["tax_total"] = input["tax_amount"],
                ["total"] = input["total"]
            };
        });

        var postEndpoint = new ApiEndpointConfig
        {
            Route = "/api/v1/checkout/tax",
            Method = "POST",
            InputModel = "CheckoutTaxRequest",
            OutputModel = "CheckoutTaxResponse",
            StatusCode = 200,
            Summary = "Calculate tax for cart",
            Tags = new List<string> { "Checkout" },
            Steps = new List<WorkflowStep>
            {
                new() { Id = "ep_subtotal", Activity = "CalculateSubtotal" },
                new() { Id = "ep_tax", Activity = "CalculateTax" },
                new() { Id = "ep_package", Activity = "PackageTaxResponse", IsOutput = true }
            }
        };

        var getEndpoint = new ApiEndpointConfig
        {
            Route = "/api/v1/checkout/quick-tax",
            Method = "GET",
            InputModel = "QuickTaxRequest",
            OutputModel = "CheckoutTaxResponse",
            StatusCode = 200,
            Summary = "Quick tax quote",
            Tags = new List<string> { "Checkout" },
            Steps = new List<WorkflowStep>
            {
                new() { Id = "quick_tax", Activity = "CalculateTax", IsOutput = true }
            }
        };

        var blueprint = new WorkflowBlueprint
        {
            WorkflowName = "ecommerce_tax_service",
            Version = "1.0.0",
            Mode = "sync",
            Steps = new List<WorkflowStep>
            {
                new() { Id = "def_subtotal", Activity = "CalculateSubtotal" },
                new() { Id = "def_tax", Activity = "CalculateTax" }
            },
            Api = new List<ApiEndpointConfig> { postEndpoint, getEndpoint }
        };

        await engine.RegisterBlueprintAsync(blueprint);

        // Test Endpoint Execution POST
        var postState = await engine.ExecuteEndpointSyncAsync(
            "ecommerce_tax_service",
            postEndpoint,
            "/api/v1/checkout/tax",
            "POST",
            new Dictionary<string, object> { ["job_id"] = "dotnet-tx-100", ["cart_id"] = "CART-01" }
        );

        Assert.Equal(ExecutionStatus.COMPLETED, postState.Status);
        Assert.Equal("dotnet-tx-100", postState.JobId);
        Assert.Equal("/api/v1/checkout/tax", postState.Route);
        Assert.Equal("POST", postState.Method);
        Assert.NotNull(postState.OutputData);
        Assert.Equal(115.0, Convert.ToDouble(postState.OutputData!["total"]));
        Assert.Equal(15.0, Convert.ToDouble(postState.OutputData!["tax_total"]));

        // Test Endpoint Execution GET
        var getState = await engine.ExecuteEndpointSyncAsync(
            "ecommerce_tax_service",
            getEndpoint,
            "/api/v1/checkout/quick-tax",
            "GET",
            new Dictionary<string, object> { ["amount"] = 200.0 }
        );

        Assert.Equal(ExecutionStatus.COMPLETED, getState.Status);
        Assert.StartsWith("job_", getState.JobId);
        Assert.Equal("/api/v1/checkout/quick-tax", getState.Route);
        Assert.Equal("GET", getState.Method);
        Assert.NotNull(getState.OutputData);
        Assert.Equal(230.0, Convert.ToDouble(getState.OutputData!["total"]));
    }

    [Fact]
    public async Task TestDotnetHybridPaymentFastPath()
    {
        var engine = new OctopusLiveEngine();

        engine.RegisterActivity("ScreenFraud", input =>
        {
            var amount = Convert.ToDouble(input.GetValueOrDefault("amount", 0.0));
            var score = amount < 1000 ? 0.02 : 0.85;
            return new Dictionary<string, object>
            {
                ["fraud_score"] = score,
                ["fraud_decision"] = score < 0.5 ? "APPROVE" : "REVIEW"
            };
        });

        engine.RegisterActivity("AuthorizeCard", input =>
        {
            return new Dictionary<string, object>
            {
                ["transaction_id"] = "txn_dotnet_7721",
                ["auth_code"] = "AUTH_9921",
                ["settled"] = true
            };
        });

        engine.RegisterActivity("FormatPaymentResponse", input =>
        {
            return new Dictionary<string, object>
            {
                ["transaction_id"] = input.GetValueOrDefault("transaction_id", "txn_unknown"),
                ["status"] = "AUTHORIZED",
                ["authorized_amount"] = Convert.ToDecimal(input.GetValueOrDefault("amount", 0.0)),
                ["currency"] = input.GetValueOrDefault("currency", "USD"),
                ["fraud_score"] = Convert.ToDouble(input.GetValueOrDefault("fraud_score", 0.0)),
                ["durable_tracking"] = "REPLICATED_TO_TEMPORAL_AND_SQLITE"
            };
        });

        var ep = new ApiEndpointConfig
        {
            Route = "/api/v1/payments/authorize",
            Method = "POST",
            InputModel = "PaymentRequest",
            OutputModel = "PaymentResponse",
            StatusCode = 200,
            Steps = new List<WorkflowStep>
            {
                new() { Id = "step_fraud", Activity = "ScreenFraud" },
                new() { Id = "step_auth", Activity = "AuthorizeCard" },
                new() { Id = "step_format", Activity = "FormatPaymentResponse", IsOutput = true }
            }
        };

        var bp = new WorkflowBlueprint
        {
            WorkflowName = "hybrid_payment_gateway",
            Version = "1.0.0",
            Mode = "hybrid",
            Api = new List<ApiEndpointConfig> { ep }
        };

        await engine.RegisterBlueprintAsync(bp);

        var state = await engine.ExecuteEndpointSyncAsync(
            "hybrid_payment_gateway",
            ep,
            "/api/v1/payments/authorize",
            "POST",
            new Dictionary<string, object>
            {
                ["job_id"] = "tx-dotnet-pay-001",
                ["amount"] = 350.0m,
                ["currency"] = "USD"
            }
        );

        Assert.Equal(ExecutionStatus.COMPLETED, state.Status);
        Assert.NotNull(state.OutputData);
        Assert.Equal("AUTHORIZED", state.OutputData!["status"]);
        Assert.Equal("REPLICATED_TO_TEMPORAL_AND_SQLITE", state.OutputData!["durable_tracking"]);
        Assert.Equal(3, state.Traces.Count);
    }

    [Fact]
    public async Task TestDotnetOrderFulfillmentPipeline()
    {
        var engine = new OctopusLiveEngine();

        engine.RegisterActivity("ValidateOrder", input =>
        {
            double total = 0.0;
            int count = 0;
            if (input.TryGetValue("items", out var itemsObj) && itemsObj is IEnumerable<object> itemsList)
            {
                var list = itemsList.ToList();
                count = list.Count;
                foreach (var raw in list)
                {
                    if (raw is Dictionary<string, object> itm)
                    {
                        var qty = itm.TryGetValue("quantity", out var q) ? Convert.ToInt32(q) : 1;
                        var price = itm.TryGetValue("unit_price", out var p) ? Convert.ToDouble(p) : 0.0;
                        total += qty * price;
                    }
                }
            }

            var tier = input.GetValueOrDefault("customer_tier")?.ToString() ?? "STANDARD";
            if (string.Equals(tier, "VIP", StringComparison.OrdinalIgnoreCase) && total < 1000.0)
            {
                total = 1500.0;
            }

            return new Dictionary<string, object>
            {
                ["order_total"] = total,
                ["order_validated"] = true,
                ["items_count"] = count
            };
        });

        engine.RegisterActivity("ReserveInventoryItem", input =>
        {
            return new Dictionary<string, object>
            {
                ["last_reserved_sku"] = "SKU_TEST"
            };
        });

        engine.RegisterActivity("ScheduleExpressShipping", input =>
        {
            return new Dictionary<string, object>
            {
                ["shipping_method"] = "PRIORITY_EXPRESS_AIR",
                ["courier"] = "FedEx Priority Overnight",
                ["estimated_delivery_days"] = 1
            };
        });

        engine.RegisterActivity("ScheduleStandardShipping", input =>
        {
            return new Dictionary<string, object>
            {
                ["shipping_method"] = "STANDARD_GROUND",
                ["courier"] = "UPS Ground",
                ["estimated_delivery_days"] = 3
            };
        });

        engine.RegisterActivity("SendCustomerNotification", input =>
        {
            return new Dictionary<string, object>
            {
                ["notification_dispatched"] = true
            };
        });

        engine.RegisterActivity("FinalizeOrderResponse", input =>
        {
            return new Dictionary<string, object>
            {
                ["order_id"] = input.GetValueOrDefault("order_id") ?? "ord_unknown",
                ["fulfillment_id"] = "ful_dotnet_123",
                ["status"] = "FULFILLED",
                ["order_total"] = input.GetValueOrDefault("order_total") ?? 0.0,
                ["shipping_method"] = input.GetValueOrDefault("shipping_method") ?? "STANDARD_GROUND",
                ["estimated_delivery_days"] = input.GetValueOrDefault("estimated_delivery_days") ?? 3,
                ["fast_path_latency_ms"] = 0.48,
                ["durable_tracking"] = "REPLICATED_TO_TEMPORAL_AND_SQLITE"
            };
        });

        engine.LoadBlueprintFile("octopus_live_engine/examples/dotnet/order_fulfillment_api/order_fulfillment_workflow.yaml");

        // 1. Standard Order Test
        var stdState = await engine.ExecuteSyncAsync("order_fulfillment_pipeline", new Dictionary<string, object>
        {
            ["customer_id"] = "cust_std_001",
            ["customer_tier"] = "STANDARD",
            ["order_id"] = "ord_std_9921",
            ["shipping_address"] = "742 Evergreen Terrace",
            ["items"] = new List<object>
            {
                new Dictionary<string, object> { ["sku"] = "SKU_BOOK", ["quantity"] = 2, ["unit_price"] = 25.0 }
            }
        });

        Assert.Equal(ExecutionStatus.COMPLETED, stdState.Status);
        Assert.NotNull(stdState.OutputData);
        Assert.Equal("STANDARD_GROUND", stdState.OutputData!["shipping_method"]);

        // 2. VIP Order Test
        var vipState = await engine.ExecuteSyncAsync("order_fulfillment_pipeline", new Dictionary<string, object>
        {
            ["customer_id"] = "cust_vip_007",
            ["customer_tier"] = "VIP",
            ["order_id"] = "ord_vip_7788",
            ["shipping_address"] = "100 Ocean Drive",
            ["items"] = new List<object>
            {
                new Dictionary<string, object> { ["sku"] = "SKU_LAPTOP", ["quantity"] = 2, ["unit_price"] = 1499.0 }
            }
        });

        Assert.Equal(ExecutionStatus.COMPLETED, vipState.Status);
        Assert.NotNull(vipState.OutputData);
        Assert.Equal("PRIORITY_EXPRESS_AIR", vipState.OutputData!["shipping_method"]);

        // 3. Visualizer Diagram Test
        var diagram = await engine.VisualizerService.CompileX6DiagramAsync("order_fulfillment_pipeline", null);
        var nodes = Assert.IsAssignableFrom<IEnumerable<Dictionary<string, object>>>(diagram["nodes"]).ToList();

        var endNode = nodes.FirstOrDefault(n => n.TryGetValue("id", out var id) && id?.ToString() == "node__end");
        Assert.NotNull(endNode);
        Assert.True(endNode["label"]?.ToString() == "COMPLETED" || endNode["label"]?.ToString()?.Contains("200") == true);

        var loopNode = nodes.FirstOrDefault(n => n.TryGetValue("id", out var id) && id?.ToString() == "node__step_reserve_items");
        Assert.NotNull(loopNode);
        Assert.Contains("[Loop Iteration]", loopNode["label"]?.ToString());

        var asyncNode = nodes.FirstOrDefault(n => n.TryGetValue("id", out var id) && id?.ToString() == "node__step_send_notification");
        Assert.NotNull(asyncNode);
        Assert.Contains("[Async Background]", asyncNode["label"]?.ToString());
    }

    [Fact]
    public async Task TestDotnetSensitiveFieldMasking()
    {
        var engine = new OctopusLiveEngine();

        string receivedPassword = string.Empty;
        engine.RegisterActivity("AuthActivity", input =>
        {
            receivedPassword = input.GetValueOrDefault("password")?.ToString() ?? string.Empty;
            return new Dictionary<string, object>
            {
                ["access_token"] = "jwt_secret_token_abc123",
                ["status"] = "authenticated"
            };
        });

        var blueprint = new WorkflowBlueprint
        {
            WorkflowName = "secure_auth_pipeline",
            Version = "1.0.0",
            Mode = "sync",
            Telemetry = new TelemetryConfig
            {
                Masking = new MaskingConfig
                {
                    Enabled = true,
                    Fields = new List<string> { "password", "access_token" },
                    MaskValue = "********"
                }
            },
            Steps = new List<WorkflowStep>
            {
                new()
                {
                    Id = "step_auth",
                    Activity = "AuthActivity",
                    MaskFields = new List<string> { "password" }
                }
            }
        };

        await engine.RegisterBlueprintAsync(blueprint);

        var state = await engine.ExecuteSyncAsync("secure_auth_pipeline", new Dictionary<string, object>
        {
            ["username"] = "alice",
            ["password"] = "superSecretPassword999!"
        });

        Assert.Equal(ExecutionStatus.COMPLETED, state.Status);
        // Ensure in-memory activity received raw plaintext
        Assert.Equal("superSecretPassword999!", receivedPassword);

        // Ensure trace has masked input and output payloads
        Assert.Single(state.Traces);
        var trace = state.Traces[0];
        Assert.Equal("********", trace.InputPayload["password"]);
        Assert.Equal("alice", trace.InputPayload["username"]);
        Assert.Equal("********", trace.OutputPayload["access_token"]);
        Assert.Equal("authenticated", trace.OutputPayload["status"]);

        // Verify visualizer diagram inspection nodes are masked
        var diagram = await engine.VisualizerService.CompileX6DiagramAsync("secure_auth_pipeline", state.JobId);
        var nodes = Assert.IsAssignableFrom<IEnumerable<Dictionary<string, object>>>(diagram["nodes"]).ToList();
        var authNode = nodes.FirstOrDefault(n => n.TryGetValue("id", out var id) && id?.ToString() == "node__step_auth");
        Assert.NotNull(authNode);
        var nodeData = authNode["data"] as Dictionary<string, object>;
        Assert.NotNull(nodeData);
        var nodeInput = nodeData!["input_payload"] as Dictionary<string, object>;
        Assert.NotNull(nodeInput);
        Assert.Equal("********", nodeInput!["password"]);
        var nodeOutput = nodeData!["output_payload"] as Dictionary<string, object>;
        Assert.NotNull(nodeOutput);
        Assert.Equal("********", nodeOutput!["access_token"]);
    }

    [Fact]
    public async Task TestRelationalSqlStateStoreDialects()
    {
        // 1. Dialect SQL Validation
        var pg = new PostgresDialect();
        Assert.Equal("postgres", pg.Name);
        Assert.Equal("$1", pg.Placeholder(1));
        Assert.Contains("ON CONFLICT (job_id) DO UPDATE", pg.UpsertExecutionStateSql);

        var my = new MySqlDialect();
        Assert.Equal("mysql", my.Name);
        Assert.Equal("?", my.Placeholder(1));
        Assert.Contains("ON DUPLICATE KEY UPDATE", my.UpsertExecutionStateSql);

        var ms = new SqlServerDialect();
        Assert.Equal("sqlserver", ms.Name);
        Assert.Contains("MERGE INTO execution_states", ms.UpsertExecutionStateSql);

        var sq = new SqliteDialect();
        Assert.Equal("sqlite", sq.Name);
        Assert.Contains("ON CONFLICT(job_id) DO UPDATE", sq.UpsertExecutionStateSql);

        // 2. StoreFactory Creation
        var storeSqlite = StoreFactory.Create("sqlite");
        Assert.IsType<RelationalSqlStateStore>(storeSqlite);

        var storePg = StoreFactory.Create("postgres", new Dictionary<string, object> { ["dsn"] = "postgresql://localhost:5432/db" });
        Assert.IsType<RelationalSqlStateStore>(storePg);
        Assert.Equal("postgres", ((RelationalSqlStateStore)storePg).Dialect.Name);

        var storeMy = StoreFactory.Create("mysql", new Dictionary<string, object> { ["dsn"] = "mysql://localhost:3306/db" });
        Assert.IsType<RelationalSqlStateStore>(storeMy);
        Assert.Equal("mysql", ((RelationalSqlStateStore)storeMy).Dialect.Name);

        var storeMs = StoreFactory.Create("sqlserver", new Dictionary<string, object> { ["dsn"] = "sqlserver://localhost:1433" });
        Assert.IsType<RelationalSqlStateStore>(storeMs);
        Assert.Equal("sqlserver", ((RelationalSqlStateStore)storeMs).Dialect.Name);

        // 3. Functional Execution
        var state = new ExecutionState
        {
            JobId = "dotnet-relational-job-001",
            WorkflowName = "order_fulfillment_pipeline",
            Status = ExecutionStatus.COMPLETED,
            Route = "/api/v1/orders/checkout",
            Method = "POST",
            StartedAt = "2026-09-23T18:00:00Z",
            Data = new Dictionary<string, object> { ["subtotal"] = 250.0 },
            OutputData = new Dictionary<string, object> { ["total"] = 250.0, ["status"] = "PROCESSED" },
            Traces = new List<StepExecutionTrace>
            {
                new()
                {
                    StepId = "step_reserve",
                    ActivityName = "ReserveInventoryItem",
                    Status = "COMPLETED",
                    InputPayload = new Dictionary<string, object> { ["sku"] = "SKU_DOTNET" },
                    OutputPayload = new Dictionary<string, object> { ["reserved"] = true },
                    DurationMs = 0.72
                }
            }
        };

        await storePg.SaveStateAsync(state);

        var fetched = await storePg.GetStateAsync("dotnet-relational-job-001");
        Assert.NotNull(fetched);
        Assert.Equal("dotnet-relational-job-001", fetched!.JobId);
        Assert.Equal("order_fulfillment_pipeline", fetched.WorkflowName);
        Assert.Equal(ExecutionStatus.COMPLETED, fetched.Status);
        Assert.Single(fetched.Traces);
        Assert.Equal("ReserveInventoryItem", fetched.Traces[0].ActivityName);
        Assert.Equal(0.72, fetched.Traces[0].DurationMs);

        var list = await storePg.ListStatesAsync("order_fulfillment_pipeline", 10);
        Assert.Single(list);
        Assert.Equal("dotnet-relational-job-001", list[0].JobId);
    }
}


