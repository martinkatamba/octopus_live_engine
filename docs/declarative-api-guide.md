# Declarative Model-Driven REST API Guide for OCTOPUS Live Engine

The **OCTOPUS Live In-Memory Engine** (`octopus_live_engine`) enables developers to build high-performance, model-driven, synchronous REST APIs directly from YAML workflow blueprints.

Instead of writing boilerplate controllers, route bindings, and trace handlers, you declare your API endpoints, client-side input/output models, and step pipelines directly in YAML. The engine handles:
- **Direct HTTP Routing**: Exposes declared HTTP endpoints (`GET`, `POST`, `PUT`, `DELETE`, `PATCH`) with custom status codes (200, 201, 204).
- **Client-side Model Registration**: Register Pydantic models (Python), Structs (Go), C# records/classes (.NET), or Java POJOs/Records to validate and structure request/response payloads.
- **Transaction Tracing with `job_id`**: Every HTTP request automatically extracts or generates a `job_id` (propagated via `X-Job-ID` header), which is fully traceable in the interactive **Visualizer**.
- **Interactive Visualizer with Route & Method Filtering**: The Visualizer renders the exact DAG pipeline corresponding to the requested endpoint, and highlights the executed path for any given `job_id`.
- **Automatic OpenAPI 3.0 & Swagger UI**: Serves interactive API documentation at `/docs` and the OpenAPI schema at `/openapi.json`.

---

## 1. Declarative YAML Schema

A single workflow blueprint can declare multiple API endpoints with dedicated step pipelines:

```yaml
workflow_name: checkout_tax_calculator
version: "1.0.0"
mode: sync

api:
  # ENDPOINT 1: Calculate Sales Tax & VAT (POST)
  - route: /api/v1/checkout/tax
    method: POST
    input_model: CheckoutTaxRequest
    output_model: CheckoutTaxResponse
    status_code: 200
    summary: "Calculate sales tax and customs duty"
    tags: ["Checkout"]
    steps:
      - id: step_normalize_cart_and_address
        activity: NormalizeCartAndAddress
        timeout_seconds: 5

      - id: step_calculate_jurisdiction_tax
        activity: CalculateJurisdictionTax
        timeout_seconds: 5

      - id: step_package_tax_response
        activity: PackageTaxResponse
        is_output: true                   # Explicitly marks this activity to format the output_model response
        timeout_seconds: 2

  # ENDPOINT 2: Retrieve Default Tax Summary (GET)
  - route: /api/v1/checkout/tax
    method: GET
    output_model: DefaultTaxRatesResponse
    status_code: 200
    summary: "Fetch standard tax rates for jurisdictions"
    tags: ["Checkout"]
    steps:
      - id: step_fetch_default_rates
        activity: FetchDefaultTaxRates
        is_output: true
        timeout_seconds: 3

  # ENDPOINT 3: Quick Postal Code Verification (POST)
  - route: /api/v1/checkout/validate
    method: POST
    status_code: 201
    summary: "Validate shipping postal code"
    tags: ["Checkout"]
    steps:
      - id: step_validate_postal
        activity: ValidatePostalCode
        is_output: true
        timeout_seconds: 2
```

---

## 2. Polyglot Implementation Examples

### Python (`FastAPI` / `Pydantic`)

```python
from pydantic import BaseModel
from octopus_live import LiveEngine

# 1. Define Client Models
class CheckoutTaxRequest(BaseModel):
    amount: float
    country: str = "US"

class CheckoutTaxResponse(BaseModel):
    total_tax: float
    total_amount: float

# 2. Instantiate Engine & Register Models
engine = LiveEngine()
engine.register_model("CheckoutTaxRequest", CheckoutTaxRequest)
engine.register_model("CheckoutTaxResponse", CheckoutTaxResponse)

# 3. Register Business Activities
@engine.activity("NormalizeCartAndAddress")
def normalize(payload):
    return {"normalized": True, "country": payload.get("country", "US").upper()}

@engine.activity("CalculateJurisdictionTax")
def calc_tax(payload):
    amount = float(payload.get("amount", 0.0))
    rate = 0.08 if payload.get("country") == "US" else 0.20
    tax = amount * rate
    return {"total_tax": tax, "total_amount": amount + tax}

# 4. Load Blueprint & Serve FastAPI App
engine.load_blueprint_file("checkout_tax_calculator.yaml")
app = engine.get_fastapi_app()
# Uvicorn: uvicorn main:app --port 8000
```

---

### Go (`Gin` / Structs)

```go
package main

import (
    "context"
    "github.com/martinkatamba/octopus_live_engine/go/pkg/octopus_live"
)

type CheckoutTaxRequest struct {
    Amount  float64 `json:"amount"`
    Country string  `json:"country"`
}

type CheckoutTaxResponse struct {
    TotalTax    float64 `json:"total_tax"`
    TotalAmount float64 `json:"total_amount"`
}

func main() {
    engine := octopus_live.NewLiveEngine()

    // 1. Register Models
    engine.RegisterModel("CheckoutTaxRequest", CheckoutTaxRequest{})
    engine.RegisterModel("CheckoutTaxResponse", CheckoutTaxResponse{})

    // 2. Register Activities
    engine.RegisterActivity("NormalizeCartAndAddress", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
        country, _ := input["country"].(string)
        if country == "" { country = "US" }
        return map[string]interface{}{"normalized": true, "country": country}, nil
    })

    engine.RegisterActivity("CalculateJurisdictionTax", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
        amount, _ := input["amount"].(float64)
        rate := 0.08
        tax := amount * rate
        return map[string]interface{}{"total_tax": tax, "total_amount": amount + tax}, nil
    })

    // 3. Load Blueprint & Run Gin Server
    engine.LoadBlueprintFile("checkout_tax_calculator.yaml")
    r := engine.GetGinEngine()
    r.Run(":8000")
}
```

---

### .NET 8 (C# / ASP.NET Core Minimal APIs)

```csharp
using Octopus.LiveEngine;
using Octopus.LiveEngine.Adapters.Inbound;

var builder = WebApplication.CreateBuilder(args);
var app = builder.Build();

var engine = new OctopusLiveEngine();

// 1. Register Models
public record CheckoutTaxRequest(double Amount, string Country);
public record CheckoutTaxResponse(double TotalTax, double TotalAmount);

engine.RegisterModel<CheckoutTaxRequest>("CheckoutTaxRequest");
engine.RegisterModel<CheckoutTaxResponse>("CheckoutTaxResponse");

// 2. Register Activities
engine.RegisterActivity("NormalizeCartAndAddress", (input, ct) => {
    var country = input.GetValueOrDefault("country", "US")?.ToString()?.ToUpper() ?? "US";
    return Task.FromResult<IDictionary<string, object>>(new Dictionary<string, object> {
        ["normalized"] = true,
        ["country"] = country
    });
});

engine.RegisterActivity("CalculateJurisdictionTax", (input, ct) => {
    var amount = Convert.ToDouble(input.GetValueOrDefault("amount", 100.0));
    var tax = amount * 0.08;
    return Task.FromResult<IDictionary<string, object>>(new Dictionary<string, object> {
        ["total_tax"] = tax,
        ["total_amount"] = amount + tax
    });
});

// 3. Load Blueprint & Map Endpoints
engine.LoadBlueprintFile("checkout_tax_calculator.yaml");
app.MapLiveEndpoints(engine);
app.Run("http://0.0.0.0:8000");
```

---

### Java 21+ (`LiveEngine` / Virtual Threads)

```java
import io.octopus.live.LiveEngine;
import java.util.Map;
import java.util.HashMap;

public class Main {
    public static class CheckoutTaxRequest {
        public double amount;
        public String country;
    }

    public static class CheckoutTaxResponse {
        public double total_tax;
        public double total_amount;
    }

    public static void main(String[] args) throws Exception {
        LiveEngine engine = new LiveEngine();

        // 1. Register Models
        engine.registerModel("CheckoutTaxRequest", CheckoutTaxRequest.class);
        engine.registerModel("CheckoutTaxResponse", CheckoutTaxResponse.class);

        // 2. Register Activities
        engine.registerActivity("NormalizeCartAndAddress", input -> {
            Map<String, Object> out = new HashMap<>();
            out.put("normalized", true);
            out.put("country", input.getOrDefault("country", "US").toString().toUpperCase());
            return out;
        });

        engine.registerActivity("CalculateJurisdictionTax", input -> {
            double amount = ((Number) input.getOrDefault("amount", 100.0)).doubleValue();
            double tax = amount * 0.08;
            Map<String, Object> out = new HashMap<>();
            out.put("total_tax", tax);
            out.put("total_amount", amount + tax);
            return out;
        });

        // 3. Load Blueprint & Start HTTP Server
        engine.loadBlueprintsFromDirectory("./blueprints");
        engine.startServer(8000);
    }
}
```

---

## 3. Transaction Tracing with `job_id`

Every API call receives or generates a unique `job_id`. 

### Inbound Propagation
You can pass your own transaction ID in any of the following:
- HTTP Header: `X-Job-ID: tx-checkout-98124`
- HTTP Header: `X-Request-ID` or `X-Correlation-ID`
- JSON Body: `{"job_id": "tx-checkout-98124", "amount": 250.0}`
- Query Param: `?job_id=tx-checkout-98124`

### Outbound Response Headers
All API responses include telemetry headers:
- `X-Job-ID`: `tx-checkout-98124`
- `X-Execution-Status`: `COMPLETED`
- `X-Execution-Duration-Ms`: `1.42`

---

## 4. Interactive Visualizer

Visit `/visualizer` in your browser:
- **Search by `job_id`**: Enter `tx-checkout-98124` to highlight the exact execution path taken by that transaction.
- **Filter by Route & Method**: Switch between `POST /checkout/tax`, `GET /checkout/tax`, and `POST /checkout/validate` to see the exact DAG diagram for each endpoint.
- **Inspector Drawer**: Click any node to inspect execution status, duration in milliseconds, inputs, and outputs.
