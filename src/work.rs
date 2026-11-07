//! ERPNext / Frappe integration for the Work tab (`Sprint Backlogs`).
//! Runs all network I/O on a dedicated background worker thread over WinHTTP
//! (zero external crates) and posts `WM_WORK` to the main window when ready.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use crate::gfx::Color;
use crate::store::{normalize_erp_auth, normalize_erp_url};

/// Posted to the main window whenever Sprint Backlogs data, user search, or a field update finishes.
pub const WM_WORK: u32 = 0x8000 + 13; // WM_APP + 13

pub const DEFAULT_STATUSES: &[&str] = &[
    "Open",
    "Working",
    "On Hold",
    "Pending Review",
    "For Hotfix",
    "For Release Tag",
    "Overdue",
    "Completed",
    "Closed",
    "Cancelled",
    "Template",
];

#[derive(Clone, Debug, PartialEq)]
pub struct BacklogItem {
    pub name: String,
    pub subject: String,
    pub status: String,
    pub current_assignee: String,
    pub dev_assignee: String,
    pub priority: String,
    pub sprint: String,
    pub comments: usize,
    pub modified: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UserOpt {
    pub email: String,
    pub name: String,
}

#[derive(Clone, Default)]
pub struct WorkData {
    pub user: String,
    pub items: Vec<BacklogItem>,
    pub users: Vec<UserOpt>,
    pub search_users: Vec<UserOpt>,
    pub statuses: Vec<String>,
    pub loading: bool,
    pub searching: bool,
    pub saving: bool,
    pub error: Option<String>,
    /// Latest save/action outcome `(message, ok)` consumed once by the UI thread.
    pub toast: Option<(String, bool)>,
    /// Revert payload `(doc_name, field, prev_value)` when a PUT fails.
    pub revert: Option<(String, String, String)>,
}

pub enum Req {
    Refresh {
        url: String,
        token: String,
    },
    SearchUsers {
        url: String,
        token: String,
        txt: String,
    },
    UpdateField {
        url: String,
        token: String,
        user: String,
        name: String,
        field: String,
        value: String,
        prev: String,
    },
}

pub struct Handle {
    pub data: Arc<Mutex<WorkData>>,
    pub tx: Sender<Req>,
}

pub fn start(hwnd: isize) -> Handle {
    let data = Arc::new(Mutex::new(WorkData {
        statuses: DEFAULT_STATUSES.iter().map(|s| (*s).to_string()).collect(),
        ..Default::default()
    }));
    let (tx, rx) = channel();
    let shared = data.clone();
    std::thread::Builder::new()
        .name("work".into())
        .spawn(move || worker(hwnd, shared, rx))
        .ok();
    Handle { data, tx }
}

/// Colour pill / dot per Frappe `Sprint Backlogs` status.
pub fn status_color(status: &str) -> Color {
    match status.trim() {
        "Completed" => Color(0.32, 0.84, 0.52, 1.0),
        "Working" => Color(0.36, 0.68, 1.0, 1.0),
        "Open" => Color(0.98, 0.62, 0.24, 1.0),
        "Overdue" => Color(1.0, 0.40, 0.42, 1.0),
        "Pending Review" | "For Release Tag" => Color(0.74, 0.56, 1.0, 1.0),
        "For Hotfix" => Color(1.0, 0.48, 0.62, 1.0),
        "On Hold" => Color(0.96, 0.80, 0.28, 1.0),
        "Closed" | "Cancelled" | "Template" => Color(0.56, 0.58, 0.62, 1.0),
        _ => Color(0.45, 0.72, 0.98, 1.0),
    }
}

/// Full ERPNext Desk URL for opening a Sprint Backlog item in the browser.
pub fn backlog_browser_url(erp_url: &str, name: &str) -> String {
    let base = normalize_erp_url(erp_url);
    if base.is_empty() {
        return String::new();
    }
    format!("{base}/app/sprint-backlogs/{}", url_encode(name.trim()))
}

pub fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

// ------------------------------------------------------------ tiny JSON DOM --

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn str_or(&self, key: &str) -> String {
        self.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
    }
    pub fn as_arr(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(a) => Some(a),
            _ => None,
        }
    }
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
}

pub fn parse_json(src: &str) -> Option<Json> {
    let mut p = Parser { b: src.as_bytes(), i: 0 };
    let v = p.val()?;
    p.ws();
    Some(v)
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Parser<'a> {
    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\r' | b'\n') {
            self.i += 1;
        }
    }
    fn peek(&mut self) -> Option<u8> {
        self.ws();
        self.b.get(self.i).copied()
    }
    fn val(&mut self) -> Option<Json> {
        match self.peek()? {
            b'"' => self.string().map(Json::Str),
            b'{' => self.obj(),
            b'[' => self.arr(),
            b't' if self.b[self.i..].starts_with(b"true") => {
                self.i += 4;
                Some(Json::Bool(true))
            }
            b'f' if self.b[self.i..].starts_with(b"false") => {
                self.i += 5;
                Some(Json::Bool(false))
            }
            b'n' if self.b[self.i..].starts_with(b"null") => {
                self.i += 4;
                Some(Json::Null)
            }
            c if c == b'-' || c.is_ascii_digit() => self.num(),
            _ => None,
        }
    }
    fn num(&mut self) -> Option<Json> {
        let start = self.i;
        if self.b.get(self.i) == Some(&b'-') {
            self.i += 1;
        }
        while self.i < self.b.len() && matches!(self.b[self.i], b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-') {
            self.i += 1;
        }
        let s = std::str::from_utf8(&self.b[start..self.i]).ok()?;
        s.parse::<f64>().ok().map(Json::Num)
    }
    fn hex4(&mut self) -> Option<u16> {
        let s = std::str::from_utf8(self.b.get(self.i..self.i + 4)?).ok()?;
        let v = u16::from_str_radix(s, 16).ok()?;
        self.i += 4;
        Some(v)
    }
    fn string(&mut self) -> Option<String> {
        self.ws();
        if self.b.get(self.i) != Some(&b'"') {
            return None;
        }
        self.i += 1;
        let mut out = String::new();
        while self.i < self.b.len() {
            let b = self.b[self.i];
            if b == b'"' {
                self.i += 1;
                return Some(out);
            }
            if b == b'\\' {
                self.i += 1;
                let esc = *self.b.get(self.i)?;
                self.i += 1;
                match esc {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'b' => out.push('\u{0008}'),
                    b'f' => out.push('\u{000C}'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        let hi = self.hex4()?;
                        if (0xD800..=0xDBFF).contains(&hi) && self.b.get(self.i..self.i + 2) == Some(b"\\u") {
                            self.i += 2;
                            let lo = self.hex4()?;
                            let cp = 0x10000 + (((hi as u32 - 0xD800) << 10) | (lo as u32 - 0xDC00));
                            out.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
                        } else {
                            out.push(char::from_u32(hi as u32).unwrap_or('\u{FFFD}'));
                        }
                    }
                    other => out.push(other as char),
                }
            } else {
                let start = self.i;
                while self.i < self.b.len() && self.b[self.i] != b'"' && self.b[self.i] != b'\\' {
                    self.i += 1;
                }
                out.push_str(&String::from_utf8_lossy(&self.b[start..self.i]));
            }
        }
        None
    }
    fn arr(&mut self) -> Option<Json> {
        self.i += 1;
        let mut v = Vec::new();
        if self.peek()? == b']' {
            self.i += 1;
            return Some(Json::Arr(v));
        }
        loop {
            v.push(self.val()?);
            match self.peek()? {
                b',' => self.i += 1,
                b']' => {
                    self.i += 1;
                    return Some(Json::Arr(v));
                }
                _ => return None,
            }
        }
    }
    fn obj(&mut self) -> Option<Json> {
        self.i += 1;
        let mut v = Vec::new();
        if self.peek()? == b'}' {
            self.i += 1;
            return Some(Json::Obj(v));
        }
        loop {
            let k = self.string()?;
            if self.peek()? != b':' {
                return None;
            }
            self.i += 1;
            let val = self.val()?;
            v.push((k, val));
            match self.peek()? {
                b',' => self.i += 1,
                b'}' => {
                    self.i += 1;
                    return Some(Json::Obj(v));
                }
                _ => return None,
            }
        }
    }
}

/// Count comments inside Frappe's `_comments` JSON string column (`"[{...}, ...]"`).
pub fn count_comments(raw: &str) -> usize {
    let t = raw.trim();
    if t.is_empty() || t == "[]" {
        return 0;
    }
    parse_json(t).and_then(|j| j.as_arr().map(|a| a.len())).unwrap_or(0)
}

fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Pull a short human-readable error out of a Frappe error JSON payload.
pub fn extract_frappe_error(status: u32, body: &str) -> String {
    if status == 401 || status == 403 {
        return format!("ERPNext auth failed ({status}) — check erp_token in config.ini");
    }
    if let Some(j) = parse_json(body) {
        if let Some(sm) = j.get("_server_messages").and_then(|v| v.as_str()) {
            if let Some(arr) = parse_json(sm).and_then(|a| a.as_arr().map(|s| s.to_vec())) {
                for item in arr {
                    if let Some(inner_s) = item.as_str() {
                        if let Some(inner) = parse_json(inner_s) {
                            let msg = strip_html(&inner.str_or("message"));
                            if !msg.is_empty() {
                                return msg;
                            }
                        } else {
                            let msg = strip_html(inner_s);
                            if !msg.is_empty() {
                                return msg;
                            }
                        }
                    }
                }
            }
        }
        let exc = j.str_or("exception");
        if !exc.is_empty() {
            let tail = exc.split(':').skip(1).collect::<Vec<_>>().join(":");
            let clean = strip_html(if tail.trim().is_empty() { &exc } else { &tail });
            if !clean.is_empty() {
                return clean;
            }
        }
        let msg = strip_html(&j.str_or("message"));
        if !msg.is_empty() {
            return msg;
        }
    }
    format!("HTTP {status}")
}

pub fn parse_backlog_list(body: &str) -> Vec<BacklogItem> {
    let Some(root) = parse_json(body) else { return Vec::new() };
    let arr = root
        .get("message")
        .and_then(|v| v.as_arr())
        .or_else(|| root.get("data").and_then(|v| v.as_arr()))
        .or_else(|| root.as_arr());
    let Some(arr) = arr else { return Vec::new() };
    arr.iter()
        .filter_map(|row| {
            let name = row.str_or("name");
            if name.is_empty() {
                return None;
            }
            let subject = row.str_or("subject");
            Some(BacklogItem {
                subject: if subject.trim().is_empty() { name.clone() } else { subject.trim().to_string() },
                name,
                status: {
                    let s = row.str_or("status");
                    if s.is_empty() { "Open".to_string() } else { s }
                },
                current_assignee: row.str_or("current_assignee"),
                dev_assignee: row.str_or("dev_assignee"),
                priority: row.str_or("priority"),
                sprint: row.str_or("sprint_assign"),
                comments: count_comments(&row.str_or("_comments")),
                modified: row.str_or("modified"),
            })
        })
        .collect()
}

pub fn parse_user_search(body: &str) -> Vec<UserOpt> {
    let Some(root) = parse_json(body) else { return Vec::new() };
    let arr = root
        .get("message")
        .and_then(|v| v.as_arr())
        .or_else(|| root.get("results").and_then(|v| v.as_arr()))
        .or_else(|| root.as_arr());
    let Some(arr) = arr else { return Vec::new() };
    let mut out = Vec::new();
    for row in arr {
        let email = row.str_or("value");
        if email.is_empty() {
            continue;
        }
        let desc = row.str_or("description");
        out.push(UserOpt {
            name: if desc.trim().is_empty() { email.clone() } else { desc.trim().to_string() },
            email,
        });
    }
    out
}

pub fn parse_base_url(raw: &str) -> Option<(bool, String, u16, String)> {
    let norm = normalize_erp_url(raw);
    let (https, rest) = if let Some(r) = norm.strip_prefix("https://") {
        (true, r)
    } else if let Some(r) = norm.strip_prefix("http://") {
        (false, r)
    } else {
        return None;
    };
    let (authority, base_path) = match rest.find('/') {
        Some(idx) => (&rest[..idx], rest[idx..].trim_end_matches('/').to_string()),
        None => (rest, String::new()),
    };
    if authority.is_empty() {
        return None;
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if !h.is_empty() && !h.contains(']') => {
            let port = p.parse::<u16>().ok()?;
            (h.to_string(), port)
        }
        _ => (authority.to_string(), if https { 443 } else { 80 }),
    };
    Some((https, host, port, base_path))
}

// -------------------------------------------------------- WinHTTP transport --

#[cfg(windows)]
mod http {
    use std::ffi::c_void;
    use std::ptr::{null, null_mut};

    type HINTERNET = *mut c_void;

    const WINHTTP_ACCESS_TYPE_DEFAULT_PROXY: u32 = 0;
    const WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY: u32 = 4;
    const WINHTTP_FLAG_SECURE: u32 = 0x0080_0000;
    const WINHTTP_QUERY_STATUS_CODE: u32 = 19;
    const WINHTTP_QUERY_FLAG_NUMBER: u32 = 0x2000_0000;

    #[link(name = "winhttp")]
    extern "system" {
        fn WinHttpOpen(
            agent: *const u16,
            access_type: u32,
            proxy: *const u16,
            bypass: *const u16,
            flags: u32,
        ) -> HINTERNET;
        fn WinHttpSetTimeouts(
            h: HINTERNET,
            resolve_ms: i32,
            connect_ms: i32,
            send_ms: i32,
            recv_ms: i32,
        ) -> i32;
        fn WinHttpConnect(
            session: HINTERNET,
            server: *const u16,
            port: u16,
            reserved: u32,
        ) -> HINTERNET;
        fn WinHttpOpenRequest(
            connect: HINTERNET,
            verb: *const u16,
            object: *const u16,
            version: *const u16,
            referrer: *const u16,
            accept: *const *const u16,
            flags: u32,
        ) -> HINTERNET;
        fn WinHttpSendRequest(
            request: HINTERNET,
            headers: *const u16,
            headers_len: u32,
            optional: *const c_void,
            optional_len: u32,
            total_len: u32,
            context: usize,
        ) -> i32;
        fn WinHttpReceiveResponse(request: HINTERNET, reserved: *mut c_void) -> i32;
        fn WinHttpQueryHeaders(
            request: HINTERNET,
            info_level: u32,
            name: *const u16,
            buffer: *mut c_void,
            buffer_len: *mut u32,
            index: *mut u32,
        ) -> i32;
        fn WinHttpReadData(
            request: HINTERNET,
            buffer: *mut c_void,
            to_read: u32,
            read: *mut u32,
        ) -> i32;
        fn WinHttpCloseHandle(h: HINTERNET) -> i32;
    }

    struct HandleGuard(HINTERNET);
    impl Drop for HandleGuard {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    WinHttpCloseHandle(self.0);
                }
            }
        }
    }

    pub struct Client {
        session: HINTERNET,
    }

    impl Client {
        pub fn new() -> Self {
            let agent = wide("Vey/0.13");
            let mut s = unsafe {
                WinHttpOpen(
                    agent.as_ptr(),
                    WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                    null(),
                    null(),
                    0,
                )
            };
            if s.is_null() {
                s = unsafe {
                    WinHttpOpen(
                        agent.as_ptr(),
                        WINHTTP_ACCESS_TYPE_DEFAULT_PROXY,
                        null(),
                        null(),
                        0,
                    )
                };
            }
            if !s.is_null() {
                unsafe {
                    WinHttpSetTimeouts(s, 6_000, 8_000, 10_000, 15_000);
                }
            }
            Client { session: s }
        }

        pub fn request(
            &self,
            method: &str,
            base_url: &str,
            path: &str,
            auth_header: &str,
            json_body: Option<&str>,
        ) -> Result<(u32, String), String> {
            if self.session.is_null() {
                return Err("Could not open WinHTTP session".into());
            }
            let (https, host, port, base_path) =
                super::parse_base_url(base_url).ok_or_else(|| "Invalid erp_url".to_string())?;
            let host_w = wide(&host);
            let conn = HandleGuard(unsafe { WinHttpConnect(self.session, host_w.as_ptr(), port, 0) });
            if conn.0.is_null() {
                return Err(format!("Could not connect to {host}"));
            }
            let full_path = format!("{base_path}{path}");
            let verb_w = wide(method);
            let path_w = wide(&full_path);
            let flags = if https { WINHTTP_FLAG_SECURE } else { 0 };
            let req = HandleGuard(unsafe {
                WinHttpOpenRequest(conn.0, verb_w.as_ptr(), path_w.as_ptr(), null(), null(), null(), flags)
            });
            if req.0.is_null() {
                return Err("Could not create HTTP request".into());
            }
            let mut hdr = format!("Accept: application/json\r\nAuthorization: {auth_header}\r\n");
            if json_body.is_some() {
                hdr.push_str("Content-Type: application/json; charset=utf-8\r\n");
            }
            let hdr_w = wide(&hdr);
            let body_bytes = json_body.map(|b| b.as_bytes()).unwrap_or(&[]);
            let body_ptr = if body_bytes.is_empty() {
                null()
            } else {
                body_bytes.as_ptr() as *const c_void
            };
            let len = body_bytes.len() as u32;
            let sent = unsafe {
                WinHttpSendRequest(
                    req.0,
                    hdr_w.as_ptr(),
                    u32::MAX,
                    body_ptr,
                    len,
                    len,
                    0,
                )
            };
            if sent == 0 {
                return Err(format!("Cannot reach {host}"));
            }
            if unsafe { WinHttpReceiveResponse(req.0, null_mut()) } == 0 {
                return Err(format!("No response from {host}"));
            }
            let mut status: u32 = 0;
            let mut status_sz = std::mem::size_of::<u32>() as u32;
            unsafe {
                WinHttpQueryHeaders(
                    req.0,
                    WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                    null(),
                    &mut status as *mut u32 as *mut c_void,
                    &mut status_sz,
                    null_mut(),
                );
            }
            let mut out = Vec::new();
            let mut buf = [0u8; 8192];
            loop {
                let mut n: u32 = 0;
                let ok = unsafe {
                    WinHttpReadData(req.0, buf.as_mut_ptr() as *mut c_void, buf.len() as u32, &mut n)
                };
                if ok == 0 || n == 0 {
                    break;
                }
                out.extend_from_slice(&buf[..n as usize]);
            }
            Ok((status, String::from_utf8_lossy(&out).into_owned()))
        }
    }

    impl Drop for Client {
        fn drop(&mut self) {
            if !self.session.is_null() {
                unsafe {
                    WinHttpCloseHandle(self.session);
                }
            }
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

#[cfg(windows)]
fn post_ui(hwnd: isize) {
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
    if hwnd != 0 {
        unsafe {
            let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_WORK, WPARAM(0), LPARAM(0));
        }
    }
}

fn merge_users(dst: &mut Vec<UserOpt>, incoming: Vec<UserOpt>) {
    for u in incoming {
        if let Some(existing) = dst.iter_mut().find(|x| x.email.eq_ignore_ascii_case(&u.email)) {
            if existing.name == existing.email && u.name != u.email && !u.name.is_empty() {
                existing.name = u.name;
            }
        } else {
            dst.push(u);
        }
    }
}

#[cfg(windows)]
fn worker(hwnd: isize, shared: Arc<Mutex<WorkData>>, rx: Receiver<Req>) {
    let client = http::Client::new();
    while let Ok(first) = rx.recv() {
        let mut pending = vec![first];
        while let Ok(next) = rx.try_recv() {
            // If multiple SearchUsers queued up while typing fast, keep only the latest one.
            if matches!(next, Req::SearchUsers { .. }) {
                pending.retain(|r| !matches!(r, Req::SearchUsers { .. }));
            }
            pending.push(next);
        }
        for req in pending {
            match req {
                Req::Refresh { url, token } => do_refresh(&client, hwnd, &shared, &url, &token),
                Req::SearchUsers { url, token, txt } => do_search_users(&client, hwnd, &shared, &url, &token, &txt),
                Req::UpdateField { url, token, user, name, field, value, prev } => {
                    do_update_field(&client, hwnd, &shared, &url, &token, &user, &name, &field, &value, &prev);
                }
            }
        }
    }
}

#[cfg(windows)]
fn do_refresh(client: &http::Client, hwnd: isize, shared: &Arc<Mutex<WorkData>>, raw_url: &str, raw_tok: &str) {
    let url = normalize_erp_url(raw_url);
    let auth = normalize_erp_auth(raw_tok);
    if url.is_empty() || auth.is_empty() {
        let mut d = shared.lock().unwrap();
        d.loading = false;
        d.error = Some("Configure erp_url and erp_token in config.ini".into());
        drop(d);
        post_ui(hwnd);
        return;
    }

    // 1. Resolve logged-in user for "my sprint backlogs"
    let user = match client.request("GET", &url, "/api/method/frappe.auth.get_logged_user", &auth, None) {
        Ok((200..=299, body)) => parse_json(&body)
            .and_then(|j| j.get("message").and_then(|m| m.as_str()).map(str::to_string))
            .unwrap_or_default(),
        Ok((code, body)) => {
            let mut d = shared.lock().unwrap();
            d.loading = false;
            d.error = Some(extract_frappe_error(code, &body));
            drop(d);
            post_ui(hwnd);
            return;
        }
        Err(e) => {
            let mut d = shared.lock().unwrap();
            d.loading = false;
            d.error = Some(e);
            drop(d);
            post_ui(hwnd);
            return;
        }
    };

    // 2. Fetch my Sprint Backlogs (assigned as current, dev, QA, or tech assignee)
    let u_json = json_str(&user);
    let list_body = if user.is_empty() {
        r#"{"doctype":"Sprint Backlogs","fields":["name","subject","status","current_assignee","dev_assignee","priority","sprint_assign","_comments","modified"],"order_by":"modified desc","limit_page_length":60}"#.to_string()
    } else {
        format!(
            r#"{{"doctype":"Sprint Backlogs","fields":["name","subject","status","current_assignee","dev_assignee","priority","sprint_assign","_comments","modified"],"or_filters":[["Sprint Backlogs","current_assignee","=",{u}],["Sprint Backlogs","dev_assignee","=",{u}],["Sprint Backlogs","qa_assignee","=",{u}],["Sprint Backlogs","tech_assignee","=",{u}]],"order_by":"modified desc","limit_page_length":80}}"#,
            u = u_json
        )
    };

    let items = match client.request("POST", &url, "/api/method/frappe.client.get_list", &auth, Some(&list_body)) {
        Ok((200..=299, body)) => parse_backlog_list(&body),
        Ok((code, body)) => {
            let mut d = shared.lock().unwrap();
            d.loading = false;
            d.error = Some(extract_frappe_error(code, &body));
            drop(d);
            post_ui(hwnd);
            return;
        }
        Err(e) => {
            let mut d = shared.lock().unwrap();
            d.loading = false;
            d.error = Some(e);
            drop(d);
            post_ui(hwnd);
            return;
        }
    };

    // 3. Seed user directory so the Assignee dropdown opens instantaneously
    let seed_body = r#"{"doctype":"User","txt":"","reference_doctype":"Sprint Backlogs","page_length":30}"#;
    let mut seed_users = match client.request("POST", &url, "/api/method/frappe.desk.search.search_link", &auth, Some(seed_body)) {
        Ok((200..=299, body)) => parse_user_search(&body),
        _ => Vec::new(),
    };
    if !user.is_empty() {
        // Also look up the logged-in user's full name if not in the top 30 alphabetical users
        if !seed_users.iter().any(|u| u.email.eq_ignore_ascii_case(&user)) {
            let me_body = format!(
                r#"{{"doctype":"User","txt":{},"reference_doctype":"Sprint Backlogs","page_length":5}}"#,
                json_str(&user)
            );
            if let Ok((200..=299, body)) =
                client.request("POST", &url, "/api/method/frappe.desk.search.search_link", &auth, Some(&me_body))
            {
                merge_users(&mut seed_users, parse_user_search(&body));
            }
        }
    }
    for it in &items {
        for email in [&it.current_assignee, &it.dev_assignee] {
            let e = email.trim();
            if !e.is_empty() && !seed_users.iter().any(|u| u.email.eq_ignore_ascii_case(e)) {
                seed_users.push(UserOpt { email: e.to_string(), name: e.to_string() });
            }
        }
    }

    let mut d = shared.lock().unwrap();
    d.user = user;
    for it in &items {
        if !it.status.is_empty() && !d.statuses.iter().any(|s| s == &it.status) {
            d.statuses.push(it.status.clone());
        }
    }
    d.items = items;
    merge_users(&mut d.users, seed_users);
    if d.search_users.is_empty() {
        d.search_users = d.users.clone();
    }
    d.loading = false;
    d.error = None;
    drop(d);
    post_ui(hwnd);
}

#[cfg(windows)]
fn do_search_users(
    client: &http::Client,
    hwnd: isize,
    shared: &Arc<Mutex<WorkData>>,
    raw_url: &str,
    raw_tok: &str,
    txt: &str,
) {
    let url = normalize_erp_url(raw_url);
    let auth = normalize_erp_auth(raw_tok);
    if url.is_empty() || auth.is_empty() {
        let mut d = shared.lock().unwrap();
        d.searching = false;
        drop(d);
        post_ui(hwnd);
        return;
    }
    let body = format!(
        r#"{{"doctype":"User","txt":{},"reference_doctype":"Sprint Backlogs","page_length":25}}"#,
        json_str(txt.trim())
    );
    let found = match client.request("POST", &url, "/api/method/frappe.desk.search.search_link", &auth, Some(&body)) {
        Ok((200..=299, resp)) => Some(parse_user_search(&resp)),
        _ => None,
    };
    let mut d = shared.lock().unwrap();
    if let Some(list) = found {
        merge_users(&mut d.users, list.clone());
        d.search_users = list;
    }
    d.searching = false;
    drop(d);
    post_ui(hwnd);
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn do_update_field(
    client: &http::Client,
    hwnd: isize,
    shared: &Arc<Mutex<WorkData>>,
    raw_url: &str,
    raw_tok: &str,
    logged_user: &str,
    name: &str,
    field: &str,
    value: &str,
    prev: &str,
) {
    let url = normalize_erp_url(raw_url);
    let auth = normalize_erp_auth(raw_tok);
    let path = format!("/api/resource/Sprint%20Backlogs/{}", url_encode(name));
    let payload = if field == "status" && value == "Completed" {
        let (y, m, d, ..) = crate::sys::local_time();
        let today = format!("{y:04}-{m:02}-{d:02}");
        if logged_user.is_empty() {
            format!(r#"{{"status":"Completed","completed_on":{}}}"#, json_str(&today))
        } else {
            format!(
                r#"{{"status":"Completed","completed_on":{},"completed_by":{}}}"#,
                json_str(&today),
                json_str(logged_user)
            )
        }
    } else {
        format!("{{{}:{}}}", json_str(field), json_str(value))
    };

    let res = client.request("PUT", &url, &path, &auth, Some(&payload));
    let mut d = shared.lock().unwrap();
    d.saving = false;
    match res {
        Ok((200..=299, body)) => {
            if let Some(doc) = parse_json(&body).and_then(|j| j.get("data").cloned()) {
                if let Some(it) = d.items.iter_mut().find(|i| i.name == name) {
                    let s = doc.str_or("status");
                    if !s.is_empty() {
                        it.status = s;
                    }
                    let ca = doc.str_or("current_assignee");
                    if !ca.is_empty() || field == "current_assignee" {
                        it.current_assignee = ca;
                    }
                    if doc.get("_comments").is_some() {
                        it.comments = count_comments(&doc.str_or("_comments"));
                    }
                }
            } else if let Some(it) = d.items.iter_mut().find(|i| i.name == name) {
                if field == "status" {
                    it.status = value.to_string();
                } else if field == "current_assignee" {
                    it.current_assignee = value.to_string();
                }
            }
            d.toast = Some(("Saved".into(), true));
        }
        Ok((code, body)) => {
            if let Some(it) = d.items.iter_mut().find(|i| i.name == name) {
                if field == "status" {
                    it.status = prev.to_string();
                } else if field == "current_assignee" {
                    it.current_assignee = prev.to_string();
                }
            }
            d.revert = Some((name.to_string(), field.to_string(), prev.to_string()));
            d.toast = Some((extract_frappe_error(code, &body), false));
        }
        Err(e) => {
            if let Some(it) = d.items.iter_mut().find(|i| i.name == name) {
                if field == "status" {
                    it.status = prev.to_string();
                } else if field == "current_assignee" {
                    it.current_assignee = prev.to_string();
                }
            }
            d.revert = Some((name.to_string(), field.to_string(), prev.to_string()));
            d.toast = Some((e, false));
        }
    }
    drop(d);
    post_ui(hwnd);
}

#[cfg(not(windows))]
fn worker(_hwnd: isize, _shared: Arc<Mutex<WorkData>>, _rx: Receiver<Req>) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sprint_backlogs_and_comment_counts() {
        let body = r#"{
            "message": [
                {
                    "name": "SPB-07407",
                    "subject": "Develop Teacher Quiz Analytics in SILID LMS v3",
                    "status": "Overdue",
                    "current_assignee": "hervey.geralph@livro.systems",
                    "dev_assignee": "hervey.geralph@livro.systems",
                    "priority": "Medium",
                    "sprint_assign": "SPR-0100: Research & Development - Sprint 22 R&D",
                    "_comments": "[{\"comment\":\"<p>Hi</p>\",\"by\":\"a@b.c\",\"name\":\"1\"},{\"comment\":\"<p>Done \\u2014 ok</p>\",\"by\":\"d@e.f\",\"name\":\"2\"}]",
                    "modified": "2026-11-06 21:52:39.826860"
                }
            ]
        }"#;
        let items = parse_backlog_list(body);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "SPB-07407");
        assert_eq!(items[0].subject, "Develop Teacher Quiz Analytics in SILID LMS v3");
        assert_eq!(items[0].status, "Overdue");
        assert_eq!(items[0].current_assignee, "hervey.geralph@livro.systems");
        assert_eq!(items[0].comments, 2);
    }

    #[test]
    fn parses_search_link_results() {
        let body = r#"{"message":[{"value":"denesse@livro.systems","description":"Denesse"},{"value":"cllamis@livro.systems","description":"Cllamis"}]}"#;
        let u = parse_user_search(body);
        assert_eq!(u.len(), 2);
        assert_eq!(u[0].email, "denesse@livro.systems");
        assert_eq!(u[0].name, "Denesse");
    }

    #[test]
    fn builds_browser_url_and_parses_base() {
        assert_eq!(
            backlog_browser_url("https://erp.livro.sytems/", "SPB-07407"),
            "https://erp.livro.systems/app/sprint-backlogs/SPB-07407"
        );
        assert_eq!(
            parse_base_url("https://erp.livro.sytems"),
            Some((true, "erp.livro.systems".into(), 443, "".into()))
        );
        assert_eq!(
            parse_base_url("http://frappe.local:8000/sub/"),
            Some((false, "frappe.local".into(), 8000, "/sub".into()))
        );
    }

    #[test]
    fn extracts_frappe_server_messages() {
        let err = r#"{"_server_messages":"[\"{\\\"message\\\": \\\"<b>Completed On</b> is mandatory\\\"}\"]"}"#;
        assert_eq!(extract_frappe_error(417, err), "Completed On is mandatory");
    }
}
