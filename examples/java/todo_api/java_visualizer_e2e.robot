*** Settings ***
Documentation     OCTOPUS Java Todo REST API Live Engine Visualizer End-to-End Test Suite
Library           Process
Library           RequestsLibrary
Library           Collections
Library           OperatingSystem
Library           String
Library           SeleniumLibrary

Suite Setup       Setup Java Visualizer E2E Suite
Suite Teardown    Teardown Java Visualizer E2E Suite

*** Variables ***
${PORT}               8097
${HOST}               http://127.0.0.1:${PORT}
${BASE_URL}           ${HOST}/api/v1
${DOMAIN}             todo_api_service
${VISUALIZER_URL}     ${HOST}/visualizer?domain=${DOMAIN}
${SCREENSHOTS_DIR}    ${CURDIR}/screenshots
${JAVA_BIN}           /Users/martin/Library/Java/JavaVirtualMachines/openjdk-23.0.1/Contents/Home/bin/java
${CP}                 /Users/martin/.m2/repository/com/fasterxml/jackson/core/jackson-databind/2.17.0/jackson-databind-2.17.0.jar:/Users/martin/.m2/repository/com/fasterxml/jackson/core/jackson-core/2.17.0/jackson-core-2.17.0.jar:/Users/martin/.m2/repository/com/fasterxml/jackson/core/jackson-annotations/2.17.0/jackson-annotations-2.17.0.jar:/Users/martin/.m2/repository/com/fasterxml/jackson/dataformat/jackson-dataformat-yaml/2.17.0/jackson-dataformat-yaml-2.17.0.jar:/Users/martin/.m2/repository/org/yaml/snakeyaml/2.2/snakeyaml-2.2.jar:${CURDIR}/../../../java/target/classes

*** Test Cases ***

Scenario 1: Java Visualizer Loads Static Blueprint Graph :: Verifies X6 Canvas
    [Documentation]    Verifies that navigating to /visualizer loads the Java Todo Service workflow blueprint.
    Go To    ${VISUALIZER_URL}
    Wait Until Page Contains Element    id:jobIdInput    15s
    Wait Until Page Contains Element    id:graph-container    15s
    
    Wait Until Keyword Succeeds    15s    1s    Wait For Graph Nodes Rendered

    ${node_count}=    Execute Javascript    return (window.graph && window.graph.getNodes) ? window.graph.getNodes().length : 0;
    ${edge_count}=    Execute Javascript    return (window.graph && window.graph.getEdges) ? window.graph.getEdges().length : 0;
    
    Log To Console    \n📊 Java Todo Blueprint: ${node_count} nodes, ${edge_count} edges
    Should Be True    ${node_count} > 0    Expected nodes to be rendered on canvas
    Should Be True    ${edge_count} > 0    Expected edges to be rendered on canvas

    Execute Javascript    if (window.graph && window.graph.zoomToFit) { window.graph.zoomToFit({ padding: 30, maxScale: 1.2 }); }
    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/01_java_todo_blueprint.png


Scenario 2: Java Synchronous POST Creates a New Todo Item :: Calls POST /api/v1/todos
    [Documentation]    Creates a new Todo item via the Java Live Engine.
    ${job_id}=        Generate Random Job Id    todo-create-java
    Set Suite Variable    ${CREATED_JOB_ID}    ${job_id}
    ${payload}=       Create Dictionary    title=Java E2E Todo Item    description=Automated test item created by Robot Framework
    ${headers}=       Create Dictionary    X-Job-ID=${job_id}    Content-Type=application/json

    ${resp}=          POST On Session    octopus_java_todo    url=/todos    json=${payload}    headers=${headers}
    Status Should Be  201    ${resp}
    ${body}=          Set Variable    ${resp.json()}
    
    ${todo_id}=       Get From Dictionary    ${body}    id
    ${title}=         Get From Dictionary    ${body}    title
    Set Suite Variable    ${TODO_ID}    ${todo_id}

    Should Be Equal As Strings    ${title}    Java E2E Todo Item
    Log To Console    \n⚡ Java Todo Created: ID=${todo_id}, Title="${title}"


Scenario 3: Java Visualizer Traces Completed Todo Creation Workflow
    [Documentation]    Looks up the executed creation job in the visualizer.
    Go To    ${VISUALIZER_URL}
    Wait Until Page Contains Element    id:jobIdInput    15s
    Clear Element Text    id:jobIdInput
    Input Text    id:jobIdInput    ${CREATED_JOB_ID}
    Click Button    id:lookupBtn

    Wait Until Element Is Visible    id:executionBanner    15s
    Wait Until Element Contains      id:executionSummaryText    COMPLETED    15s

    Execute Javascript    if (window.graph && window.graph.zoomToFit) { window.graph.zoomToFit({ padding: 30, maxScale: 1.2 }); }
    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/02_java_todo_creation_journey.png


Scenario 4: Java Synchronous GET Retrieves Active Todo List
    [Documentation]    Calls GET /api/v1/todos on Java Live Engine.
    ${resp}=          GET On Session    octopus_java_todo    url=/todos
    Status Should Be  200    ${resp}
    ${body}=          Set Variable    ${resp.json()}
    ${todos}=         Get From Dictionary    ${body}    todos
    ${count}=         Get Length    ${todos}
    Should Be True    ${count} >= 1
    Log To Console    \n📋 Java Todo List Fetched: ${count} item(s) found


Scenario 5: Java Visualizer Step Details Inspection
    [Documentation]    Opens step inspector drawer and verifies payload rendering.
    Go To    ${VISUALIZER_URL}
    Wait Until Page Contains Element    id:jobIdInput    15s
    Clear Element Text    id:jobIdInput
    Input Text    id:jobIdInput    ${CREATED_JOB_ID}
    Click Button    id:lookupBtn

    Wait Until Element Is Visible    id:executionBanner    15s
    Wait Until Element Contains      id:executionSummaryText    COMPLETED    15s

    Execute Javascript    if (window.selectNode) { window.selectNode('step_package_todo_response'); } else if (window.graph && window.graph.getNodes) { const nodes = window.graph.getNodes(); const target = nodes.find(n => n.id.includes('step_package') || n.id.includes('step_persist')); if (target) { window.graph.trigger('node:click', { node: target }); } }
    Wait Until Page Contains Element    css:#inspector.open    10s
    Wait Until Element Is Visible       id:insp-input-payload-field    10s
    Wait Until Element Is Visible       id:insp-output-payload-field   10s
    Wait Until Element Contains         id:insp-output-json-content    Java E2E Todo Item    10s
    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/03_java_todo_inspector_details.png


*** Keywords ***

Setup Java Visualizer E2E Suite
    Create Directory    ${SCREENSHOTS_DIR}
    Log To Console      \nStarting Java Todo REST API on port ${PORT}...
    ${handle}=          Start Process    ${JAVA_BIN}    -cp    ${CP}    io.octopus.examples.todo.TodoApplication    cwd=${CURDIR}    env:PORT=${PORT}
    Set Suite Variable  ${SERVER_PROCESS}    ${handle}
    
    Wait Until Keyword Succeeds    30s    1s    Check Server Health
    ${headers}=         Create Dictionary    X-API-Key=default-api-key
    Create Session      octopus_java_todo    ${BASE_URL}    headers=${headers}    disable_warnings=1
    Open Headless Browser

Teardown Java Visualizer E2E Suite
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
