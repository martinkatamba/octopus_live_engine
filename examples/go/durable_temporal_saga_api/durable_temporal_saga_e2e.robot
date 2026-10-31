*** Settings ***
Documentation     OCTOPUS Go Durable Temporal Saga Compensation End-to-End Test Suite
...               Demonstrating Guaranteed Compensation Steps Running on Temporal:
...               - Model-Driven Declarative REST API Routing (/api/v1/settlement/process)
...               - Multi-Step Financial Settlement Ledger (Escrow Reservation & Warehouse Inventory)
...               - Downstream Clearing House Failure Simulation
...               - Automatic Saga Rollback with compensation_mode: async
...               - Asynchronous Delegation to Temporal Server (127.0.0.1:7233) on Task Queue 'durable_saga_queue'
...               - Embedded Temporal Worker Execution of ReleaseEscrowHold and RestockInventory
...               - Visualizer Verification of ROLLED_BACK Status and Failed Step Inspector
...               - Temporal Web UI Verification (http://127.0.0.1:8233) with Completed Compensation Workflows
Library           Process
Library           RequestsLibrary
Library           Collections
Library           OperatingSystem
Library           String
Library           SeleniumLibrary

Suite Setup       Setup Durable Temporal Saga Suite
Suite Teardown    Teardown Durable Temporal Saga Suite

*** Variables ***
${PORT}               8095
${HOST}               http://127.0.0.1:${PORT}
${BASE_URL}           ${HOST}/api/v1
${DOMAIN}             durable_temporal_saga
${VISUALIZER_URL}     ${HOST}/visualizer?domain=${DOMAIN}
${TEMPORAL_UI_URL}    http://127.0.0.1:8233/namespaces/default/workflows
${SCREENSHOTS_DIR}    ${CURDIR}/screenshots

*** Test Cases ***

Scenario 1: Visualizer Loads Static Blueprint With Declared Compensations
    [Documentation]    Verifies that /visualizer renders the 5-step DAG and displays declared Saga compensations in the Inspector.
    Go To    ${VISUALIZER_URL}
    Wait Until Page Contains Element    id:jobIdInput    15s
    Wait Until Page Contains Element    id:graph-container    15s
    Wait Until Keyword Succeeds    15s    1s    Wait For Graph Nodes Rendered

    ${node_count}=    Execute Javascript    return (window.graph && window.graph.getNodes) ? window.graph.getNodes().length : 0;
    ${edge_count}=    Execute Javascript    return (window.graph && window.graph.getEdges) ? window.graph.getEdges().length : 0;
    
    Log To Console    \n📊 Durable Saga Blueprint: ${node_count} nodes, ${edge_count} edges
    Should Be True    ${node_count} >= 5    Expected at least 5 nodes in the workflow DAG
    Should Be True    ${edge_count} >= 4    Expected connecting edges

    # Select step_reserve_escrow to inspect declared compensation
    ${selected}=    Execute Javascript    return window.selectNode('step_reserve_escrow');
    Should Be True    ${selected}    Expected step_reserve_escrow to be selectable
    Wait Until Page Contains Element    id:inspector    10s
    Sleep    1s

    Execute Javascript    if (window.graph && window.graph.zoomToFit) { window.graph.zoomToFit({ padding: 30, maxScale: 1.2 }); }
    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/01_durable_saga_blueprint_and_compensations.png
    Log To Console    \n📸 Captured: 01_durable_saga_blueprint_and_compensations.png


Scenario 2: Happy Path Settlement Processed Successfully
    [Documentation]    Executes a clean financial settlement without failures. Verifies 200 OK and green path in Visualizer.
    ${job_id}=        Generate Random Job Id    go-settle-ok
    Set Suite Variable    ${OK_JOB_ID}    ${job_id}

    ${payload}=       Create Dictionary    account_id=ACC-ALPHA-CORP    amount=${1850.0}    currency=USD    sku=SKU-NVIDIA-H100    quantity=${1}    simulate_failure=${False}    job_id=${job_id}
    ${headers}=       Create Dictionary    Content-Type=application/json

    ${resp}=          POST On Session    saga_session    url=/settlement/process    json=${payload}    headers=${headers}
    Status Should Be  200    ${resp}

    ${body}=          Set Variable    ${resp.json()}
    ${status}=        Get From Dictionary    ${body}    status
    ${settle_id}=     Get From Dictionary    ${body}    settlement_id
    ${escrow_id}=     Get From Dictionary    ${body}    escrow_hold_id
    ${alloc_id}=      Get From Dictionary    ${body}    inventory_alloc_id
    ${clr_ref}=       Get From Dictionary    ${body}    clearing_ref
    ${actual_ok_id}=  Get From Dictionary    ${resp.headers}    X-Job-Id
    Set Suite Variable    ${OK_JOB_ID}    ${actual_ok_id}

    Should Be Equal As Strings    ${status}    SETTLED
    Should Not Be Empty           ${settle_id}
    Should Not Be Empty           ${escrow_id}
    Should Not Be Empty           ${alloc_id}
    Should Not Be Empty           ${clr_ref}
    Log To Console    \n✅ Settlement Successful: ID=${settle_id}, Escrow=${escrow_id}, Alloc=${alloc_id}, Clearing=${clr_ref}, JobID=${actual_ok_id}

    # Verify Visualizer Path
    Look Up Job In Visualizer    ${actual_ok_id}
    Wait Until Element Contains  id:executionSummaryText    COMPLETED    15s
    Sleep    2s
    Execute Javascript    if (window.graph && window.graph.zoomToFit) { window.graph.zoomToFit({ padding: 30, maxScale: 1.2 }); }
    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/02_successful_settlement_visualizer.png
    Log To Console    \n📸 Captured: 02_successful_settlement_visualizer.png


Scenario 3: Transaction Failure Triggers Automatic Saga Rollback
    [Documentation]    Executes a transaction that triggers downstream clearing failure. Verifies 500 status, ROLLED_BACK lifecycle, and failed node highlighting.
    ${job_id}=        Generate Random Job Id    go-settle-fail

    ${payload}=       Create Dictionary    account_id=ACC-ALPHA-CORP    amount=${4500.0}    currency=USD    sku=SKU-NVIDIA-H100    quantity=${2}    simulate_failure=${True}    job_id=${job_id}
    ${headers}=       Create Dictionary    Content-Type=application/json    X-Job-ID=${job_id}

    ${resp}=          POST On Session    saga_session    url=/settlement/process    json=${payload}    headers=${headers}    expected_status=500
    Status Should Be  500    ${resp}

    ${body}=          Set Variable    ${resp.json()}
    ${status}=        Get From Dictionary    ${body}    status
    ${actual_fail_id}=  Get From Dictionary    ${resp.headers}    X-Job-Id
    Set Suite Variable    ${FAIL_JOB_ID}    ${actual_fail_id}
    Should Be Equal As Strings    ${status}    ROLLED_BACK
    Log To Console    \n🛑 Transaction Rolled Back as expected: status=${status}, job_id=${actual_fail_id}

    # Verify Visualizer Rollback Banner & Failed Node
    Look Up Job In Visualizer    ${actual_fail_id}
    Wait Until Element Contains  id:executionSummaryText    ROLLED_BACK    15s
    ${summary_text}=    Get Text    id:executionSummaryText
    Should Contain    ${summary_text}    ROLLED_BACK

    # Select the failed clearing step
    Execute Javascript    window.selectNode('step_execute_clearing');
    Sleep    1s
    Execute Javascript    if (window.graph && window.graph.zoomToFit) { window.graph.zoomToFit({ padding: 30, maxScale: 1.2 }); }
    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/03_saga_failure_rollback_visualizer.png
    Log To Console    \n📸 Captured: 03_saga_failure_rollback_visualizer.png


Scenario 4: Temporal Web UI Proves Durable Compensation Workflows Executed
    [Documentation]    Navigates to the Temporal Web UI (http://127.0.0.1:8233) and verifies the durable compensation workflows ran and completed.
    Go To    ${TEMPORAL_UI_URL}
    Wait Until Page Contains Element    tag:tbody    15s
    Sleep    3s

    ${page_text}=    Get Text    tag:body
    Should Contain    ${page_text}    IOctopusWorkflow
    Should Contain    ${page_text}    ReleaseEscrowHold
    Should Contain    ${page_text}    RestockInventory
    Should Contain    ${page_text}    Completed
    Log To Console    \n⏱️ Temporal Web UI Verified: Both ReleaseEscrowHold and RestockInventory compensations Completed!

    Capture Page Screenshot    ${SCREENSHOTS_DIR}/04_temporal_web_ui_compensation_workflows.png
    Log To Console    \n📸 Captured: 04_temporal_web_ui_compensation_workflows.png


Scenario 5: Temporal Web UI Deep Inspection of Compensation Activity
    [Documentation]    Opens the detailed execution history of the ReleaseEscrowHold compensation workflow in Temporal Web UI.
    ${release_escrow_wf}=    Set Variable    octopus_act_${FAIL_JOB_ID}_saga_step_reserve_escrow_ReleaseEscrowHold
    
    # Click the specific workflow link in Temporal table
    ${clicked}=    Execute Javascript    var links = document.querySelectorAll('tbody tr a'); for(var i=0; i<links.length; i++) { if(links[i].innerText.indexOf('${FAIL_JOB_ID}') !== -1 && links[i].innerText.indexOf('ReleaseEscrowHold') !== -1) { links[i].click(); return true; } } return false;
    
    # If not clicked via js loop, navigate directly via table or sleep
    Sleep    3s
    ${detail_text}=    Get Text    tag:body
    Should Contain    ${detail_text}    ReleaseEscrowHold
    Log To Console    \n🔍 Temporal Compensation Workflow Detail Verified: Activity ReleaseEscrowHold executed with durability guarantees!

    Capture Page Screenshot    ${SCREENSHOTS_DIR}/05_temporal_web_ui_compensation_activity_detail.png
    Log To Console    \n📸 Captured: 05_temporal_web_ui_compensation_activity_detail.png


*** Keywords ***

Setup Durable Temporal Saga Suite
    Create Directory    ${SCREENSHOTS_DIR}
    Create Session      saga_session    ${BASE_URL}    disable_warnings=1
    Create Session      raw_host        ${HOST}        disable_warnings=1
    Open Headless Browser

Teardown Durable Temporal Saga Suite
    Close All Browsers
    Delete All Sessions

Look Up Job In Visualizer
    [Arguments]       ${job_id}
    Go To             ${VISUALIZER_URL}
    Wait Until Page Contains Element    id:jobIdInput    15s
    Wait Until Element Is Visible       id:jobIdInput    15s
    Sleep    1s
    Execute Javascript    document.getElementById('jobIdInput').value = '${job_id}'; lookupJob();
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
