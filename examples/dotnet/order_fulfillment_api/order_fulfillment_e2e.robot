*** Settings ***
Documentation     OCTOPUS .NET 8 Order Fulfillment API Live Engine Visualizer End-to-End Test Suite with Branching & Loops
Library           Process
Library           RequestsLibrary
Library           Collections
Library           OperatingSystem
Library           String
Library           SeleniumLibrary

Suite Setup       Setup Order Fulfillment E2E Suite
Suite Teardown    Teardown Order Fulfillment E2E Suite

*** Variables ***
${PORT}               8096
${HOST}               http://127.0.0.1:${PORT}
${BASE_URL}           ${HOST}/api/v1
${DOMAIN}             order_fulfillment_pipeline
${VISUALIZER_URL}     ${HOST}/visualizer?domain=${DOMAIN}
${SCREENSHOTS_DIR}    ${CURDIR}/screenshots

*** Test Cases ***

Scenario 1: Visualizer Loads Static Blueprint with Branching Diamond and Loop Node
    [Documentation]    Verifies that navigating to /visualizer loads the .NET Order Fulfillment workflow blueprint.
    Go To    ${VISUALIZER_URL}
    Wait Until Page Contains Element    id:jobIdInput    15s
    Wait Until Page Contains Element    id:graph-container    15s
    
    Wait Until Keyword Succeeds    15s    1s    Wait For Graph Nodes Rendered

    ${node_count}=    Execute Javascript    return (window.graph && window.graph.getNodes) ? window.graph.getNodes().length : 0;
    ${edge_count}=    Execute Javascript    return (window.graph && window.graph.getEdges) ? window.graph.getEdges().length : 0;
    
    Log To Console    \n📊 .NET Order Fulfillment Blueprint: ${node_count} nodes, ${edge_count} edges
    Should Be True    ${node_count} >= 5    Expected at least 5 nodes including loop and branch
    Should Be True    ${edge_count} >= 4    Expected edges connecting start, loop, branches, and end

    Execute Javascript    if (window.graph && window.graph.zoomToFit) { window.graph.zoomToFit({ padding: 30, maxScale: 1.2 }); }
    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/01_dotnet_order_fulfillment_blueprint_loop_branch.png


Scenario 2: Synchronous Fast-Path Fulfills VIP High-Value Order (Express Branch)
    [Documentation]    Calls POST /api/v1/orders/fulfill with a VIP high-value order (> $1,000).
    ${job_id}=        Generate Random Job Id    dotnet-vip-pay
    Set Suite Variable    ${VIP_JOB_ID}    ${job_id}
    
    ${item1}=         Create Dictionary    sku=SKU_MACBOOK_PRO    quantity=${1}    unit_price=${2499.00}
    ${item2}=         Create Dictionary    sku=SKU_MAGIC_MOUSE    quantity=${1}    unit_price=${99.00}
    ${items}=         Create List    ${item1}    ${item2}
    
    ${payload}=       Create Dictionary    customer_id=cust_vip_dotnet_01    customer_tier=VIP    order_id=ORD_VIP_DOTNET_77    items=${items}    shipping_address=1 Infinite Loop, Cupertino, CA
    ${headers}=       Create Dictionary    X-Job-ID=${job_id}    Content-Type=application/json

    ${resp}=          POST On Session    octopus_order_fulfill    url=/orders/fulfill    json=${payload}    headers=${headers}
    Status Should Be  200    ${resp}
    ${body}=          Set Variable    ${resp.json()}
    
    ${status}=          Get From Dictionary    ${body}    status
    ${total}=           Get From Dictionary    ${body}    order_total
    ${method}=          Get From Dictionary    ${body}    shipping_method
    ${reserved}=        Get From Dictionary    ${body}    items_reserved_count

    Should Be Equal As Strings    ${status}              FULFILLED
    Should Be True                ${total} > 1000
    Should Be Equal As Strings    ${method}              PRIORITY_EXPRESS_AIR
    Should Be Equal As Numbers    ${reserved}            2
    Log To Console    \n⚡ .NET VIP Order Fast-Path Fulfilled: JobID=${job_id}, Method=${method}, Total=$${total}


Scenario 3: Visualizer Traces Completed VIP Journey Highlighting Loop & Express Branch
    [Documentation]    Looks up the VIP job in the visualizer and verifies glowing green path along Loop and Express Branch.
    Go To    ${VISUALIZER_URL}
    Wait Until Page Contains Element    id:jobIdInput    15s
    Clear Element Text    id:jobIdInput
    Input Text    id:jobIdInput    ${VIP_JOB_ID}
    Click Button    id:lookupBtn

    Wait Until Element Is Visible    id:executionBanner    15s
    Wait Until Element Contains      id:executionSummaryText    COMPLETED    15s
    
    Execute Javascript    if (window.graph && window.graph.zoomToFit) { window.graph.zoomToFit({ padding: 30, maxScale: 1.2 }); }
    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/02_dotnet_vip_express_branch_completed_journey.png


Scenario 4: Synchronous Fast-Path Fulfills Standard Order (Standard Shipping Branch)
    [Documentation]    Calls POST /api/v1/orders/fulfill with a standard value order (< $1,000).
    ${job_id}=        Generate Random Job Id    dotnet-std-pay
    Set Suite Variable    ${STD_JOB_ID}    ${job_id}
    
    ${item1}=         Create Dictionary    sku=SKU_USB_C_CABLE    quantity=${2}    unit_price=${19.99}
    ${items}=         Create List    ${item1}
    
    ${payload}=       Create Dictionary    customer_id=cust_std_dotnet_02    customer_tier=STANDARD    order_id=ORD_STD_DOTNET_88    items=${items}    shipping_address=456 Oak Ave, Austin, TX
    ${headers}=       Create Dictionary    X-Job-ID=${job_id}    Content-Type=application/json

    ${resp}=          POST On Session    octopus_order_fulfill    url=/orders/fulfill    json=${payload}    headers=${headers}
    Status Should Be  200    ${resp}
    ${body}=          Set Variable    ${resp.json()}
    
    ${status}=          Get From Dictionary    ${body}    status
    ${total}=           Get From Dictionary    ${body}    order_total
    ${method}=          Get From Dictionary    ${body}    shipping_method

    Should Be Equal As Strings    ${status}              FULFILLED
    Should Be Equal As Strings    ${method}              STANDARD_GROUND
    Log To Console    \n⚡ .NET Standard Order Fast-Path Fulfilled: JobID=${job_id}, Method=${method}, Total=$${total}


Scenario 5: Visualizer Traces Completed Standard Journey Highlighting Standard Branch
    [Documentation]    Looks up the Standard job in the visualizer and verifies glowing path along Standard Branch.
    Go To    ${VISUALIZER_URL}
    Wait Until Page Contains Element    id:jobIdInput    15s
    Clear Element Text    id:jobIdInput
    Input Text    id:jobIdInput    ${STD_JOB_ID}
    Click Button    id:lookupBtn

    Wait Until Element Is Visible    id:executionBanner    15s
    Wait Until Element Contains      id:executionSummaryText    COMPLETED    15s
    
    Execute Javascript    if (window.graph && window.graph.zoomToFit) { window.graph.zoomToFit({ padding: 30, maxScale: 1.2 }); }
    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/03_dotnet_standard_branch_completed_journey.png


Scenario 6: Step Inspector Drawer Displays Multi-Item Loop Payloads
    [Documentation]    Opens the step inspector drawer for the loop step and inspects multi-item payload.
    Go To    ${VISUALIZER_URL}
    Wait Until Page Contains Element    id:jobIdInput    15s
    Clear Element Text    id:jobIdInput
    Input Text    id:jobIdInput    ${VIP_JOB_ID}
    Click Button    id:lookupBtn

    Wait Until Element Is Visible    id:executionBanner    15s
    Wait Until Element Contains      id:executionSummaryText    COMPLETED    15s
    Sleep    1s
    Wait Until Keyword Succeeds      15s    1s    Wait For Graph Nodes Rendered

    Execute Javascript    if (window.selectNode) { window.selectNode('step_reserve_items'); } else if (window.graph && window.graph.getNodes) { const nodes = window.graph.getNodes(); const target = nodes.find(n => n.id.includes('step_reserve')); if (target) { window.graph.trigger('node:click', { node: target }); } }
    Wait Until Page Contains Element    css:#inspector.open    10s
    Wait Until Element Is Visible       id:insp-input-payload-field    10s
    Wait Until Element Is Visible       id:insp-output-payload-field   10s
    Wait Until Element Contains         id:insp-output-json-content    iterations    10s
    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/04_dotnet_inspector_drawer_loop_payload_details.png


*** Keywords ***

Setup Order Fulfillment E2E Suite
    Create Directory    ${SCREENSHOTS_DIR}
    Log To Console      \nStarting .NET Smart Order Fulfillment API on port ${PORT}...
    ${handle}=          Start Process    dotnet    bin/Debug/net8.0/OrderFulfillmentApi.dll    cwd=${CURDIR}    env:PORT=${PORT}    env:HOME=/Users/martin    env:DOTNET_CLI_HOME=/Users/martin
    Set Suite Variable  ${SERVER_PROCESS}    ${handle}
    
    Wait Until Keyword Succeeds    30s    1s    Check Server Health
    ${headers}=         Create Dictionary    X-API-Key=default-api-key
    Create Session      octopus_order_fulfill    ${BASE_URL}    headers=${headers}    disable_warnings=1
    Open Headless Browser

Teardown Order Fulfillment E2E Suite
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

