*** Settings ***
Documentation     OCTOPUS Customer Auth Portal (Java 21+) Visualizer End-to-End Test Suite
Library           RequestsLibrary
Library           Collections
Library           OperatingSystem
Library           String
Library           SeleniumLibrary

Suite Setup       Setup Visualizer E2E Suite
Suite Teardown    Teardown Visualizer E2E Suite

*** Variables ***
${PORT}               8084
${HOST}               http://localhost:${PORT}
${BASE_URL}           ${HOST}/api/v1.0.0/customer_auth_portal
${VISUALIZER_URL}     ${HOST}/visualizer?domain=customer_auth_portal
${SCREENSHOTS_DIR}    ${CURDIR}/screenshots

*** Test Cases ***

Scenario 1: Visualizer Loads Public Login Route with Excluded Bearer Auth
    [Documentation]    Verifies that the login route displays 3 active ingress middlewares and BearerAuth is excluded.
    Go To    ${VISUALIZER_URL}&route=/api/v1.0.0/customer_auth_portal/auth/login&method=POST
    Wait Until Page Contains Element    id:graph-container    15s
    Wait Until Keyword Succeeds    15s    1s    Wait For Graph Nodes Rendered

    ${node_count}=    Execute Javascript    return (window.graph && window.graph.getNodes) ? window.graph.getNodes().length : 0;
    ${edge_count}=    Execute Javascript    return (window.graph && window.graph.getEdges) ? window.graph.getEdges().length : 0;
    Log To Console    \n📊 [Login Route] Blueprint Loaded: ${node_count} nodes, ${edge_count} edges
    Should Be True    ${node_count} > 0
    Should Be True    ${edge_count} > 0

    # Verify edge to node__end exists
    ${has_end_edge}=    Execute Javascript    return window.graph.getEdges().some(e => (e.getTargetCellId && e.getTargetCellId() === 'node__end') || (e.target && e.target.cell === 'node__end'));
    Should Be True    ${has_end_edge}    Expected edge connecting to node__end (✅ 200 OK)

    # Click Ingress Guards and verify Step Inspector
    Click Ingress Middleware Guards Node
    Wait Until Element Is Visible    id:insp-middlewares-field    10s
    ${mw_count_text}=    Get Text    id:insp-middlewares-count-badge
    Should Contain    ${mw_count_text}    3 Active Guards

    Execute Javascript    if (window.graph && window.graph.zoomToFit) { window.graph.zoomToFit({ padding: 30, maxScale: 1.1 }); }
    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/01_login_route_blueprint.png


Scenario 2: Visualizer Loads Checkout Route with Branching and Connected 200 OK Terminal
    [Documentation]    Verifies the checkout route graph layout, VIP conditional diamond, and the line connecting step_finalize_checkout to ✅ 200 OK.
    Go To    ${VISUALIZER_URL}&route=/api/v1.0.0/customer_auth_portal/cart/checkout&method=POST
    Wait Until Page Contains Element    id:graph-container    15s
    Wait Until Keyword Succeeds    15s    1s    Wait For Graph Nodes Rendered

    ${node_count}=    Execute Javascript    return (window.graph && window.graph.getNodes) ? window.graph.getNodes().length : 0;
    ${edge_count}=    Execute Javascript    return (window.graph && window.graph.getEdges) ? window.graph.getEdges().length : 0;
    Log To Console    \n📊 [Checkout Route] Blueprint Loaded: ${node_count} nodes, ${edge_count} edges

    # Verify specific nodes exist
    ${has_branch_node}=    Execute Javascript    return window.graph.getNodes().some(n => n.id === 'node__step_branch_by_tier');
    ${has_finalize_node}=  Execute Javascript    return window.graph.getNodes().some(n => n.id === 'node__step_finalize_checkout');
    ${has_end_node}=       Execute Javascript    return window.graph.getNodes().some(n => n.id === 'node__end');
    Should Be True    ${has_branch_node}    Expected branch diamond node__step_branch_by_tier
    Should Be True    ${has_finalize_node}  Expected step_finalize_checkout node
    Should Be True    ${has_end_node}       Expected terminal node__end

    # Verify edge from step_finalize_checkout to node__end exists and is connected
    ${end_edge_src}=    Execute Javascript    return (function() { var e = window.graph.getEdges().find(x => (x.getTargetCellId && x.getTargetCellId() === 'node__end') || (x.target && x.target.cell === 'node__end')); return e ? (e.getSourceCellId ? e.getSourceCellId() : e.source.cell) : null; })();
    Log To Console    \n🔗 Edge to node__end source: ${end_edge_src}
    Should Be Equal As Strings    ${end_edge_src}    node__step_finalize_checkout

    # Click Ingress Guards and verify 4 Active Guards on checkout route
    Click Ingress Middleware Guards Node
    Wait Until Element Is Visible    id:insp-middlewares-field    10s
    ${mw_count_text}=    Get Text    id:insp-middlewares-count-badge
    Should Contain    ${mw_count_text}    4 Active Guards

    Execute Javascript    if (window.graph && window.graph.zoomToFit) { window.graph.zoomToFit({ padding: 30, maxScale: 1.1 }); }
    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/02_checkout_route_branching_and_200_ok.png


Scenario 3: Live Trace - Sarah VIP Cart Checkout Executes VIP Branch & Connects 200 OK
    [Documentation]    Executes VIP checkout request and visualizes the completed VIP discount path leading into ✅ 200 OK.
    ${job_id}=        Generate Random Job Id    job-e2e-vip-checkout
    
    # 1. Login Sarah VIP to get token
    ${login_payload}=    Create Dictionary    username=sarah_vip    password=password123
    ${headers}=          Create Dictionary    Content-Type=application/json
    ${login_resp}=       POST On Session    octopus    url=/auth/login    json=${login_payload}    headers=${headers}
    Status Should Be     200    ${login_resp}
    ${token}=            Get From Dictionary    ${login_resp.json()}    access_token

    # 2. Execute Checkout
    ${item1}=            Create Dictionary    sku=SKU-PRO-LATTE    name=Organic Espresso    quantity=${2}    price=${300.0}
    ${item2}=            Create Dictionary    sku=SKU-MUG-CERAMIC  name=Octopus Travel Mug  quantity=${1}    price=${200.0}
    ${items}=            Create List    ${item1}    ${item2}
    ${checkout_body}=    Create Dictionary    items=${items}    shipping_address=456 Market St, San Francisco, CA
    ${auth_headers}=     Create Dictionary    Content-Type=application/json    Authorization=Bearer ${token}    X-Job-ID=${job_id}
    
    ${checkout_resp}=    POST On Session    octopus    url=/cart/checkout    json=${checkout_body}    headers=${auth_headers}
    Status Should Be     200    ${checkout_resp}
    ${discount}=         Get From Dictionary    ${checkout_resp.json()}    discount_applied
    Log To Console       \n💳 VIP Checkout Response Discount: $${discount}

    # 3. View Execution in Visualizer
    Go To    ${VISUALIZER_URL}&job_id=${job_id}
    Wait Until Page Contains Element    id:graph-container    15s
    Wait Until Keyword Succeeds    15s    1s    Wait For Graph Nodes Rendered

    # Verify VIP Node and End Node executed status
    ${vip_executed}=    Execute Javascript    return (function() { var n = window.graph.getNodes().find(x => x.id.includes('step_apply_vip_discount')); return n ? (n.getData ? n.getData().executed : false) : false; })();
    ${end_executed}=    Execute Javascript    return (function() { var n = window.graph.getNodes().find(x => x.id === 'node__end'); return n ? (n.getData ? n.getData().executed : false) : false; })();
    Log To Console      \n✅ VIP Branch Executed: ${vip_executed}, Terminal 200 OK Executed: ${end_executed}
    Should Be True      ${vip_executed}
    Should Be True      ${end_executed}

    # Click Terminal Node and verify Step Inspector
    Execute Javascript    var n = window.graph.getNodes().find(x => x.id === 'node__end'); if (n) window.graph.trigger('node:click', { node: n });
    Wait Until Element Is Visible    id:inspector    10s
    ${insp_status}=     Get Text    id:insp-status
    Should Contain      ${insp_status}    COMPLETED

    Execute Javascript    if (window.graph && window.graph.zoomToFit) { window.graph.zoomToFit({ padding: 30, maxScale: 1.1 }); }
    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/03_vip_checkout_live_trace.png


Scenario 4: Live Trace - Standard Cart Checkout Executes Retail Pricing Branch
    [Documentation]    Executes Standard retail checkout and visualizes the retail branch path leading into ✅ 200 OK.
    ${job_id}=        Generate Random Job Id    job-e2e-std-checkout
    
    # 1. Login John Doe Standard to get token
    ${login_payload}=    Create Dictionary    username=john_doe    password=password123
    ${headers}=          Create Dictionary    Content-Type=application/json
    ${login_resp}=       POST On Session    octopus    url=/auth/login    json=${login_payload}    headers=${headers}
    Status Should Be     200    ${login_resp}
    ${token}=            Get From Dictionary    ${login_resp.json()}    access_token

    # 2. Execute Checkout
    ${item1}=            Create Dictionary    sku=SKU-NOTEBOOK    name=Developer Notebook    quantity=${2}    price=${20.0}
    ${items}=            Create List    ${item1}
    ${checkout_body}=    Create Dictionary    items=${items}    shipping_address=123 Main St, Springfield, OR
    ${auth_headers}=     Create Dictionary    Content-Type=application/json    Authorization=Bearer ${token}    X-Job-ID=${job_id}
    
    ${checkout_resp}=    POST On Session    octopus    url=/cart/checkout    json=${checkout_body}    headers=${auth_headers}
    Status Should Be     200    ${checkout_resp}

    # 3. View Execution in Visualizer
    Go To    ${VISUALIZER_URL}&job_id=${job_id}
    Wait Until Page Contains Element    id:graph-container    15s
    Wait Until Keyword Succeeds    15s    1s    Wait For Graph Nodes Rendered

    ${std_executed}=    Execute Javascript    return (function() { var n = window.graph.getNodes().find(x => x.id.includes('step_apply_standard_pricing')); return n ? (n.getData ? n.getData().executed : false) : false; })();
    ${end_executed}=    Execute Javascript    return (function() { var n = window.graph.getNodes().find(x => x.id === 'node__end'); return n ? (n.getData ? n.getData().executed : false) : false; })();
    Log To Console      \n✅ Standard Branch Executed: ${std_executed}, Terminal 200 OK Executed: ${end_executed}
    Should Be True      ${std_executed}
    Should Be True      ${end_executed}

    Execute Javascript    if (window.graph && window.graph.zoomToFit) { window.graph.zoomToFit({ padding: 30, maxScale: 1.1 }); }
    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/04_standard_checkout_live_trace.png


Scenario 5: Live Trace - Blocked IP Address Triggers 403 Forbidden Terminal Node
    [Documentation]    Sends request from blocked IP and visualizes the exception branch terminating at 🛑 403 FORBIDDEN.
    ${job_id}=        Generate Random Job Id    job-e2e-ip-blocked
    ${payload}=       Create Dictionary    username=sarah_vip    password=password123
    ${headers}=       Create Dictionary    Content-Type=application/json    X-Forwarded-For=10.0.0.99    X-Job-ID=${job_id}
    
    ${resp}=          POST On Session    octopus    url=/auth/login    json=${payload}    headers=${headers}    expected_status=403
    Status Should Be  403    ${resp}

    Go To    ${VISUALIZER_URL}&job_id=${job_id}
    Wait Until Page Contains Element    id:graph-container    15s
    Wait Until Keyword Succeeds    15s    1s    Wait For Graph Nodes Rendered

    ${term_403_executed}=    Execute Javascript    return (function() { var n = window.graph.getNodes().find(x => x.id === 'node__failed_term_403'); return n ? (n.getData ? n.getData().executed : false) : false; })();
    Should Be True           ${term_403_executed}    Expected 403 terminal node to be executed and glowing

    # Verify exception node label is clean (no bracketed words)
    ${exc_label}=     Execute Javascript    return (function() { var n = window.graph.getNodes().find(x => x.id.includes('exc_')); return n ? (n.getLabel ? n.getLabel() : (n.label || '')) : ''; })();
    Log To Console    \n🚨 Exception Node Label: ${exc_label}
    Should Not Contain    ${exc_label}    (Async

    # Click Exception Activity Handler and verify Step Inspector provenance
    Execute Javascript    var n = window.graph.getNodes().find(x => x.id === 'node__exc_ipaccessdenied'); if (n) window.graph.trigger('node:click', { node: n });
    Wait Until Element Is Visible    id:insp-exception-field    10s
    ${exc_origin}=    Get Text    id:insp-exc-origin
    Log To Console    \n🔍 Exception Inspector Origin: ${exc_origin}
    Should Contain    ${exc_origin}    IpWhitelistMiddleware

    Execute Javascript    if (window.graph && window.graph.zoomToFit) { window.graph.zoomToFit({ padding: 30, maxScale: 1.1 }); }
    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/05_ip_blocked_403_trace.png


Scenario 6: Minimizable Top-Left Workflow Description Overlay Tile
    [Documentation]    Verifies the glassmorphic workflow overview overlay tile in expanded and minimized states.
    Go To    ${VISUALIZER_URL}&route=/api/v1.0.0/customer_auth_portal/cart/checkout&method=POST
    Wait Until Page Contains Element    id:graph-container    15s
    Wait Until Keyword Succeeds    15s    1s    Wait For Graph Nodes Rendered
    Wait Until Element Is Visible    id:workflowTile    10s

    # Verify structured tile expanded content
    ${wf_header}=      Get Text    id:wf-tile-name
    ${wf_name_val}=    Get Text    id:wf-tile-name-val
    ${wf_ver_val}=     Get Text    id:wf-tile-version-val
    ${wf_desc_val}=    Get Text    id:wf-tile-desc-val
    ${wf_route_val}=   Get Text    id:wf-tile-route-val
    Log To Console    \n📜 Workflow Tile Header: ${wf_header} | name=${wf_name_val}, version=${wf_ver_val}, route=${wf_route_val}
    Should Be Equal As Strings    ${wf_header}      Workflow Overview
    Should Be Equal As Strings    ${wf_name_val}    customer_auth_portal
    Should Be Equal As Strings    ${wf_ver_val}     v1.0.0
    Should Not Be Empty           ${wf_desc_val}
    Should Contain                ${wf_route_val}   POST /api/v1.0.0/customer_auth_portal/cart/checkout

    # Capture expanded state
    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/08_workflow_tile_expanded.png

    # Click to Minimize
    Click Element    css:.wf-tile-header
    Sleep    500ms
    ${has_minimized_class}=    Execute Javascript    return document.getElementById('workflowTile').classList.contains('minimized');
    ${toggle_icon}=            Get Text    id:wfTileToggleIcon
    Should Be True             ${has_minimized_class}    Workflow tile should have minimized class
    Should Be Equal As Strings    ${toggle_icon}    ▼

    # Capture minimized state
    Sleep    1s
    Capture Page Screenshot    ${SCREENSHOTS_DIR}/09_workflow_tile_minimized.png

    # Click to Expand again
    Click Element    css:.wf-tile-header
    Sleep    500ms
    ${is_minimized_now}=       Execute Javascript    return document.getElementById('workflowTile').classList.contains('minimized');
    ${toggle_icon_now}=        Get Text    id:wfTileToggleIcon
    Should Not Be True         ${is_minimized_now}    Workflow tile should be expanded
    Should Be Equal As Strings    ${toggle_icon_now}    ▲


*** Keywords ***

Setup Visualizer E2E Suite
    Create Directory    ${SCREENSHOTS_DIR}
    Create Session      octopus    ${BASE_URL}    disable_warnings=1
    Open Headless Browser

Teardown Visualizer E2E Suite
    Close All Browsers

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

Wait For Graph Nodes Rendered
    ${count}=    Execute Javascript    return (window.graph && window.graph.getNodes) ? window.graph.getNodes().length : 0;
    Should Be True    ${count} > 0    Graph nodes not loaded yet

Click Ingress Middleware Guards Node
    Execute Javascript    var n = window.graph.getNodes().find(x => x.id === 'node__ingress_guards'); if (n) window.graph.trigger('node:click', { node: n });
    Sleep    500ms

Generate Random Job Id
    [Arguments]       ${prefix}
    ${epoch}=         Evaluate    int(time.time() * 1000)    modules=time
    RETURN            ${prefix}-${epoch}
