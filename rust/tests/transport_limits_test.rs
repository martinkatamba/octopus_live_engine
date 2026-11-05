use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

use octopus_live_engine::http::{
    MAX_REQUEST_BODY_BYTES, MAX_REQUEST_HEADERS, MAX_REQUEST_HEAD_BYTES,
};
use octopus_live_engine::{HttpContext, Router};

fn start_server() -> SocketAddr {
    let mut router = Router::new();
    router.post("/echo", |ctx: &mut HttpContext| {
        let len = ctx.body.len();
        ctx.status_code = 200;
        ctx.resp_body = len.to_string().into_bytes();
    });
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || router.serve(listener));
    addr
}

fn send_raw(addr: SocketAddr, raw: &[u8]) -> String {
    let mut stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let _ = stream.write_all(raw);
    let mut out = String::new();
    let _ = stream.read_to_string(&mut out);
    out
}

fn status_of(resp: &str) -> &str {
    resp.split_whitespace().nth(1).unwrap_or("")
}

#[test]
fn normal_request_is_served() {
    let addr = start_server();
    let resp = send_raw(
        addr,
        b"POST /echo HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\n\r\nhello",
    );
    assert_eq!(status_of(&resp), "200", "{resp}");
    assert!(resp.ends_with('5'), "{resp}");
}

#[test]
fn oversized_content_length_is_rejected_without_crashing() {
    let addr = start_server();
    let resp = send_raw(
        addr,
        b"POST /echo HTTP/1.1\r\nHost: x\r\nContent-Length: 999999999999999\r\n\r\n",
    );
    assert_eq!(status_of(&resp), "413", "{resp}");

    let just_over = format!(
        "POST /echo HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n",
        MAX_REQUEST_BODY_BYTES + 1
    );
    assert_eq!(status_of(&send_raw(addr, just_over.as_bytes())), "413");

    // The server must still be alive afterwards.
    let resp = send_raw(
        addr,
        b"POST /echo HTTP/1.1\r\nHost: x\r\nContent-Length: 2\r\n\r\nok",
    );
    assert_eq!(status_of(&resp), "200", "{resp}");
}

#[test]
fn invalid_content_length_is_rejected() {
    let addr = start_server();
    let resp = send_raw(
        addr,
        b"POST /echo HTTP/1.1\r\nHost: x\r\nContent-Length: -1\r\n\r\n",
    );
    assert_eq!(status_of(&resp), "400", "{resp}");
}

#[test]
fn oversized_header_line_is_rejected() {
    let addr = start_server();
    let big = "a".repeat(MAX_REQUEST_HEAD_BYTES + 10);
    let raw = format!("POST /echo HTTP/1.1\r\nX-Big: {big}\r\n\r\n");
    assert_eq!(status_of(&send_raw(addr, raw.as_bytes())), "431");
}

#[test]
fn oversized_request_line_is_rejected() {
    let addr = start_server();
    let path = "a".repeat(MAX_REQUEST_HEAD_BYTES + 10);
    let raw = format!("GET /{path} HTTP/1.1\r\n\r\n");
    assert_eq!(status_of(&send_raw(addr, raw.as_bytes())), "414");
}

#[test]
fn too_many_headers_are_rejected() {
    let addr = start_server();
    let mut raw = String::from("POST /echo HTTP/1.1\r\n");
    for i in 0..=MAX_REQUEST_HEADERS {
        raw.push_str(&format!("X-H{i}: v\r\n"));
    }
    raw.push_str("\r\n");
    assert_eq!(status_of(&send_raw(addr, raw.as_bytes())), "431");
}
