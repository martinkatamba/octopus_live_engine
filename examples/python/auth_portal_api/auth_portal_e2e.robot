*** Settings ***
Documentation     OCTOPUS Python Customer Auth Portal API Live Engine Visualizer End-to-End Test Suite with JWT Auth, Tier Branching & Middlewares
Library           Process
Library           RequestsLibrary
Library           Collections
Library           OperatingSystem
Library           String
Library           SeleniumLibrary

Suite Setup       Setup Customer Auth Portal E2E Suite
Suite Teardown    Teardown Customer Auth Portal E2E Suite

*** Variables ***
${PORT}               8093
${HOST}               http://127.0.0.1:${PORT}
${BASE_URL}           ${HOST}/api/v1.0.0/customer_auth_portal
${DOMAIN}             customer_auth_portal
${VISUALIZER_URL}     ${HOST}/visualizer?domain=${DOMAIN}
${SCREENSHOTS_DIR}    ${CURDIR}/screenshots

*** Test Cases ***

Scenario 1: Visualizer Loads Static Blueprint for Customer Auth Portal
    [Documentation]    Verifies that navigating to /visualizer loads the Python Customer Auth Portal workflow blueprint.
    Go To    ${VISUALIZER_URL}
    Wait Until Page Contains Element    id:jobIdInput    15s
    Wait Until Page Contains Element    id:graph-container    15s
    
    Wait Until Keyword Succeeds    15s    1s    Wait For Graph Nodes Rendered

    ${node_count}=    Execute Javascript    return (window.graph && window.graph.getNodes) ? window.graph.getNodes().length : 0;
    ${edge_count}=    Execute Javascript    return (window.graph && window.graph.getEdges) ? window.graph.getEdges().length : 0;
    
    Log To Console    \n📊 Python Customer Auth Portal Blueprint: ${node_count} nodes, ${edge_count} edges
    Should Be True    ${node_count} >= 3    Expected at least 3 nodes
    Should Be True    ${edge_count} >= 2    Expected edges connecting start, activities, and end

    Execute Javascript    if (window.graph && window.graph.zoomToFit) { window.graph.zoomToFit({ padding: 30, maxScale: 1.2 }); }
    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/01_python_auth_portal_blueprint.png


Scenario 2: Route Selector Dropdown Displays All Declared API Routes
    [Documentation]    Verifies that the route selection dropdown displays all 3 declared API routes and dynamically switches the DAG.
    Go To    ${VISUALIZER_URL}
    Wait Until Page Contains Element    id:routeSelect    15s
    Wait Until Element Is Visible       id:routeSelect    15s
    
    ${options}=    Get List Items       id:routeSelect
    Log To Console    \n🔀 Python Visualizer Routes: ${options}
    Length Should Be    ${options}    3

    # Switch to Cart Checkout Route (Option 3)
    Select From List By Index    id:routeSelect    2
    Execute Javascript    onRouteSelectChanged();
    Sleep    1s
    Wait Until Keyword Succeeds    15s    1s    Wait For Graph Nodes Rendered

    ${node_count}=    Execute Javascript    return (window.graph && window.graph.getNodes) ? window.graph.getNodes().length : 0;
    Log To Console    \n🛒 Checkout Route DAG Node Count: ${node_count}
    Should Be True    ${node_count} >= 5    Expected checkout route to contain branching nodes


Scenario 3: Public Login Issues Valid Signed Bearer JWT Token
    [Documentation]    Calls POST /auth/login with Sarah VIP credentials and receives a valid Bearer JWT.
    ${job_id}=        Generate Random Job Id    py-auth-login
    ${payload}=       Create Dictionary    username=sarah_vip    password=password123
    ${headers}=       Create Dictionary    X-Job-ID=${job_id}    Content-Type=application/json

    ${resp}=          POST On Session    auth_portal_session    url=/auth/login    json=${payload}    headers=${headers}
    Status Should Be  200    ${resp}
    ${body}=          Set Variable    ${resp.json()}
    
    Dictionary Should Contain Key    ${body}    access_token
    ${token}=         Get From Dictionary    ${body}    access_token
    ${token_type}=    Get From Dictionary    ${body}    token_type
    ${user_id}=       Get From Dictionary    ${body}    user_id
    ${tier}=          Get From Dictionary    ${body}    customer_tier

    Should Be Equal As Strings    ${token_type}    Bearer
    Should Be Equal As Strings    ${user_id}       usr_9988_vip
    Should Be Equal As Strings    ${tier}          VIP
    Set Suite Variable    ${SARAH_TOKEN}    ${token}
    Log To Console    \n🔑 Python Login Succeeded for User: ${user_id} (Tier: ${tier})


Scenario 4: Public Login Rejects Invalid Password with Declarative Error
    [Documentation]    Calls POST /auth/login with invalid password and verifies 401 response.
    ${payload}=       Create Dictionary    username=sarah_vip    password=wrongPassword123
    ${headers}=       Create Dictionary    Content-Type=application/json

    ${resp}=          POST On Session    auth_portal_session    url=/auth/login    json=${payload}    headers=${headers}    expected_status=401
    Status Should Be  401    ${resp}
    ${body}=          Set Variable    ${resp.json()}
    Log To Console    \n🛑 Python Login Rejection Response: ${body}


Scenario 5: Protected Profile Endpoint Rejects Unauthenticated Request
    [Documentation]    Calls GET /profile without Authorization header and expects 401 Unauthorized.
    ${resp}=          GET On Session    auth_portal_session    url=/profile    expected_status=401
    Status Should Be  401    ${resp}
    Log To Console    \n🛡️ Python Unauthenticated Request Blocked Successfully


Scenario 6: Protected Profile Endpoint Succeeds with Valid JWT
    [Documentation]    Calls GET /profile with Sarah's Bearer JWT and retrieves customer profile.
    ${headers}=       Create Dictionary    Authorization=Bearer ${SARAH_TOKEN}
    ${resp}=          GET On Session    auth_portal_session    url=/profile    headers=${headers}
    Status Should Be  200    ${resp}
    ${body}=          Set Variable    ${resp.json()}
    
    ${user_id}=       Get From Dictionary    ${body}    user_id
    ${username}=      Get From Dictionary    ${body}    username
    ${email}=         Get From Dictionary    ${body}    email
    ${tier}=          Get From Dictionary    ${body}    customer_tier
    ${status}=        Get From Dictionary    ${body}    account_status

    Should Be Equal As Strings    ${user_id}     usr_9988_vip
    Should Be Equal As Strings    ${username}    sarah_vip
    Should Be Equal As Strings    ${email}       sarah@acme-corp.com
    Should Be Equal As Strings    ${tier}        VIP
    Should Be Equal As Strings    ${status}      ACTIVE_VIP
    Log To Console    \n👤 Python Profile Lookup Succeeded for VIP User: ${username} (${email})


Scenario 7: VIP Shopping Cart Checkout with Preferential 20% Discount
    [Documentation]    Calls POST /cart/checkout with VIP token and verifies 20% discount & Free Priority Air shipping.
    ${job_id}=        Generate Random Job Id    py-vip-cart
    Set Suite Variable    ${VIP_CHECKOUT_JOB_ID}    ${job_id}
    
    ${item1}=         Create Dictionary    sku=SONY-WH1000XM5    description=Wireless Headphones    quantity=${2}    unit_price=${400.00}
    ${items}=         Create List    ${item1}
    
    ${payload}=       Create Dictionary    cart_id=cart_vip_001    items=${items}
    ${headers}=       Create Dictionary    Authorization=Bearer ${SARAH_TOKEN}    X-Job-ID=${job_id}    Content-Type=application/json

    ${resp}=          POST On Session    auth_portal_session    url=/cart/checkout    json=${payload}    headers=${headers}
    Status Should Be  200    ${resp}
    ${body}=          Set Variable    ${resp.json()}
    
    ${tier}=          Get From Dictionary    ${body}    customer_tier
    ${subtotal}=      Get From Dictionary    ${body}    subtotal
    ${discount}=      Get From Dictionary    ${body}    discount_applied
    ${final_total}=   Get From Dictionary    ${body}    final_total
    ${pricing}=       Get From Dictionary    ${body}    pricing_tier
    ${shipping}=      Get From Dictionary    ${body}    shipping_method

    Should Be Equal As Strings    ${tier}           VIP
    Should Be Equal As Numbers    ${subtotal}       800.00
    Should Be Equal As Numbers    ${discount}       160.00
    Should Be Equal As Numbers    ${final_total}    640.00
    Should Be Equal As Strings    ${pricing}        VIP_PREFERENTIAL_20_PERCENT_DISCOUNT
    Should Be Equal As Strings    ${shipping}       FREE_PRIORITY_EXPRESS_AIR
    Log To Console    \n🎁 Python VIP Cart Checkout Succeeded: Subtotal=$${subtotal}, Discount=-$${discount}, Final=$${final_total}


Scenario 8: Standard User Cart Checkout with Standard Retail Pricing
    [Documentation]    Logs in as John Doe (Standard user) and verifies regular retail pricing on checkout.
    # 1. Login as standard user
    ${login_payload}=    Create Dictionary    username=john_doe    password=password123
    ${login_headers}=    Create Dictionary    Content-Type=application/json
    ${login_resp}=       POST On Session    auth_portal_session    url=/auth/login    json=${login_payload}    headers=${login_headers}
    Status Should Be     200    ${login_resp}
    ${std_token}=        Get From Dictionary    ${login_resp.json()}    access_token

    # 2. Checkout
    ${item1}=         Create Dictionary    sku=USB-C-CABLE    description=Braided Cable    quantity=${2}    unit_price=${20.00}
    ${items}=         Create List    ${item1}
    ${payload}=       Create Dictionary    cart_id=cart_std_002    items=${items}
    ${headers}=       Create Dictionary    Authorization=Bearer ${std_token}    Content-Type=application/json

    ${resp}=          POST On Session    auth_portal_session    url=/cart/checkout    json=${payload}    headers=${headers}
    Status Should Be  200    ${resp}
    ${body}=          Set Variable    ${resp.json()}
    
    ${tier}=          Get From Dictionary    ${body}    customer_tier
    ${subtotal}=      Get From Dictionary    ${body}    subtotal
    ${discount}=      Get From Dictionary    ${body}    discount_applied
    ${final_total}=   Get From Dictionary    ${body}    final_total
    ${pricing}=       Get From Dictionary    ${body}    pricing_tier
    ${shipping}=      Get From Dictionary    ${body}    shipping_method

    Should Be Equal As Strings    ${tier}           STANDARD
    Should Be Equal As Numbers    ${subtotal}       40.00
    Should Be Equal As Numbers    ${discount}       0.00
    Should Be Equal As Numbers    ${final_total}    40.00
    Should Be Equal As Strings    ${pricing}        STANDARD_RETAIL_PRICING
    Should Be Equal As Strings    ${shipping}       STANDARD_GROUND_SHIPPING
    Log To Console    \n🛒 Python Standard Cart Checkout Succeeded: Final Total=$${final_total}


Scenario 9: Visualizer Live Interactive Inspection for VIP Journey
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
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/02_python_auth_portal_vip_journey.png


Scenario 10: Visualizer Interactive Branch Condition & Evaluation Inspector
    [Documentation]    Clicks the branch step 'step_branch_by_tier' and verifies branch rules & live evaluation.
    # Click branch node via window.selectNode helper
    ${selected}=    Execute Javascript    return window.selectNode('step_branch_by_tier');
    Should Be True    ${selected}    Expected step_branch_by_tier node to be selected
    
    # Wait for inspector panel to open
    Wait Until Page Contains Element    id:insp-branch-field       15s
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
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/03_python_auth_portal_branch_inspector.png


*** Keywords ***

Setup Customer Auth Portal E2E Suite
    Create Directory    ${SCREENSHOTS_DIR}
    Create Directory    ${CURDIR}/results

    # Start Python Server process
    ${server_process}=    Start Process    /Volumes/Macintosh HD - Data/2026/AI_PROJECTS/project-octopus/octopus_live_engine/python/.venv/bin/python    server.py    --port    ${PORT}    cwd=${CURDIR}
    Set Suite Variable    ${SERVER_PROCESS}    ${server_process}
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
