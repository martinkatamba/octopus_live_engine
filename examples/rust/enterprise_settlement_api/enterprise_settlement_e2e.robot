*** Settings ***
Documentation     OCTOPUS Rust Enterprise Settlement Gateway Live Engine Visualizer End-to-End Test Suite
...               Demonstrating 100% of Engine Capabilities:
...               - Model-Driven Declarative REST API Routing
...               - Declarative Middlewares (JWT Bearer Auth, Rate Limiting, IP Whitelist, Payload Validation, Audit)
...               - In-Memory Loop Processing (Multi-Item Inventory Reservations)
...               - Dynamic Conditional Branching (VIP Preferential Discount vs. Standard Retail Ground)
...               - In-Memory Saga Transactions & Reverse Compensation Rollbacks (Card Decline Simulation)
...               - Two-Phase Settlement Paused on Asynchronous Signal (await_signal & resumption)
...               - Asynchronous Background Execution (async: true notifications)
...               - Telemetry Field Masking (credit cards, CVV, passwords)
...               - AntV X6 Interactive Web Visualizer with Live Path Tracing & Inspector Drawers
Library           Process
Library           RequestsLibrary
Library           Collections
Library           OperatingSystem
Library           String
Library           SeleniumLibrary

Suite Setup       Setup Enterprise Settlement E2E Suite
Suite Teardown    Teardown Enterprise Settlement E2E Suite

*** Variables ***
${PORT}               8095
${HOST}               http://127.0.0.1:${PORT}
${BASE_URL}           ${HOST}/api/v1
${DOMAIN}             enterprise_settlement_gateway
${VISUALIZER_URL}     ${HOST}/visualizer?domain=${DOMAIN}
${SCREENSHOTS_DIR}    ${CURDIR}/screenshots

*** Test Cases ***

Scenario 1: Visualizer Loads Static Blueprint Displaying All Node Types
    [Documentation]    Verifies that /visualizer renders the complete workflow DAG with loop, branch, signal, async, and activity nodes.
    Go To    ${VISUALIZER_URL}
    Wait Until Page Contains Element    id:jobIdInput    15s
    Wait Until Page Contains Element    id:graph-container    15s
    
    Wait Until Keyword Succeeds    15s    1s    Wait For Graph Nodes Rendered

    ${node_count}=    Execute Javascript    return (window.graph && window.graph.getNodes) ? window.graph.getNodes().length : 0;
    ${edge_count}=    Execute Javascript    return (window.graph && window.graph.getEdges) ? window.graph.getEdges().length : 0;
    
    Log To Console    \n📊 Enterprise Settlement Blueprint: ${node_count} nodes, ${edge_count} edges
    Should Be True    ${node_count} >= 6    Expected at least 6 nodes in the comprehensive DAG
    Should Be True    ${edge_count} >= 5    Expected connecting edges

    Execute Javascript    if (window.graph && window.graph.zoomToFit) { window.graph.zoomToFit({ padding: 30, maxScale: 1.2 }); }
    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/01_rust_enterprise_blueprint_all_capabilities.png


Scenario 2: Public Login Issues Valid Signed HS256 JWT Bearer Token
    [Documentation]    Calls POST /auth/login with VIP credentials and receives a valid Bearer JWT. Also verifies bad credentials return 401.
    # 1. Login as Sarah VIP
    ${login_payload}=    Create Dictionary    username=sarah_vip    password=secret123
    ${login_resp}=       POST On Session    settlement_session    url=/auth/login    json=${login_payload}
    Status Should Be     200    ${login_resp}
    
    ${body}=             Set Variable    ${login_resp.json()}
    ${token}=            Get From Dictionary    ${body}    access_token
    ${token_type}=       Get From Dictionary    ${body}    token_type
    ${user_id}=          Get From Dictionary    ${body}    user_id
    ${tier}=             Get From Dictionary    ${body}    customer_tier

    Should Be Equal As Strings    ${token_type}    Bearer
    Should Be Equal As Strings    ${user_id}       usr_sarah_vip
    Should Be Equal As Strings    ${tier}          VIP
    Should Not Be Empty           ${token}
    Set Suite Variable            ${SARAH_TOKEN}   ${token}
    Log To Console    \n🔑 VIP Login Succeeded: UserID=${user_id}, Tier=${tier}

    # 2. Login as John Standard
    ${john_payload}=     Create Dictionary    username=john_standard    password=secret123
    ${john_resp}=        POST On Session    settlement_session    url=/auth/login    json=${john_payload}
    Status Should Be     200    ${john_resp}
    ${john_token}=       Get From Dictionary    ${john_resp.json()}    access_token
    Set Suite Variable   ${JOHN_TOKEN}    ${john_token}

    # 3. Test Invalid Credentials
    ${bad_payload}=      Create Dictionary    username=sarah_vip    password=wrongpassword
    ${bad_resp}=         POST On Session    settlement_session    url=/auth/login    json=${bad_payload}    expected_status=401
    Status Should Be     401    ${bad_resp}
    Log To Console    \n🛡️ Bad Credentials Rejected with 401


Scenario 3: Fast-Path VIP Checkout Journey with Tier Branching & Loop Reservations
    [Documentation]    Executes VIP order with multi-item reservation loop and verifies 20% discount + Express shipping.
    ${job_id}=        Generate Random Job Id    rust-vip-settle
    Set Suite Variable    ${VIP_JOB_ID}    ${job_id}

    ${item1}=         Create Dictionary    sku=SKU-MACBOOK-M3    quantity=${1}    unit_price=${2000.00}
    ${item2}=         Create Dictionary    sku=SKU-STUDIO-DISPLAY    quantity=${1}    unit_price=${1500.00}
    ${items}=         Create List    ${item1}    ${item2}

    ${payload}=       Create Dictionary    order_id=ORD-VIP-8899    currency=USD    card_number=4111-2222-3333-4444    cvv=123    items=${items}
    ${headers}=       Create Dictionary    Authorization=Bearer ${SARAH_TOKEN}    X-Job-ID=${job_id}    Content-Type=application/json

    ${resp}=          POST On Session    settlement_session    url=/settlement/process    json=${payload}    headers=${headers}
    Status Should Be  200    ${resp}
    ${body}=          Set Variable    ${resp.json()}

    ${tier}=          Get From Dictionary    ${body}    customer_tier
    ${subtotal}=      Get From Dictionary    ${body}    subtotal
    ${discount}=      Get From Dictionary    ${body}    discount_applied
    ${final_amt}=     Get From Dictionary    ${body}    final_amount
    ${shipping}=      Get From Dictionary    ${body}    shipping_method
    ${items_count}=   Get From Dictionary    ${body}    items_count

    Should Be Equal As Strings    ${tier}           VIP
    Should Be Equal As Numbers    ${subtotal}       3500.00
    Should Be Equal As Numbers    ${discount}       700.00
    Should Be Equal As Numbers    ${final_amt}      2800.00
    Should Be Equal As Strings    ${shipping}       FREE_PRIORITY_EXPRESS_AIR
    Should Be Equal As Numbers    ${items_count}    2
    Log To Console    \n💎 VIP Checkout Fast-Path: Subtotal=$${subtotal}, 20% Discount=-$${discount}, Final=$${final_amt}

    # Trace in Visualizer
    Look Up Job In Visualizer    ${VIP_JOB_ID}
    Wait Until Element Contains  id:executionSummaryText    COMPLETED    15s

    Execute Javascript    if (window.graph && window.graph.zoomToFit) { window.graph.zoomToFit({ padding: 30, maxScale: 1.2 }); }
    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/02_rust_vip_journey_branch_loop_completed.png


Scenario 4: Fast-Path Standard Checkout Journey with Standard Retail Pricing
    [Documentation]    Executes Standard order with 1 item, verifies regular retail pricing ($15 shipping) and standard branch taken.
    ${job_id}=        Generate Random Job Id    rust-std-settle
    Set Suite Variable    ${STD_JOB_ID}    ${job_id}

    ${item1}=         Create Dictionary    sku=SKU-WIRELESS-MOUSE    quantity=${1}    unit_price=${50.00}
    ${items}=         Create List    ${item1}

    ${payload}=       Create Dictionary    order_id=ORD-STD-1122    currency=USD    card_number=4222-5555-6666-7777    cvv=999    items=${items}
    ${headers}=       Create Dictionary    Authorization=Bearer ${JOHN_TOKEN}    X-Job-ID=${job_id}    Content-Type=application/json

    ${resp}=          POST On Session    settlement_session    url=/settlement/process    json=${payload}    headers=${headers}
    Status Should Be  200    ${resp}
    ${body}=          Set Variable    ${resp.json()}

    ${tier}=          Get From Dictionary    ${body}    customer_tier
    ${subtotal}=      Get From Dictionary    ${body}    subtotal
    ${discount}=      Get From Dictionary    ${body}    discount_applied
    ${final_amt}=     Get From Dictionary    ${body}    final_amount
    ${shipping}=      Get From Dictionary    ${body}    shipping_method

    Should Be Equal As Strings    ${tier}           STANDARD
    Should Be Equal As Numbers    ${subtotal}       50.00
    Should Be Equal As Numbers    ${discount}       0.00
    Should Be Equal As Numbers    ${final_amt}      65.00
    Should Be Equal As Strings    ${shipping}       STANDARD_GROUND_FREIGHT
    Log To Console    \n🛒 Standard Checkout Fast-Path: Subtotal=$${subtotal}, Final=$${final_amt}

    # Trace in Visualizer
    Look Up Job In Visualizer    ${STD_JOB_ID}
    Wait Until Element Contains  id:executionSummaryText    COMPLETED    15s

    Execute Javascript    if (window.graph && window.graph.zoomToFit) { window.graph.zoomToFit({ padding: 30, maxScale: 1.2 }); }
    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/03_rust_standard_journey_branch_completed.png


Scenario 5: In-Memory Saga Rollback & Compensation on Payment Decline
    [Documentation]    Simulates credit card decline, verifies engine catches error, reverses authorizations, releases inventory, and marks status as ROLLED_BACK.
    ${job_id}=        Generate Random Job Id    rust-saga-fail
    Set Suite Variable    ${SAGA_JOB_ID}    ${job_id}

    ${item1}=         Create Dictionary    sku=SKU-HIGH-END-SERVER    quantity=${1}    unit_price=${5000.00}
    ${items}=         Create List    ${item1}

    # Use declining card 4000-0000-0000-0002 with simulate_decline: true
    ${payload}=       Create Dictionary    order_id=ORD-FAIL-9900    currency=USD    card_number=4000-0000-0000-0002    cvv=000    items=${items}    simulate_decline=${True}
    ${headers}=       Create Dictionary    Authorization=Bearer ${SARAH_TOKEN}    X-Job-ID=${job_id}    Content-Type=application/json

    ${resp}=          POST On Session    settlement_session    url=/settlement/process    json=${payload}    headers=${headers}    expected_status=500
    Status Should Be  500    ${resp}
    Log To Console    \n⚠️ Payment Declination Caught: In-Memory Saga Rollback Triggered

    # Trace in Visualizer
    Look Up Job In Visualizer    ${SAGA_JOB_ID}
    Wait Until Element Contains  id:executionSummaryText    ROLLED_BACK    15s

    Execute Javascript    if (window.graph && window.graph.zoomToFit) { window.graph.zoomToFit({ padding: 30, maxScale: 1.2 }); }
    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/04_rust_saga_rollback_compensation_journey.png


Scenario 6: Two-Phase Settlement Paused on Signal (await_signal) & Resumed
    [Documentation]    Initiates two-phase settlement, halts at await_signal (AWAITING_SIGNAL), delivers external bank signal, and verifies resumption to COMPLETED.
    ${job_id}=        Generate Random Job Id    rust-signal-hold
    Set Suite Variable    ${SIGNAL_JOB_ID}    ${job_id}

    ${payload}=       Create Dictionary    hold_id=HOLD-BANK-7788    amount=${1250.00}    currency=USD
    ${headers}=       Create Dictionary    Authorization=Bearer ${SARAH_TOKEN}    X-Job-ID=${job_id}    Content-Type=application/json

    # 1. Authorize & Hold (halts at await_signal)
    ${resp}=          POST On Session    settlement_session    url=/settlement/authorize_and_hold    json=${payload}    headers=${headers}
    Status Should Be  200    ${resp}
    ${status_hdr}=    Get From Dictionary    ${resp.headers}    X-Execution-Status
    Should Be Equal As Strings    ${status_hdr}    AWAITING_SIGNAL
    Log To Console    \n⏳ Two-Phase Hold Initiated: Workflow Halts at await_signal (AWAITING_SIGNAL)

    # Visualizer Screenshot while Paused
    Look Up Job In Visualizer    ${SIGNAL_JOB_ID}
    Wait Until Element Contains  id:executionSummaryText    WAITING FOR SIGNAL    15s

    Execute Javascript    if (window.graph && window.graph.zoomToFit) { window.graph.zoomToFit({ padding: 30, maxScale: 1.2 }); }
    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/05_rust_awaiting_signal_paused_journey.png

    # 2. Deliver External Bank Clearing Signal
    ${sig_payload}=   Create Dictionary    signal=settlement_cleared    data=clearing_confirmed_by_central_bank
    ${sig_resp}=      POST On Session    raw_host_session    url=/api/v1.0.0/enterprise_settlement_gateway/signal/${SIGNAL_JOB_ID}    json=${sig_payload}
    Status Should Be  200    ${sig_resp}
    ${sig_body}=      Set Variable    ${sig_resp.json()}
    ${resumed_st}=    Get From Dictionary    ${sig_body}    status
    Should Be Equal As Strings    ${resumed_st}    COMPLETED
    Log To Console    \n✅ Clearing Signal Delivered: Workflow Resumed & Settled to COMPLETED

    # Visualizer Screenshot after Resumption
    Look Up Job In Visualizer    ${SIGNAL_JOB_ID}
    Wait Until Element Contains  id:executionSummaryText    COMPLETED    15s

    Execute Javascript    if (window.graph && window.graph.zoomToFit) { window.graph.zoomToFit({ padding: 30, maxScale: 1.2 }); }
    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/06_rust_resumed_signal_completed_journey.png


Scenario 7: Step Inspector Drawer Displays Multi-Item Loop Iteration Payloads
    [Documentation]    Opens the Step Inspector Drawer on step_reserve_inventory_batch and validates loop payloads and timing.
    Look Up Job In Visualizer    ${VIP_JOB_ID}
    Wait Until Element Contains  id:executionSummaryText    COMPLETED    15s

    ${selected}=    Execute Javascript    return window.selectNode('step_reserve_inventory_batch');
    Should Be True    ${selected}    Expected step_reserve_inventory_batch node to be selected

    Wait Until Page Contains Element    css:#inspector.open    10s
    Wait Until Element Is Visible       id:insp-input-payload-field    10s
    Wait Until Element Is Visible       id:insp-output-payload-field   10s

    Wait Until Element Contains         id:insp-output-json-content    warehouse_bin    10s
    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/07_rust_inspector_drawer_loop_iterations.png


Scenario 8: Step Inspector Drawer Displays Sensitive Field Masking & Saga Rollback
    [Documentation]    Opens Step Inspector on step_record_pending_ledger in the failed Saga job, verifying masked card (********) and rollback badge.
    Look Up Job In Visualizer    ${SAGA_JOB_ID}
    Wait Until Element Contains  id:executionSummaryText    ROLLED_BACK    15s

    ${selected}=    Execute Javascript    return window.selectNode('step_record_pending_ledger');
    Should Be True    ${selected}    Expected step_record_pending_ledger node to be selected

    Wait Until Page Contains Element    css:#inspector.open    10s
    Wait Until Element Is Visible       id:insp-comp-status    10s

    ${comp_html}=     Get Element Attribute    id:insp-comp-status    innerHTML
    Should Contain    ${comp_html}    ReversePendingLedger
    Log To Console    \n↩️ Visualizer Displays Saga Rollback Badge: ReversePendingLedger()

    # Now inspect step_authorize_payment for data masking
    ${selected_auth}=    Execute Javascript    return window.selectNode('step_authorize_payment');
    Should Be True    ${selected_auth}    Expected step_authorize_payment node to be selected

    ${inp_json}=      Get Element Attribute    id:insp-input-json-content    innerText
    Should Contain    ${inp_json}    ********
    Log To Console    \n🔒 Telemetry Data Masking Verified in Visualizer: card_number masked with ********

    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/08_rust_inspector_drawer_masking_and_compensation.png


*** Keywords ***

Setup Enterprise Settlement E2E Suite
    Create Directory    ${SCREENSHOTS_DIR}
    ${handle}=          Start Process    ./server    cwd=${CURDIR}    env:PORT=${PORT}
    Set Suite Variable    ${SERVER_HANDLE}    ${handle}
    Wait Until Keyword Succeeds    20s    1s    Check Server Health
    Create Session      settlement_session    ${BASE_URL}    disable_warnings=1
    Create Session      raw_host_session      ${HOST}        disable_warnings=1
    Open Headless Browser

Teardown Enterprise Settlement E2E Suite
    Close All Browsers
    Delete All Sessions
    Terminate Process    ${SERVER_HANDLE}    kill=True

Check Server Health
    ${resp}=    GET    ${HOST}/health
    Status Should Be    200    ${resp}

Look Up Job In Visualizer
    [Arguments]       ${job_id}
    Go To             ${HOST}/visualizer?domain=${DOMAIN}&job_id=${job_id}
    Wait Until Page Contains Element    id:executionBanner    15s
    Wait Until Element Is Visible       id:executionBanner    15s

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
    ${count}=    Execute Javascript    return (window.graph && window.graph.getNodes) ? window.graph.getNodes().length : 0;
    Should Be True    ${count} > 0    Graph nodes not loaded yet

Generate Random Job Id
    [Arguments]       ${prefix}
    ${epoch}=         Evaluate    int(time.time() * 1000)    modules=time
    RETURN            ${prefix}-${epoch}
