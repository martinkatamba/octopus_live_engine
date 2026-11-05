use crate::json::{Value, ValueMap};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, TcpListener, TcpStream};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

pub type HandlerFunc = Arc<dyn Fn(&mut HttpContext) + Send + Sync>;

pub use octopus_core::net::IpNet;

#[derive(Clone, Debug)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub raw_query: String,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
    pub remote_addr: String,
}

impl HttpRequest {
    pub fn new(method: &str, url: &str) -> Self {
        let (path, raw_query) = match url.split_once('?') {
            Some((p, q)) => (p.to_string(), q.to_string()),
            None => (url.to_string(), String::new()),
        };
        Self {
            method: method.to_uppercase(),
            path,
            raw_query,
            headers: BTreeMap::new(),
            body: Vec::new(),
            remote_addr: "127.0.0.1:4000".to_string(),
        }
    }

    pub fn with_body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }

    pub fn with_header(mut self, key: &str, value: &str) -> Self {
        self.headers.insert(key.to_string(), value.to_string());
        self
    }

    pub fn with_remote_addr(mut self, remote: &str) -> Self {
        if !remote.is_empty() {
            self.remote_addr = remote.to_string();
        }
        self
    }
}

#[derive(Clone, Debug)]
pub struct HttpResponse {
    pub code: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn body_string(&self) -> String {
        String::from_utf8_lossy(&self.body).to_string()
    }

    pub fn header(&self, name: &str) -> String {
        for (k, v) in &self.headers {
            if k.eq_ignore_ascii_case(name) {
                return v.clone();
            }
        }
        String::new()
    }

    pub fn json(&self) -> Result<Value, String> {
        Value::parse_json(&self.body_string())
    }
}

pub struct SseEventMessage {
    pub event: String,
    pub data: String,
}

pub struct HttpContext {
    pub method: String,
    pub path: String,
    pub raw_query: String,
    pub query_params: BTreeMap<String, Vec<String>>,
    pub req_headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
    pub remote_addr: String,
    pub params: BTreeMap<String, String>,
    pub store: ValueMap,
    pub status_code: u16,
    pub resp_headers: BTreeMap<String, String>,
    pub resp_body: Vec<u8>,
    pub aborted: bool,
    pub sse_receiver: Option<Receiver<SseEventMessage>>,
    trusted_proxies: Vec<IpNet>,
    handlers: Vec<HandlerFunc>,
    index: usize,
}

impl HttpContext {
    pub fn new(req: HttpRequest, trusted_proxies: Vec<IpNet>, handlers: Vec<HandlerFunc>) -> Self {
        let query_params = parse_query_string(&req.raw_query);
        Self {
            method: req.method.to_uppercase(),
            path: req.path,
            raw_query: req.raw_query,
            query_params,
            req_headers: req.headers,
            body: req.body,
            remote_addr: req.remote_addr,
            params: BTreeMap::new(),
            store: ValueMap::new(),
            status_code: 200,
            resp_headers: BTreeMap::new(),
            resp_body: Vec::new(),
            aborted: false,
            sse_receiver: None,
            trusted_proxies,
            handlers,
            index: 0,
        }
    }

    pub fn next(&mut self) {
        while self.index < self.handlers.len() && !self.aborted {
            let idx = self.index;
            self.index += 1;
            let handler = self.handlers[idx].clone();
            handler(self);
        }
    }

    pub fn abort(&mut self) {
        self.aborted = true;
    }

    pub fn abort_with_status(&mut self, status: u16) {
        self.status_code = status;
        self.aborted = true;
    }

    pub fn abort_with_status_json(&mut self, status: u16, val: Value) {
        self.json(status, val);
        self.aborted = true;
    }

    pub fn is_aborted(&self) -> bool {
        self.aborted
    }

    pub fn get_header(&self, key: &str) -> String {
        for (k, v) in &self.req_headers {
            if k.eq_ignore_ascii_case(key) {
                return v.clone();
            }
        }
        String::new()
    }

    pub fn content_type(&self) -> String {
        self.get_header("Content-Type")
    }

    pub fn header(&mut self, key: &str, value: &str) {
        self.resp_headers.insert(key.to_string(), value.to_string());
    }

    pub fn query(&self, key: &str) -> String {
        self.query_params
            .get(key)
            .and_then(|v| v.first())
            .cloned()
            .unwrap_or_default()
    }

    pub fn param(&self, key: &str) -> String {
        self.params.get(key).cloned().unwrap_or_default()
    }

    pub fn set(&mut self, key: &str, val: Value) {
        self.store.insert(key.to_string(), val);
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.store.get(key)
    }

    pub fn json(&mut self, status: u16, val: Value) {
        self.status_code = status;
        self.header("Content-Type", "application/json; charset=utf-8");
        self.resp_body = val.to_json_string().into_bytes();
    }

    pub fn data(&mut self, status: u16, content_type: &str, bytes: Vec<u8>) {
        self.status_code = status;
        self.header("Content-Type", content_type);
        self.resp_body = bytes;
    }

    pub fn client_ip(&self) -> String {
        let peer_ip = extract_ip_from_remote(&self.remote_addr);
        octopus_core::net::client_ip(
            &peer_ip,
            &self.get_header("X-Forwarded-For"),
            &self.get_header("X-Real-IP"),
            &self.trusted_proxies,
        )
    }
}

pub fn extract_ip_from_remote(remote: &str) -> String {
    let trimmed = remote.trim();
    if trimmed.is_empty() {
        return "127.0.0.1".to_string();
    }
    if trimmed.starts_with('[') {
        if let Some(end_bracket) = trimmed.find(']') {
            return trimmed[1..end_bracket].to_string();
        }
    }
    if let Ok(ip) = trimmed.parse::<IpAddr>() {
        return ip.to_string();
    }
    if let Some((host, _port)) = trimmed.rsplit_once(':') {
        if let Ok(ip) = host.parse::<IpAddr>() {
            return ip.to_string();
        }
        return host.to_string();
    }
    trimmed.to_string()
}

pub fn parse_query_string(raw: &str) -> BTreeMap<String, Vec<String>> {
    let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    if raw.trim().is_empty() {
        return map;
    }
    for pair in raw.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = match pair.split_once('=') {
            Some((k, v)) => (url_decode(k), url_decode(v)),
            None => (url_decode(pair), String::new()),
        };
        map.entry(k).or_default().push(v);
    }
    map
}

pub use octopus_core::url::url_decode;

#[derive(Clone)]
struct RouteEntry {
    method: String,
    pattern: String,
    middlewares: Vec<HandlerFunc>,
    handler: HandlerFunc,
}

#[derive(Clone, Default)]
pub struct Router {
    routes: Vec<RouteEntry>,
    middlewares: Vec<HandlerFunc>,
    trusted_proxies: Vec<IpNet>,
}

impl Router {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn use_middleware(&mut self, mw: HandlerFunc) {
        self.middlewares.push(mw);
    }

    pub fn set_trusted_proxies(&mut self, proxies: &[String]) -> Result<(), String> {
        if proxies.is_empty() {
            self.trusted_proxies.clear();
            return Ok(());
        }
        let mut parsed = Vec::with_capacity(proxies.len());
        for p in proxies {
            parsed.push(IpNet::parse(p)?);
        }
        self.trusted_proxies = parsed;
        Ok(())
    }

    pub fn handle(&mut self, method: &str, pattern: &str, handler: HandlerFunc) {
        self.handle_with_middlewares(method, pattern, Vec::new(), handler);
    }

    pub fn handle_with_middlewares(
        &mut self,
        method: &str,
        pattern: &str,
        middlewares: Vec<HandlerFunc>,
        handler: HandlerFunc,
    ) {
        let norm_method = method.to_uppercase();
        let norm_pattern = if pattern.starts_with('/') {
            pattern.to_string()
        } else {
            format!("/{}", pattern)
        };
        if let Some(existing) = self
            .routes
            .iter_mut()
            .find(|r| r.method == norm_method && r.pattern == norm_pattern)
        {
            existing.middlewares = middlewares;
            existing.handler = handler;
            return;
        }
        self.routes.push(RouteEntry {
            method: norm_method,
            pattern: norm_pattern,
            middlewares,
            handler,
        });
    }

    pub fn get<F>(&mut self, pattern: &str, handler: F)
    where
        F: Fn(&mut HttpContext) + Send + Sync + 'static,
    {
        self.handle("GET", pattern, Arc::new(handler));
    }

    pub fn get_with_mw<F>(&mut self, pattern: &str, mw: HandlerFunc, handler: F)
    where
        F: Fn(&mut HttpContext) + Send + Sync + 'static,
    {
        self.handle_with_middlewares("GET", pattern, vec![mw], Arc::new(handler));
    }

    pub fn post<F>(&mut self, pattern: &str, handler: F)
    where
        F: Fn(&mut HttpContext) + Send + Sync + 'static,
    {
        self.handle("POST", pattern, Arc::new(handler));
    }

    pub fn post_with_mw<F>(&mut self, pattern: &str, mw: HandlerFunc, handler: F)
    where
        F: Fn(&mut HttpContext) + Send + Sync + 'static,
    {
        self.handle_with_middlewares("POST", pattern, vec![mw], Arc::new(handler));
    }

    pub fn serve_http(&self, req: HttpRequest) -> HttpResponse {
        let mut ctx = self.execute_context(req);
        if let Some(rx) = ctx.sse_receiver.take() {
            let mut sse_buf = String::new();
            while let Ok(ev) = rx.recv() {
                sse_buf.push_str(&format!("event:{}\ndata:{}\n\n", ev.event, ev.data));
            }
            ctx.header("Content-Type", "text/event-stream");
            ctx.resp_body = sse_buf.into_bytes();
        }
        HttpResponse {
            code: ctx.status_code,
            headers: ctx.resp_headers,
            body: ctx.resp_body,
        }
    }

    fn execute_context(&self, req: HttpRequest) -> HttpContext {
        let method = req.method.to_uppercase();
        let path = req.path.clone();

        let mut matched_route: Option<(&RouteEntry, BTreeMap<String, String>)> = None;
        for route in &self.routes {
            if route.method == method {
                if let Some(params) = match_route_pattern(&route.pattern, &path) {
                    matched_route = Some((route, params));
                    break;
                }
            }
        }

        let mut chain: Vec<HandlerFunc> = self.middlewares.clone();
        let mut route_params = BTreeMap::new();
        if let Some((route, params)) = matched_route {
            route_params = params;
            chain.extend(route.middlewares.iter().cloned());
            chain.push(route.handler.clone());
        } else {
            chain.push(Arc::new(|c: &mut HttpContext| {
                c.status_code = 404;
                c.resp_body = b"404 page not found".to_vec();
            }));
        }

        let mut ctx = HttpContext::new(req, self.trusted_proxies.clone(), chain);
        ctx.params = route_params;
        ctx.next();
        ctx
    }

    pub fn run(&self, addr: &str) -> std::io::Result<()> {
        let bind_addr = if addr.starts_with(':') {
            format!("0.0.0.0{}", addr)
        } else {
            addr.to_string()
        };
        self.serve(TcpListener::bind(&bind_addr)?)
    }

    /// Serves connections from an already-bound listener (lets callers bind port 0).
    pub fn serve(&self, listener: TcpListener) -> std::io::Result<()> {
        for stream in listener.incoming().flatten() {
            let router_clone = self.clone();
            thread::spawn(move || {
                let _ = handle_tcp_connection(stream, &router_clone);
            });
        }
        Ok(())
    }
}

fn match_route_pattern(pattern: &str, path: &str) -> Option<BTreeMap<String, String>> {
    let pat_trimmed = pattern.trim_matches('/');
    let path_trimmed = path.trim_matches('/');

    if pat_trimmed.is_empty() && path_trimmed.is_empty() {
        return Some(BTreeMap::new());
    }
    let pat_parts: Vec<&str> = if pat_trimmed.is_empty() {
        Vec::new()
    } else {
        pat_trimmed.split('/').collect()
    };
    let path_parts: Vec<&str> = if path_trimmed.is_empty() {
        Vec::new()
    } else {
        path_trimmed.split('/').collect()
    };

    if pat_parts.len() != path_parts.len() {
        return None;
    }

    let mut params = BTreeMap::new();
    for (pat_seg, path_seg) in pat_parts.iter().zip(path_parts.iter()) {
        if let Some(param_name) = pat_seg.strip_prefix(':') {
            params.insert(param_name.to_string(), url_decode(path_seg));
        } else if pat_seg.starts_with('{') && pat_seg.ends_with('}') && pat_seg.len() > 2 {
            let param_name = &pat_seg[1..pat_seg.len() - 1];
            params.insert(param_name.to_string(), url_decode(path_seg));
        } else if pat_seg != path_seg {
            return None;
        }
    }
    Some(params)
}

/// Hard transport ceilings, applied before any middleware runs. Blueprint
/// middlewares (e.g. max_body_size_bytes) can enforce tighter limits.
pub const MAX_REQUEST_BODY_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_REQUEST_HEAD_BYTES: usize = 64 * 1024;
pub const MAX_REQUEST_HEADERS: usize = 100;
pub const REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(30);

enum HeadLine {
    Line(String),
    Eof,
    TooLarge,
}

/// Reads one line without letting it consume more than `budget` bytes.
fn read_head_line<R: BufRead>(reader: &mut R, budget: &mut usize) -> std::io::Result<HeadLine> {
    let mut buf = Vec::new();
    let n = reader
        .by_ref()
        .take(*budget as u64 + 1)
        .read_until(b'\n', &mut buf)?;
    if n == 0 {
        return Ok(HeadLine::Eof);
    }
    if n > *budget {
        return Ok(HeadLine::TooLarge);
    }
    *budget -= n;
    Ok(HeadLine::Line(String::from_utf8_lossy(&buf).into_owned()))
}

fn write_simple_response(
    stream: &mut TcpStream,
    status: u16,
    message: &str,
) -> std::io::Result<()> {
    let body = format!("{{\"error\":\"{}\"}}", message);
    write!(
        stream,
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status,
        status_reason(status),
        body.len(),
        body
    )?;
    stream.flush()
}

fn handle_tcp_connection(mut stream: TcpStream, router: &Router) -> std::io::Result<()> {
    let peer_addr = stream
        .peer_addr()
        .map(|a| a.to_string())
        .unwrap_or_else(|_| "127.0.0.1:0".to_string());
    stream.set_read_timeout(Some(REQUEST_READ_TIMEOUT))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut head_budget = MAX_REQUEST_HEAD_BYTES;

    let request_line = match read_head_line(&mut reader, &mut head_budget)? {
        HeadLine::Line(line) => line,
        HeadLine::Eof => return Ok(()),
        HeadLine::TooLarge => return write_simple_response(&mut stream, 414, "URI Too Long"),
    };
    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        return Ok(());
    }
    let method = parts[0].to_string();
    let full_url = parts[1];

    let mut headers = BTreeMap::new();
    let mut content_length = 0usize;
    loop {
        let line = match read_head_line(&mut reader, &mut head_budget)? {
            HeadLine::Line(line) => line,
            HeadLine::Eof => break,
            HeadLine::TooLarge => {
                return write_simple_response(&mut stream, 431, "Request Header Fields Too Large")
            }
        };
        if line == "\r\n" || line == "\n" {
            break;
        }
        if headers.len() >= MAX_REQUEST_HEADERS {
            return write_simple_response(&mut stream, 431, "Request Header Fields Too Large");
        }
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim().to_string();
            let val = v.trim().to_string();
            if key.eq_ignore_ascii_case("content-length") {
                content_length = match val.parse::<usize>() {
                    Ok(n) => n,
                    Err(_) => {
                        return write_simple_response(&mut stream, 400, "Invalid Content-Length")
                    }
                };
            }
            headers.insert(key, val);
        }
    }

    if content_length > MAX_REQUEST_BODY_BYTES {
        return write_simple_response(&mut stream, 413, "Payload Too Large");
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body)?;
    }

    let mut req = HttpRequest::new(&method, full_url);
    req.headers = headers;
    req.body = body;
    req.remote_addr = peer_addr;

    let mut ctx = router.execute_context(req);
    if let Some(rx) = ctx.sse_receiver.take() {
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n"
        )?;
        stream.flush()?;
        while let Ok(ev) = rx.recv() {
            if write!(stream, "event: {}\ndata: {}\n\n", ev.event, ev.data).is_err() {
                break;
            }
            if stream.flush().is_err() {
                break;
            }
        }
        return Ok(());
    }

    let reason = status_reason(ctx.status_code);
    write!(stream, "HTTP/1.1 {} {}\r\n", ctx.status_code, reason)?;
    ctx.resp_headers.insert(
        "Content-Length".to_string(),
        ctx.resp_body.len().to_string(),
    );
    ctx.resp_headers
        .entry("Connection".to_string())
        .or_insert_with(|| "close".to_string());
    for (k, v) in &ctx.resp_headers {
        write!(stream, "{}: {}\r\n", k, v)?;
    }
    write!(stream, "\r\n")?;
    stream.write_all(&ctx.resp_body)?;
    stream.flush()?;
    Ok(())
}

fn status_reason(code: u16) -> &'static str {
    match code {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        413 => "Payload Too Large",
        414 => "URI Too Long",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        504 => "Gateway Timeout",
        _ => "OK",
    }
}
