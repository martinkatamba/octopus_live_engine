*** Settings ***
Documentation     OCTOPUS Todo REST API (Python Live Engine) Visualizer End-to-End Test Suite
Library           Process
Library           RequestsLibrary
Library           Collections
Library           OperatingSystem
Library           String
Library           SeleniumLibrary

Suite Setup       Setup Visualizer E2E Suite
Suite Teardown    Teardown Visualizer E2E Suite

*** Variables ***
${PORT}               8088
${HOST}               http://127.0.0.1:${PORT}
${BASE_URL}           ${HOST}/api/v1
${DOMAIN}             todo_api_service
${VISUALIZER_URL}     ${HOST}/visualizer?domain=${DOMAIN}
${SCREENSHOTS_DIR}    ${CURDIR}/screenshots

*** Test Cases ***

Scenario 1: Visualizer Loads Static Synchronous Blueprint Graph
    [Documentation]    Verifies that navigating to /visualizer loads the Todo workflow blueprint graph.
    Go To    ${VISUALIZER_URL}
    Wait Until Page Contains Element    id:jobIdInput    15s
    Wait Until Page Contains Element    id:graph-container    15s
    
    Wait Until Keyword Succeeds    15s    1s    Wait For Graph Nodes Rendered

    ${node_count}=    Execute Javascript    return (window.graph && window.graph.getNodes) ? window.graph.getNodes().length : 0;
    ${edge_count}=    Execute Javascript    return (window.graph && window.graph.getEdges) ? window.graph.getEdges().length : 0;
    
    Log To Console    \n📊 Todo Blueprint Loaded: ${node_count} nodes, ${edge_count} edges
    Should Be True    ${node_count} > 0    Expected nodes to be rendered on canvas
    Should Be True    ${edge_count} > 0    Expected edges to be rendered on canvas

    Execute Javascript    if (window.graph && window.graph.zoomToFit) { window.graph.zoomToFit({ padding: 30, maxScale: 1.2 }); }
    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/01_static_todo_blueprint.png


Scenario 2: Synchronous POST Creates a New Todo Item
    [Documentation]    Calls POST /api/v1/todos synchronously. Asserts 201 Created and validated TodoResponse model.
    ${job_id}=        Generate Random Job Id    tx-todo-create
    Set Suite Variable    ${CREATE_JOB_ID}    ${job_id}
    ${payload}=       Create Dictionary    title=Buy groceries    description=Milk, eggs, and sourdough bread
    ${headers}=       Create Dictionary    X-Job-ID=${job_id}    Content-Type=application/json

    ${resp}=          POST On Session    octopus    url=/todos    json=${payload}    headers=${headers}
    Status Should Be  201    ${resp}
    ${body}=          Set Variable    ${resp.json()}
    
    ${todo_id}=       Get From Dictionary    ${body}    id
    ${title}=         Get From Dictionary    ${body}    title
    ${completed}=     Get From Dictionary    ${body}    completed
    ${created_at}=    Get From Dictionary    ${body}    created_at

    Set Suite Variable    ${TODO_ID}    ${todo_id}
    Should Start With             ${todo_id}    todo_
    Should Be Equal As Strings    ${title}      Buy groceries
    Should Be Equal               ${completed}  ${False}
    Should Not Be Empty           ${created_at}
    Log To Console    \n⚡ Todo Created: ID=${todo_id}, Title="${title}", JobID=${job_id}


Scenario 3: Visualizer Traces Completed Todo Creation Workflow
    [Documentation]    Looks up the executed creation job in the visualizer. Confirms full forward green execution path.
    Go To    ${VISUALIZER_URL}
    Wait Until Page Contains Element    id:jobIdInput    15s
    Clear Element Text    id:jobIdInput
    Input Text    id:jobIdInput    ${CREATE_JOB_ID}
    Click Button    id:lookupBtn

    Wait Until Element Is Visible    id:executionBanner    15s
    Wait Until Element Contains      id:executionSummaryText    COMPLETED    15s

    Execute Javascript    if (window.graph && window.graph.zoomToFit) { window.graph.zoomToFit({ padding: 30, maxScale: 1.2 }); }
    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/02_create_todo_journey.png


Scenario 4: Synchronous GET Retrieves Active Todo List
    [Documentation]    Calls GET /api/v1/todos. Asserts 200 OK and total_count = 1.
    ${job_id}=        Generate Random Job Id    tx-todo-list
    ${headers}=       Create Dictionary    X-Job-ID=${job_id}

    ${resp}=          GET On Session    octopus    url=/todos    headers=${headers}
    Status Should Be  200    ${resp}
    ${body}=          Set Variable    ${resp.json()}
    
    ${count}=         Get From Dictionary    ${body}    total_count
    ${todos}=         Get From Dictionary    ${body}    todos

    Should Be Equal As Integers    ${count}    1
    ${first}=         Get From List    ${todos}    0
    ${first_id}=      Get From Dictionary    ${first}    id
    Should Be Equal As Strings     ${first_id}    ${TODO_ID}
    Log To Console    \n📋 Todo List Fetched: ${count} item(s) found


Scenario 5: Synchronous PUT Updates Todo to Completed Status
    [Documentation]    Calls PUT /api/v1/todos/{todo_id} to mark todo as completed. Asserts 200 OK.
    ${job_id}=        Generate Random Job Id    tx-todo-update
    Set Suite Variable    ${UPDATE_JOB_ID}    ${job_id}
    ${payload}=       Create Dictionary    title=Buy organic groceries    completed=${True}
    ${headers}=       Create Dictionary    X-Job-ID=${job_id}    Content-Type=application/json

    ${resp}=          PUT On Session    octopus    url=/todos/${TODO_ID}    json=${payload}    headers=${headers}
    Status Should Be  200    ${resp}
    ${body}=          Set Variable    ${resp.json()}
    
    ${updated_title}=  Get From Dictionary    ${body}    title
    ${is_done}=        Get From Dictionary    ${body}    completed

    Should Be Equal As Strings    ${updated_title}    Buy organic groceries
    Should Be Equal               ${is_done}          ${True}
    Log To Console    \n✏️ Todo Updated: ID=${TODO_ID}, Completed=${is_done}


Scenario 6: Visualizer Traces Update Workflow and Inspects Step Details
    [Documentation]    Looks up the update job in the visualizer and opens step inspector details.
    Go To    ${VISUALIZER_URL}
    Wait Until Page Contains Element    id:jobIdInput    15s
    Clear Element Text    id:jobIdInput
    Input Text    id:jobIdInput    ${UPDATE_JOB_ID}
    Click Button    id:lookupBtn

    Wait Until Element Is Visible    id:executionBanner    15s
    Wait Until Element Contains      id:executionSummaryText    COMPLETED    15s

    # Select the output node to open the inspector drawer
    Execute Javascript    if (window.selectNode) { window.selectNode('step_package'); } else if (window.graph && window.graph.getNodes) { const nodes = window.graph.getNodes(); const target = nodes.find(n => n.id.includes('step_package') || n.id.includes('step_update')); if (target) { window.graph.trigger('node:click', { node: target }); } }
    Wait Until Page Contains Element    css:#inspector.open    10s
    Wait Until Element Is Visible       id:insp-input-payload-field    10s
    Wait Until Element Is Visible       id:insp-output-payload-field   10s
    Wait Until Element Contains         id:insp-output-json-content    Buy organic groceries    10s
    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/03_inspector_drawer_todo_details.png


Scenario 7: Synchronous DELETE Removes Todo Item
    [Documentation]    Calls DELETE /api/v1/todos/{todo_id}. Asserts 200 OK and deleted = true.
    ${job_id}=        Generate Random Job Id    tx-todo-delete
    ${headers}=       Create Dictionary    X-Job-ID=${job_id}

    ${resp}=          DELETE On Session    octopus    url=/todos/${TODO_ID}    headers=${headers}
    Status Should Be  200    ${resp}
    ${body}=          Set Variable    ${resp.json()}
    
    ${deleted}=       Get From Dictionary    ${body}    deleted
    Should Be Equal   ${deleted}             ${True}
    Log To Console    \n🗑️ Todo Deleted: ID=${TODO_ID}


Scenario 8: Visualizer Alert for Non-Existent Job ID
    [Documentation]    Searches for a non-existent job ID in the visualizer to confirm error handling.
    Go To    ${VISUALIZER_URL}
    Wait Until Page Contains Element    id:jobIdInput    15s
    Clear Element Text    id:jobIdInput
    Input Text    id:jobIdInput    non-existent-todo-job-99999
    Click Button    id:lookupBtn

    Wait Until Element Is Visible    id:executionBanner    15s
    Wait Until Element Contains      id:executionSummaryText    Not Found    15s

    Execute Javascript    if (window.graph && window.graph.zoomToFit) { window.graph.zoomToFit({ padding: 30, maxScale: 1.2 }); }
    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/04_job_not_found_alert.png


*** Keywords ***

Setup Visualizer E2E Suite
    Create Directory    ${SCREENSHOTS_DIR}
    ${handle}=          Start Process    /Volumes/Macintosh HD - Data/2026/AI_PROJECTS/project-octopus/.venv/bin/python    server.py    --port    ${PORT}    cwd=${CURDIR}    env:PYTHONPATH=../../../python/src:${CURDIR}
    Set Suite Variable    ${SERVER_HANDLE}    ${handle}
    Wait Until Keyword Succeeds    20s    1s    Check Server Health
    ${headers}=         Create Dictionary    X-API-Key=default-api-key
    Create Session      octopus    ${BASE_URL}    headers=${headers}    disable_warnings=1
    Open Headless Browser

Teardown Visualizer E2E Suite
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
