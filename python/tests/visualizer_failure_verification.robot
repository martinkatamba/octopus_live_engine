*** Settings ***
Documentation     Visualizer Failure Path Verification Suite with Robot Framework
Library           OperatingSystem
Library           Process

*** Variables ***
${BASE_URL}          http://127.0.0.1:8000
${TRIGGER_SCRIPT}    octopus_live_engine/python/tests/trigger_test_failure.py

*** Test Cases ***
Verify Failed Login Route 401 Overlay
    [Documentation]    Trigger 401 InvalidCredentials login failure and verify visualizer diagram execution path
    ${res}=    Run Process    python3    ${TRIGGER_SCRIPT}    trigger    login_401    8000
    ${job_id}=    Set Variable    ${res.stdout}
    Should Not Be Empty    ${job_id}
    Log    Captured Job ID for 401: ${job_id}

    # Verify resolve-job returns exists=true and status=FAILED
    ${resolve_res}=    Run Process    python3    ${TRIGGER_SCRIPT}    resolve    ${job_id}    8000
    Should Contain    ${resolve_res.stdout}    "exists":true
    Should Contain    ${resolve_res.stdout}    "status":"FAILED"

Verify Unhandled Exception 500 Route Overlay
    [Documentation]    Trigger 500 Unhandled Exception and verify visualizer diagram execution path
    ${res}=    Run Process    python3    ${TRIGGER_SCRIPT}    trigger    crash_500    8000
    ${job_id}=    Set Variable    ${res.stdout}
    Should Not Be Empty    ${job_id}
    Log    Captured Job ID for 500: ${job_id}

    # Verify resolve-job returns exists=true and status=FAILED
    ${resolve_res}=    Run Process    python3    ${TRIGGER_SCRIPT}    resolve    ${job_id}    8000
    Should Contain    ${resolve_res.stdout}    "exists":true
    Should Contain    ${resolve_res.stdout}    "status":"FAILED"

Verify Ingress Security Guard 401 Overlay
    [Documentation]    Trigger 401 Ingress Middleware Guard failure and verify visualizer diagram execution path
    ${res}=    Run Process    python3    ${TRIGGER_SCRIPT}    trigger    guard_401    8000
    ${job_id}=    Set Variable    ${res.stdout}
    Should Not Be Empty    ${job_id}
    Log    Captured Job ID for Guard 401: ${job_id}

    # Verify resolve-job returns exists=true and status=FAILED
    ${resolve_res}=    Run Process    python3    ${TRIGGER_SCRIPT}    resolve    ${job_id}    8000
    Should Contain    ${resolve_res.stdout}    "exists":true
    Should Contain    ${resolve_res.stdout}    "status":"FAILED"
