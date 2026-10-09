*** Settings ***
Documentation     OCTOPUS Java 21+ Customer Auth Portal API Live Engine Visualizer End-to-End Test Suite with JWT Auth, Tier Branching & Middlewares
Library           Process
Library           RequestsLibrary
Library           Collections
Library           OperatingSystem
Library           String
Library           SeleniumLibrary

Suite Setup       Setup Customer Auth Portal E2E Suite
Suite Teardown    Teardown Customer Auth Portal E2E Suite

*** Variables ***
${PORT}               8094
${HOST}               http://127.0.0.1:${PORT}
${BASE_URL}           ${HOST}/api/v1.0.0/customer_auth_portal
${DOMAIN}             customer_auth_portal
${VISUALIZER_URL}     ${HOST}/visualizer?domain=${DOMAIN}
${SCREENSHOTS_DIR}    ${CURDIR}/screenshots

*** Test Cases ***

Scenario 1: Visualizer Loads Static Blueprint for Customer Auth Portal
    [Documentation]    Verifies that navigating to /visualizer loads the Java Customer Auth Portal workflow blueprint.
    Go To    ${VISUALIZER_URL}
    Wait Until Page Contains Element    id:jobIdInput    15s
    Wait Until Page Contains Element    id:graph-container    15s
    
    Wait Until Keyword Succeeds    15s    1s    Wait For Graph Nodes Rendered

    ${node_count}=    Execute Javascript    return (window.graph && window.graph.getNodes) ? window.graph.getNodes().length : 0;
    ${edge_count}=    Execute Javascript    return (window.graph && window.graph.getEdges) ? window.graph.getEdges().length : 0;
    
    Log To Console    \n📊 Java Customer Auth Portal Blueprint: ${node_count} nodes, ${edge_count} edges
    Should Be True    ${node_count} >= 3    Expected at least 3 nodes
    Should Be True    ${edge_count} >= 2    Expected edges connecting start, activities, and end

    Execute Javascript    if (window.graph && window.graph.zoomToFit) { window.graph.zoomToFit({ padding: 30, maxScale: 1.2 }); }
    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/01_java_auth_portal_blueprint.png


Scenario 2: Route Selector Dropdown Displays All Declared API Routes
    [Documentation]    Verifies that the route selection dropdown displays all 3 declared API routes and dynamically switches the DAG.
    Go To    ${VISUALIZER_URL}
    Wait Until Page Contains Element    id:routeSelect    15s
    Wait Until Element Is Visible       id:routeSelect    15s
    
    ${options}=    Get List Items       id:routeSelect
    Log To Console    \n🔀 Java Visualizer Routes: ${options}
    Length Should Be    ${options}    3

    # Switch to Cart Checkout Route (Option 3)
    Select From List By Index    id:routeSelect    2
    Execute Javascript    onRouteSelectChanged();
    Sleep    1s
    Wait Until Keyword Succeeds    15s    1s    Wait For Graph Nodes Rendered

    ${node_count}=    Execute Javascript    return (window.graph && window.graph.getNodes) ? window.graph.getNodes().length : 0;
    Log To Console    \n🛒 Checkout Route DAG Node Count: ${node_count}
    Should Be True    ${node_count} >= 5    Expected checkout branch nodes to be rendered



Scenario 3: Public Login Issues Valid Signed Bearer JWT Token
    [Documentation]    Calls POST /auth/login with Sarah VIP credentials and receives a valid Bearer JWT.
    ${job_id}=        Generate Random Job Id    java-auth-login
    Set Suite Variable    ${LOGIN_JOB_ID}    ${job_id}
    
    ${payload}=       Create Dictionary    username=sarah_vip    password=password123
    ${headers}=       Create Dictionary    X-Job-ID=${job_id}    Content-Type=application/json

    ${resp}=          POST On Session    auth_portal_session    url=/auth/login    json=${payload}    headers=${headers}
    Status Should Be  200    ${resp}
    ${body}=          Set Variable    ${resp.json()}
    
    ${token}=           Get From Dictionary    ${body}    access_token
    ${token_type}=      Get From Dictionary    ${body}    token_type
    ${user_id}=         Get From Dictionary    ${body}    user_id
    ${tier}=            Get From Dictionary    ${body}    customer_tier

    Should Be Equal As Strings    ${token_type}    Bearer
    Should Be Equal As Strings    ${user_id}       usr_9988_vip
    Should Be Equal As Strings    ${tier}          VIP
    Should Not Be Empty           ${token}
    Set Suite Variable            ${SARAH_TOKEN}   ${token}

    Log To Console    \n🔑 Java Public Login Succeeded: UserID=${user_id}, Tier=${tier}


Scenario 3: Public Login Rejects Bad Credentials with Declarative 401
    [Documentation]    Calls POST /auth/login with wrong password and verifies declarative exception message.
    ${payload}=       Create Dictionary    username=sarah_vip    password=invalidPassword
    ${headers}=       Create Dictionary    Content-Type=application/json

    ${resp}=          POST On Session    auth_portal_session    url=/auth/login    json=${payload}    headers=${headers}    expected_status=401
    Status Should Be  401    ${resp}
    ${body}=          Set Variable    ${resp.json()}
    
    ${detail}=        Get From Dictionary    ${body}    detail
    Should Contain    ${detail}    Invalid username or password. Please try again.
    Log To Console    \n🛡️ Java Bad Credentials Correctly Blocked with 401 & Declarative Injected Message


Scenario 4: Protected Profile Route Rejects Request Without Token
    [Documentation]    Calls GET /profile without Authorization header and expects 401 UserNotAuthenticated.
    ${resp}=          GET On Session    auth_portal_session    url=/profile    expected_status=401
    Status Should Be  401    ${resp}
    ${body}=          Set Variable    ${resp.json()}
    
    ${error}=         Get From Dictionary    ${body}    error
    Should Be Equal As Strings    ${error}    UserNotAuthenticated
    Log To Console    \n🔒 Java Protected Route Unauthenticated Access Denied: Error=${error}


Scenario 5: Authenticated User Profile Lookup with Bearer Token
    [Documentation]    Calls GET /profile with Sarah's Bearer token and verifies user details.
    ${headers}=       Create Dictionary    Authorization=Bearer ${SARAH_TOKEN}

    ${resp}=          GET On Session    auth_portal_session    url=/profile    headers=${headers}
    Status Should Be  200    ${resp}
    ${body}=          Set Variable    ${resp.json()}
    
    ${user_id}=       Get From Dictionary    ${body}    user_id
    ${username}=      Get From Dictionary    ${body}    username
    ${tier}=          Get From Dictionary    ${body}    customer_tier
    ${status}=        Get From Dictionary    ${body}    account_status

    Should Be Equal As Strings    ${user_id}     usr_9988_vip
    Should Be Equal As Strings    ${username}    sarah_vip
    Should Be Equal As Strings    ${tier}        VIP
    Should Be Equal As Strings    ${status}      ACTIVE_VIP
    Log To Console    \n👤 Java User Profile Retrieved for Sarah VIP: status=${status}


Scenario 6: Protected VIP Cart Checkout Applies 20% Discount
    [Documentation]    Calls POST /cart/checkout with VIP Token and verifies discount is applied via condition branch.
    ${job_id}=        Generate Random Job Id    java-vip-checkout
    Set Suite Variable    ${VIP_CHECKOUT_JOB_ID}    ${job_id}

    ${item1}=         Create Dictionary    sku=ITEM-101    description=4K Gaming Monitor    price=${500.00}    quantity=${1}
    ${item2}=         Create Dictionary    sku=ITEM-102    description=Mechanical Keyboard   price=${100.00}    quantity=${1}
    ${items}=         Create List    ${item1}    ${item2}

    ${payload}=       Create Dictionary    cart_id=cart_vip_java_99    items=${items}    shipping_address=100 Silicon Ave, San Jose, CA
    ${headers}=       Create Dictionary    Authorization=Bearer ${SARAH_TOKEN}    X-Job-ID=${job_id}    Content-Type=application/json

    ${resp}=          POST On Session    auth_portal_session    url=/cart/checkout    json=${payload}    headers=${headers}
    Status Should Be  200    ${resp}
    ${body}=          Set Variable    ${resp.json()}
    
    ${subtotal}=      Get From Dictionary    ${body}    subtotal
    ${discount}=      Get From Dictionary    ${body}    discount_applied
    ${final_total}=   Get From Dictionary    ${body}    final_total
    ${pricing_tier}=  Get From Dictionary    ${body}    pricing_tier
    ${shipping}=      Get From Dictionary    ${body}    shipping_method
    ${order_id}=      Get From Dictionary    ${body}    order_id

    Should Be Equal As Numbers    ${subtotal}       600.00
    Should Be Equal As Numbers    ${discount}       120.00    # 20% off $600
    Should Be Equal As Numbers    ${final_total}    480.00
    Should Contain                ${pricing_tier}   VIP_PREFERRED
    Should Contain                ${shipping}       Priority Express
    Should Not Be Empty           ${order_id}

    Log To Console    \n💎 Java VIP Checkout Succeeded: Subtotal=$${subtotal}, Discount=$${discount}, Final=$${final_total}


Scenario 7: Standard User Cart Checkout Gets Retail Pricing
    [Documentation]    Authenticates John Doe (Standard User) and confirms standard retail pricing without discount.
    # 1. Login as John Doe
    ${login_payload}=    Create Dictionary    username=john_doe    password=password123
    ${login_headers}=    Create Dictionary    Content-Type=application/json
    ${login_resp}=       POST On Session      auth_portal_session    url=/auth/login    json=${login_payload}    headers=${login_headers}
    Status Should Be     200    ${login_resp}
    ${john_token}=       Get From Dictionary  ${login_resp.json()}    access_token

    # 2. Checkout Cart
    ${job_id}=        Generate Random Job Id    java-std-checkout
    ${item1}=         Create Dictionary    sku=ITEM-101    description=4K Gaming Monitor    price=${500.00}    quantity=${1}
    ${items}=         Create List    ${item1}

    ${payload}=       Create Dictionary    cart_id=cart_std_java_88    items=${items}    shipping_address=456 Main St, Dallas, TX
    ${headers}=       Create Dictionary    Authorization=Bearer ${john_token}    X-Job-ID=${job_id}    Content-Type=application/json

    ${resp}=          POST On Session    auth_portal_session    url=/cart/checkout    json=${payload}    headers=${headers}
    Status Should Be  200    ${resp}
    ${body}=          Set Variable    ${resp.json()}
    
    ${subtotal}=      Get From Dictionary    ${body}    subtotal
    ${discount}=      Get From Dictionary    ${body}    discount_applied
    ${final_total}=   Get From Dictionary    ${body}    final_total
    ${pricing_tier}=  Get From Dictionary    ${body}    pricing_tier

    Should Be Equal As Numbers    ${subtotal}       500.00
    Should Be Equal As Numbers    ${discount}       0.00
    Should Be Equal As Numbers    ${final_total}    500.00
    Should Contain                ${pricing_tier}   STANDARD_RETAIL

    Log To Console    \n🛒 Java Standard Checkout Succeeded: Subtotal=$${subtotal}, Discount=$${discount}, Final=$${final_total}


Scenario 8: Visualizer Live Interactive Inspection for VIP Journey
    [Documentation]    Looks up VIP checkout job in Visualizer, inspects execution details and takes visual screenshot.
    Go To    ${VISUALIZER_URL}
    Wait Until Page Contains Element    id:jobIdInput    15s
    Clear Element Text    id:jobIdInput
    Input Text    id:jobIdInput    ${VIP_CHECKOUT_JOB_ID}
    Click Button    id:lookupBtn

    Wait Until Element Is Visible    id:executionBanner    15s
    Wait Until Element Contains      id:executionSummaryText    COMPLETED    15s
    
    Execute Javascript    if (window.graph && window.graph.zoomToFit) { window.graph.zoomToFit({ padding: 30, maxScale: 1.2 }); }
    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/02_java_auth_portal_vip_journey.png


Scenario 9: Visualizer Inspects Branch Step Condition and Live Rule Evaluation
    [Documentation]    Clicks on step_branch_by_tier in the visualizer, verifies that the branch inspector drawer opens with conditions, targets, and live evaluation verdict.
    # Click branch node
    ${selected}=    Execute Javascript    return window.selectNode('step_branch_by_tier');
    Should Be True    ${selected}    Expected step_branch_by_tier node to be selected
    
    # Wait for inspector panel to open
    Wait Until Page Contains Element    id:insp-branch-field    15s
    Wait Until Element Is Visible       id:insp-branch-field       15s
    Wait Until Element Is Visible       id:insp-branch-eval-box    15s

    ${rules_text}=      Get Text    id:insp-branch-rules-content
    ${eval_text}=       Get Text    id:insp-branch-eval-details
    ${verdict_text}=    Get Text    id:insp-branch-verdict-badge

    Log To Console    \n⚏ Branch Rules: ${rules_text}
    Log To Console    \n⚡ Branch Live Evaluation: ${eval_text}
    Log To Console    \n🏷️ Branch Verdict: ${verdict_text}

    Should Contain    ${rules_text}      customer_tier
    Should Contain    ${verdict_text}    TRUE
    Should Contain    ${eval_text}       VIP

    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/03_java_auth_portal_branch_inspector.png


Scenario 7: OpenAPI Specification Contains Typing Models and Security Schemes
    [Documentation]    Verifies that /openapi.json contains components.securitySchemes, typing schemas, request bodies, and operation security.
    ${resp}=    GET    ${HOST}/openapi.json
    Status Should Be    200    ${resp}
    ${json}=    Set Variable    ${resp.json()}
    
    # 1. Security Schemes
    Dictionary Should Contain Key    ${json}    components
    ${components}=    Get From Dictionary    ${json}    components
    Dictionary Should Contain Key    ${components}    securitySchemes
    ${sec_schemes}=    Get From Dictionary    ${components}    securitySchemes
    Dictionary Should Contain Key    ${sec_schemes}    HTTPBearer
    ${bearer}=    Get From Dictionary    ${sec_schemes}    HTTPBearer
    Should Be Equal As Strings    ${bearer}[type]    http
    Should Be Equal As Strings    ${bearer}[scheme]    bearer

    # 2. Schemas
    Dictionary Should Contain Key    ${components}    schemas
    ${schemas}=    Get From Dictionary    ${components}    schemas
    Dictionary Should Contain Key    ${schemas}    LoginRequest
    Dictionary Should Contain Key    ${schemas}    LoginResponse
    Dictionary Should Contain Key    ${schemas}    UserProfileResponse
    Dictionary Should Contain Key    ${schemas}    CheckoutRequest
    Dictionary Should Contain Key    ${schemas}    CheckoutResponse
    Dictionary Should Contain Key    ${schemas}    CartItem
    Dictionary Should Contain Key    ${schemas}    ErrorResponse

    # 3. Path Security & RequestBody
    ${paths}=    Get From Dictionary    ${json}    paths
    Dictionary Should Contain Key    ${paths}    /api/v1.0.0/customer_auth_portal/auth/login
    Dictionary Should Contain Key    ${paths}    /api/v1.0.0/customer_auth_portal/profile
    Dictionary Should Contain Key    ${paths}    /api/v1.0.0/customer_auth_portal/cart/checkout

    # Login route is public: security is empty list
    ${login_path_obj}=    Get From Dictionary    ${paths}    /api/v1.0.0/customer_auth_portal/auth/login
    ${login_op}=          Get From Dictionary    ${login_path_obj}    post
    Length Should Be    ${login_op}[security]    0

    # Profile route is protected: security contains HTTPBearer
    ${profile_path_obj}=    Get From Dictionary    ${paths}    /api/v1.0.0/customer_auth_portal/profile
    ${profile_op}=          Get From Dictionary    ${profile_path_obj}    get
    Length Should Be    ${profile_op}[security]    1

    # Checkout route is protected: security contains HTTPBearer and has CheckoutRequest body
    ${checkout_path_obj}=    Get From Dictionary    ${paths}    /api/v1.0.0/customer_auth_portal/cart/checkout
    ${checkout_op}=          Get From Dictionary    ${checkout_path_obj}    post
    Length Should Be    ${checkout_op}[security]    1
    Dictionary Should Contain Key    ${checkout_op}    requestBody


Scenario 8: Swagger UI Docs Render Security Authorize Button and Endpoint Operations
    [Documentation]    Verifies that navigating to /docs loads Swagger UI with the Authorize button and all endpoint sections.
    Go To    ${HOST}/docs
    Wait Until Page Contains Element    class:swagger-ui    15s
    Wait Until Page Contains Element    class:btn.authorize    15s
    Wait Until Element Is Visible       class:btn.authorize    15s
    
    ${auth_btn_text}=    Get Text    class:btn.authorize
    Log To Console    \n🔒 Swagger UI Authorize Button: ${auth_btn_text}
    Should Contain    ${auth_btn_text}    Authorize

    # Verify endpoint blocks
    Wait Until Page Contains Element    class:opblock-summary-path    15s
    ${paths}=    Get WebElements    class:opblock-summary-path
    ${path_count}=    Get Length    ${paths}
    Log To Console    \n📚 Swagger UI Rendered Operations Count: ${path_count}
    Should Be True    ${path_count} >= 3

    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/04_java_swagger_ui_docs.png




*** Keywords ***

Setup Customer Auth Portal E2E Suite
    Create Directory    ${SCREENSHOTS_DIR}
    Create Directory    ${CURDIR}/results

    # Build classpath
    ${cp}=    Set Variable    /Volumes/Macintosh HD - Data/2026/AI_PROJECTS/project-octopus/octopus_live_engine/examples/java/customer_auth_portal/classes:/Volumes/Macintosh HD - Data/2026/AI_PROJECTS/project-octopus/octopus_live_engine/java/target/octopus-live-engine-1.0.0.jar:/Users/martin/.m2/repository/com/fasterxml/jackson/core/jackson-core/2.17.2/jackson-core-2.17.2.jar:/Users/martin/.m2/repository/com/fasterxml/jackson/core/jackson-databind/2.17.2/jackson-databind-2.17.2.jar:/Users/martin/.m2/repository/com/fasterxml/jackson/core/jackson-annotations/2.17.2/jackson-annotations-2.17.2.jar:/Users/martin/.m2/repository/com/fasterxml/jackson/dataformat/jackson-dataformat-yaml/2.17.2/jackson-dataformat-yaml-2.17.2.jar:/Users/martin/.m2/repository/org/yaml/snakeyaml/2.2/snakeyaml-2.2.jar

    # Start Java Server
    ${server_process}=    Start Process    /Library/Java/JavaVirtualMachines/jdk-22.jdk/Contents/Home/bin/java    -cp    ${cp}    io.octopus.examples.authportal.CustomerAuthPortalApplication    --port    ${PORT}    cwd=${CURDIR}
    Set Suite Variable    ${SERVER_PROCESS}    ${server_process}

    # Wait for server health
    Wait Until Keyword Succeeds    30s    1s    Check Server Health

    # Setup REST session
    Create Session    auth_portal_session    ${BASE_URL}    verify=True

    Open Headless Browser


Teardown Customer Auth Portal E2E Suite
    Run Keyword And Ignore Error    Close All Browsers
    Run Keyword And Ignore Error    Delete All Sessions
    IF    $SERVER_PROCESS is not None
        Terminate Process    ${SERVER_PROCESS}    kill=True
    END


Check Server Health
    ${resp}=    GET    ${HOST}/health
    Status Should Be    200    ${resp}


Open Headless Browser
    ${chrome_options}=    Evaluate    sys.modules['selenium.webdriver'].ChromeOptions()    sys, selenium.webdriver
    Call Method    ${chrome_options}    add_argument    --headless
    Call Method    ${chrome_options}    add_argument    --no-sandbox
    Call Method    ${chrome_options}    add_argument    --disable-dev-shm-usage
    Call Method    ${chrome_options}    add_argument    --window-size\=1920,1080
    Call Method    ${chrome_options}    add_argument    --force-device-scale-factor\=2
    Call Method    ${chrome_options}    add_argument    --high-dpi-support\=1
    Call Method    ${chrome_options}    add_argument    --disable-gpu
    Create Webdriver    Chrome    options=${chrome_options}
    Set Selenium Implicit Wait    5s


Wait For Graph Nodes Rendered
    ${node_count}=    Execute Javascript    return (window.graph && window.graph.getNodes) ? window.graph.getNodes().length : 0;
    Should Be True    ${node_count} > 0    Nodes not yet rendered in AntV X6 graph


Generate Random Job Id
    [Arguments]    ${prefix}=job
    ${rand_str}=    Generate Random String    8    [LOWER][NUMBERS]
    ${job_id}=      Set Variable    ${prefix}-${rand_str}
    RETURN    ${job_id}
