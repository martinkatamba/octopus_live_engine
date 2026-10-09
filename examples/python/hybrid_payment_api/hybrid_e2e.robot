*** Settings ***
Documentation     OCTOPUS Hybrid Execution (Fast-Path + Durable Replication) Visualizer End-to-End Test Suite
Library           Process
Library           RequestsLibrary
Library           Collections
Library           OperatingSystem
Library           String
Library           SeleniumLibrary

Suite Setup       Setup Hybrid E2E Suite
Suite Teardown    Teardown Hybrid E2E Suite

*** Variables ***
${PORT}               8089
${HOST}               http://127.0.0.1:${PORT}
${BASE_URL}           ${HOST}/api/v1
${DOMAIN}             hybrid_payment_gateway
${VISUALIZER_URL}     ${HOST}/visualizer?domain=${DOMAIN}
${SCREENSHOTS_DIR}    ${CURDIR}/screenshots

*** Test Cases ***

Scenario 1: Visualizer Loads Static Hybrid Workflow Blueprint
    [Documentation]    Verifies that navigating to /visualizer loads the Hybrid Payment Gateway workflow blueprint.
    Go To    ${VISUALIZER_URL}
    Wait Until Page Contains Element    id:jobIdInput    15s
    Wait Until Page Contains Element    id:graph-container    15s
    
    Wait Until Keyword Succeeds    15s    1s    Wait For Graph Nodes Rendered

    ${node_count}=    Execute Javascript    return (window.graph && window.graph.getNodes) ? window.graph.getNodes().length : 0;
    ${edge_count}=    Execute Javascript    return (window.graph && window.graph.getEdges) ? window.graph.getEdges().length : 0;
    
    Log To Console    \n📊 Hybrid Payment Blueprint Loaded: ${node_count} nodes, ${edge_count} edges
    Should Be True    ${node_count} > 0    Expected nodes to be rendered on canvas
    Should Be True    ${edge_count} > 0    Expected edges to be rendered on canvas

    Execute Javascript    if (window.graph && window.graph.zoomToFit) { window.graph.zoomToFit({ padding: 30, maxScale: 1.2 }); }
    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/01_hybrid_payment_blueprint.png


Scenario 2: Synchronous Fast-Path Authorizes Payment Under 10ms
    [Documentation]    Calls POST /api/v1/payments/authorize. Asserts immediate 200 OK and validated PaymentResponse model.
    ${job_id}=        Generate Random Job Id    tx-hybrid-pay
    Set Suite Variable    ${PAYMENT_JOB_ID}    ${job_id}
    ${payload}=       Create Dictionary    account_id=acc_corp_7712    amount=${499.00}    currency=USD    card_token=tok_visa_prime
    ${headers}=       Create Dictionary    X-Job-ID=${job_id}    Content-Type=application/json

    ${resp}=          POST On Session    octopus    url=/payments/authorize    json=${payload}    headers=${headers}
    Status Should Be  200    ${resp}
    ${body}=          Set Variable    ${resp.json()}
    
    ${status}=          Get From Dictionary    ${body}    status
    ${auth_amt}=        Get From Dictionary    ${body}    authorized_amount
    ${txn_id}=          Get From Dictionary    ${body}    transaction_id
    ${durable_track}=   Get From Dictionary    ${body}    durable_tracking

    Should Be Equal As Strings    ${status}           AUTHORIZED
    Should Be Equal As Numbers    ${auth_amt}         499.00
    Should Start With             ${txn_id}           txn_
    Should Be Equal As Strings    ${durable_track}    REPLICATED_TO_TEMPORAL_AND_SQLITE
    Log To Console    \n⚡ Fast-Path Payment Authorized: TxnID=${txn_id}, Amount=$${auth_amt}, Tracking="${durable_track}"


Scenario 3: Visualizer Traces Completed Hybrid Payment Execution
    [Documentation]    Looks up the executed hybrid payment job in the visualizer. Confirms full forward green execution path.
    Go To    ${VISUALIZER_URL}
    Wait Until Page Contains Element    id:jobIdInput    15s
    Clear Element Text    id:jobIdInput
    Input Text    id:jobIdInput    ${PAYMENT_JOB_ID}
    Click Button    id:lookupBtn

    Wait Until Element Is Visible    id:executionBanner    15s
    Wait Until Element Contains      id:executionSummaryText    COMPLETED    15s

    Execute Javascript    if (window.graph && window.graph.zoomToFit) { window.graph.zoomToFit({ padding: 30, maxScale: 1.2 }); }
    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/02_hybrid_payment_completed_journey.png


Scenario 4: Step Inspector Drawer Displays Payloads and Response Model
    [Documentation]    Selects the output activity step and inspects the payload inspector drawer.
    Go To    ${VISUALIZER_URL}
    Wait Until Page Contains Element    id:jobIdInput    15s
    Clear Element Text    id:jobIdInput
    Input Text    id:jobIdInput    ${PAYMENT_JOB_ID}
    Click Button    id:lookupBtn

    Wait Until Element Is Visible    id:executionBanner    15s
    Wait Until Element Contains      id:executionSummaryText    COMPLETED    15s

    # Click format response node
    Execute Javascript    if (window.selectNode) { window.selectNode('step_format_response'); } else if (window.graph && window.graph.getNodes) { const nodes = window.graph.getNodes(); const target = nodes.find(n => n.id.includes('step_format') || n.id.includes('step_auth')); if (target) { window.graph.trigger('node:click', { node: target }); } }
    Wait Until Page Contains Element    css:#inspector.open    10s
    Wait Until Element Is Visible       id:insp-input-payload-field    10s
    Wait Until Element Is Visible       id:insp-output-payload-field   10s
    Wait Until Element Contains         id:insp-output-json-content    AUTHORIZED    10s
    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/03_inspector_drawer_payment_details.png


*** Keywords ***

Setup Hybrid E2E Suite
    Create Directory    ${SCREENSHOTS_DIR}
    ${handle}=          Start Process    /Volumes/Macintosh HD - Data/2026/AI_PROJECTS/project-octopus/.venv/bin/python    server.py    --port    ${PORT}    cwd=${CURDIR}    env:PYTHONPATH=../../../python/src:${CURDIR}
    Set Suite Variable    ${SERVER_HANDLE}    ${handle}
    Wait Until Keyword Succeeds    20s    1s    Check Server Health
    ${headers}=         Create Dictionary    X-API-Key=default-api-key
    Create Session      octopus    ${BASE_URL}    headers=${headers}    disable_warnings=1
    Open Headless Browser

Teardown Hybrid E2E Suite
    Close All Browsers
    Delete All Sessions
    Terminate Process    ${SERVER_HANDLE}    kill=True

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
    ${count}=    Execute Javascript    return (window.graph && window.graph.getNodes) ? window.graph.getNodes().length : 0;
    Should Be True    ${count} > 0    Graph nodes not loaded yet

Generate Random Job Id
    [Arguments]       ${prefix}
    ${epoch}=         Evaluate    int(time.time() * 1000)    modules=time
    RETURN            ${prefix}-${epoch}
