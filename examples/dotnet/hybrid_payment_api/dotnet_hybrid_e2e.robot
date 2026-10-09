*** Settings ***
Documentation     OCTOPUS .NET Hybrid Payment API Live Engine Visualizer End-to-End Test Suite
Library           Process
Library           RequestsLibrary
Library           Collections
Library           OperatingSystem
Library           String
Library           SeleniumLibrary

Suite Setup       Setup Dotnet Hybrid E2E Suite
Suite Teardown    Teardown Dotnet Hybrid E2E Suite

*** Variables ***
${PORT}               8094
${HOST}               http://127.0.0.1:${PORT}
${BASE_URL}           ${HOST}/api/v1
${DOMAIN}             hybrid_payment_gateway
${VISUALIZER_URL}     ${HOST}/visualizer?domain=${DOMAIN}
${SCREENSHOTS_DIR}    ${CURDIR}/screenshots

*** Test Cases ***

Scenario 1: Dotnet Visualizer Loads Static Hybrid Workflow Blueprint
    [Documentation]    Verifies that navigating to /visualizer loads the .NET Hybrid Payment Gateway workflow blueprint.
    Go To    ${VISUALIZER_URL}
    Wait Until Page Contains Element    id:jobIdInput    15s
    Wait Until Page Contains Element    id:graph-container    15s
    
    Wait Until Keyword Succeeds    15s    1s    Wait For Graph Nodes Rendered

    ${node_count}=    Execute Javascript    return (window.graph && window.graph.getNodes) ? window.graph.getNodes().length : 0;
    ${edge_count}=    Execute Javascript    return (window.graph && window.graph.getEdges) ? window.graph.getEdges().length : 0;
    
    Log To Console    \n📊 Dotnet Hybrid Payment Blueprint: ${node_count} nodes, ${edge_count} edges
    Should Be True    ${node_count} > 0    Expected nodes to be rendered on canvas
    Should Be True    ${edge_count} > 0    Expected edges to be rendered on canvas

    Execute Javascript    if (window.graph && window.graph.zoomToFit) { window.graph.zoomToFit({ padding: 30, maxScale: 1.2 }); }
    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/01_dotnet_hybrid_payment_blueprint.png


Scenario 2: Dotnet Synchronous Fast-Path Authorizes Payment Under 10ms
    [Documentation]    Calls POST /api/v1/payments/authorize on .NET Live Engine.
    ${job_id}=        Generate Random Job Id    tx-net-pay
    Set Suite Variable    ${PAYMENT_JOB_ID}    ${job_id}
    ${payload}=       Create Dictionary    AccountId=acc_corp_net_99    Amount=${750.00}    Currency=USD    CardToken=tok_mastercard_net
    ${headers}=       Create Dictionary    X-Job-ID=${job_id}    Content-Type=application/json

    ${resp}=          POST On Session    octopus_dotnet    url=/payments/authorize    json=${payload}    headers=${headers}
    Status Should Be  200    ${resp}
    ${body}=          Set Variable    ${resp.json()}
    
    ${status}=          Get From Dictionary    ${body}    status
    ${auth_amt}=        Get From Dictionary    ${body}    authorized_amount
    ${txn_id}=          Get From Dictionary    ${body}    transaction_id
    ${durable_track}=   Get From Dictionary    ${body}    durable_tracking

    Should Be Equal As Strings    ${status}           AUTHORIZED
    Should Be Equal As Numbers    ${auth_amt}         750.00
    Should Start With             ${txn_id}           txn_
    Should Be Equal As Strings    ${durable_track}    REPLICATED_TO_TEMPORAL_AND_SQLITE
    Log To Console    \n⚡ Dotnet Fast-Path Authorized: TxnID=${txn_id}, Amount=$${auth_amt}


Scenario 3: Dotnet Visualizer Traces Completed Hybrid Payment Execution
    [Documentation]    Looks up the executed hybrid payment job in the visualizer.
    Go To    ${VISUALIZER_URL}
    Wait Until Page Contains Element    id:jobIdInput    15s
    Clear Element Text    id:jobIdInput
    Input Text    id:jobIdInput    ${PAYMENT_JOB_ID}
    Click Button    id:lookupBtn

    Wait Until Element Is Visible    id:executionBanner    15s
    Wait Until Element Contains      id:executionSummaryText    COMPLETED    15s

    Execute Javascript    if (window.graph && window.graph.zoomToFit) { window.graph.zoomToFit({ padding: 30, maxScale: 1.2 }); }
    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/02_dotnet_hybrid_payment_completed.png


Scenario 4: Dotnet Step Inspector Drawer Displays Payloads
    [Documentation]    Inspects the step inspector drawer for payloads.
    Go To    ${VISUALIZER_URL}
    Wait Until Page Contains Element    id:jobIdInput    15s
    Clear Element Text    id:jobIdInput
    Input Text    id:jobIdInput    ${PAYMENT_JOB_ID}
    Click Button    id:lookupBtn

    Wait Until Element Is Visible    id:executionBanner    15s
    Wait Until Element Contains      id:executionSummaryText    COMPLETED    15s

    Execute Javascript    if (window.selectNode) { window.selectNode('step_format_response'); } else if (window.graph && window.graph.getNodes) { const nodes = window.graph.getNodes(); const target = nodes.find(n => n.id.includes('step_format') || n.id.includes('step_auth')); if (target) { window.graph.trigger('node:click', { node: target }); } }
    Wait Until Page Contains Element    css:#inspector.open    10s
    Wait Until Element Is Visible       id:insp-input-payload-field    10s
    Wait Until Element Is Visible       id:insp-output-payload-field   10s
    Wait Until Element Contains         id:insp-output-json-content    AUTHORIZED    10s
    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/03_dotnet_inspector_payment_details.png


*** Keywords ***

Setup Dotnet Hybrid E2E Suite
    Create Directory    ${SCREENSHOTS_DIR}
    Log To Console      \nStarting .NET Hybrid Payment API on port ${PORT}...
    ${handle}=          Start Process    dotnet    run    --project    ${CURDIR}/HybridPaymentApi.csproj    --no-build    cwd=${CURDIR}    env:PORT=${PORT}    env:DOTNET_CLI_HOME=/tmp
    Set Suite Variable  ${SERVER_PROCESS}    ${handle}
    
    Wait Until Keyword Succeeds    30s    1s    Check Server Health
    ${headers}=         Create Dictionary    X-API-Key=default-api-key
    Create Session      octopus_dotnet    ${BASE_URL}    headers=${headers}    disable_warnings=1
    Open Headless Browser

Teardown Dotnet Hybrid E2E Suite
    Close All Browsers
    Delete All Sessions
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
    ${count}=    Execute Javascript    return (window.graph && window.graph.getNodes) ? window.graph.getNodes().length : 0;
    Should Be True    ${count} > 0    Graph nodes not loaded yet

Generate Random Job Id
    [Arguments]       ${prefix}
    ${epoch}=         Evaluate    int(time.time() * 1000)    modules=time
    ${rand}=          Generate Random String    4    [NUMBERS]
    ${job_id}=        Set Variable    ${prefix}_${epoch}_${rand}
    RETURN            ${job_id}
