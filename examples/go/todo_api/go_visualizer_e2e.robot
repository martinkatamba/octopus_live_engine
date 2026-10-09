*** Settings ***
Documentation     OCTOPUS Go Todo REST API Live Engine Visualizer End-to-End Test Suite
Library           Process
Library           RequestsLibrary
Library           Collections
Library           OperatingSystem
Library           String
Library           SeleniumLibrary

Suite Setup       Setup Go Visualizer E2E Suite
Suite Teardown    Teardown Go Visualizer E2E Suite

*** Variables ***
${PORT}               8093
${HOST}               http://127.0.0.1:${PORT}
${BASE_URL}           ${HOST}/api/v1
${DOMAIN}             todo_api_service
${VISUALIZER_URL}     ${HOST}/visualizer?domain=${DOMAIN}
${SCREENSHOTS_DIR}    ${CURDIR}/screenshots

*** Test Cases ***

Scenario 1: Go Visualizer Loads Static Blueprint Graph
    [Documentation]    Verifies that navigating to /visualizer loads the Go Todo workflow blueprint graph.
    Go To    ${VISUALIZER_URL}
    Wait Until Page Contains Element    id:jobIdInput    15s
    Wait Until Page Contains Element    id:graph-container    15s
    
    Wait Until Keyword Succeeds    15s    1s    Wait For Graph Nodes Rendered

    ${node_count}=    Execute Javascript    return (window.graph && window.graph.getNodes) ? window.graph.getNodes().length : 0;
    ${edge_count}=    Execute Javascript    return (window.graph && window.graph.getEdges) ? window.graph.getEdges().length : 0;
    
    Log To Console    \n📊 Go Todo Blueprint: ${node_count} nodes, ${edge_count} edges
    Should Be True    ${node_count} > 0    Expected nodes to be rendered on canvas
    Should Be True    ${edge_count} > 0    Expected edges to be rendered on canvas

    Execute Javascript    if (window.graph && window.graph.zoomToFit) { window.graph.zoomToFit({ padding: 30, maxScale: 1.2 }); }
    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/01_go_todo_blueprint.png


Scenario 2: Go Synchronous POST Creates a New Todo Item
    [Documentation]    Calls POST /api/v1/todos synchronously on Go engine.
    ${job_id}=        Generate Random Job Id    tx-go-todo-create
    Set Suite Variable    ${CREATE_JOB_ID}    ${job_id}
    ${payload}=       Create Dictionary    title=Go E2E Todo Item    description=Verified via Go Live Engine
    ${headers}=       Create Dictionary    X-Job-ID=${job_id}    Content-Type=application/json

    ${resp}=          POST On Session    octopus_go_todo    url=/todos    json=${payload}    headers=${headers}
    Status Should Be  201    ${resp}
    ${body}=          Set Variable    ${resp.json()}
    
    ${todo_id}=       Get From Dictionary    ${body}    id
    ${title}=         Get From Dictionary    ${body}    title
    ${completed}=     Get From Dictionary    ${body}    completed

    Set Suite Variable    ${TODO_ID}    ${todo_id}
    Should Start With             ${todo_id}    todo_
    Should Be Equal As Strings    ${title}      Go E2E Todo Item
    Should Be Equal               ${completed}  ${False}
    Log To Console    \n⚡ Go Todo Created: ID=${todo_id}, Title="${title}"


Scenario 3: Go Visualizer Traces Completed Todo Creation Workflow
    [Documentation]    Looks up the executed creation job in the visualizer.
    Go To    ${VISUALIZER_URL}
    Wait Until Page Contains Element    id:jobIdInput    15s
    Clear Element Text    id:jobIdInput
    Input Text    id:jobIdInput    ${CREATE_JOB_ID}
    Click Button    id:lookupBtn

    Wait Until Element Is Visible    id:executionBanner    15s
    Wait Until Element Contains      id:executionSummaryText    COMPLETED    15s

    Execute Javascript    if (window.graph && window.graph.zoomToFit) { window.graph.zoomToFit({ padding: 30, maxScale: 1.2 }); }
    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/02_go_todo_creation_journey.png


Scenario 4: Go Synchronous GET Retrieves Active Todo List
    [Documentation]    Calls GET /api/v1/todos on Go engine.
    ${headers}=       Create Dictionary    X-Job-ID=tx-go-list-001

    ${resp}=          GET On Session    octopus_go_todo    url=/todos    headers=${headers}
    Status Should Be  200    ${resp}
    ${body}=          Set Variable    ${resp.json()}
    
    ${count}=         Get From Dictionary    ${body}    total_count
    Should Be True    ${count} >= 1
    Log To Console    \n📋 Go Todo List Fetched: ${count} item(s) found


Scenario 5: Go Visualizer Step Details Inspection
    [Documentation]    Opens step inspector drawer on Go engine visualizer.
    Go To    ${VISUALIZER_URL}
    Wait Until Page Contains Element    id:jobIdInput    15s
    Clear Element Text    id:jobIdInput
    Input Text    id:jobIdInput    ${CREATE_JOB_ID}
    Click Button    id:lookupBtn

    Wait Until Element Is Visible    id:executionBanner    15s
    Wait Until Element Contains      id:executionSummaryText    COMPLETED    15s

    Execute Javascript    if (window.selectNode) { window.selectNode('step_package'); } else if (window.graph && window.graph.getNodes) { const nodes = window.graph.getNodes(); const target = nodes.find(n => n.id.includes('step_package') || n.id.includes('step_persist')); if (target) { window.graph.trigger('node:click', { node: target }); } }
    Wait Until Page Contains Element    css:#inspector.open    10s
    Wait Until Element Is Visible       id:insp-output-payload-field   10s
    Wait Until Element Contains         id:insp-output-json-content    Go E2E Todo Item    10s
    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/03_go_todo_inspector_details.png


Scenario 6: Go Synchronous DELETE Removes Todo Item
    [Documentation]    Calls DELETE /api/v1/todos/{todo_id} on Go engine.
    ${headers}=       Create Dictionary    X-Job-ID=tx-go-del-001

    ${resp}=          DELETE On Session    octopus_go_todo    url=/todos/${TODO_ID}    headers=${headers}
    Status Should Be  200    ${resp}
    ${body}=          Set Variable    ${resp.json()}
    
    ${deleted}=       Get From Dictionary    ${body}    deleted
    Should Be Equal   ${deleted}             ${True}
    Log To Console    \n🗑️ Go Todo Deleted: ID=${TODO_ID}


*** Keywords ***

Setup Go Visualizer E2E Suite
    Create Directory    ${SCREENSHOTS_DIR}
    ${handle}=          Start Process    go    run    main.go    cwd=${CURDIR}    env:PORT=${PORT}
    Set Suite Variable    ${SERVER_HANDLE}    ${handle}
    Wait Until Keyword Succeeds    20s    1s    Check Server Health
    ${headers}=         Create Dictionary    X-API-Key=default-api-key
    Create Session      octopus_go_todo    ${BASE_URL}    headers=${headers}    disable_warnings=1
    Open Headless Browser

Teardown Go Visualizer E2E Suite
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
