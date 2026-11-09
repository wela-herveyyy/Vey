//! OpenAI OAuth 2.0 PKCE + ChatGPT Codex backend integration for Vey (`Page::Chat`).
//! Ported from `openai-oauth` (`packages/core`, `packages/local`, `packages/openai-oauth`):
//! - Discovers and persists `~/.codex/auth.json` (`$CODEX_HOME/auth.json` or `%USERPROFILE%\.codex\auth.json`)
//! - Runs browser OAuth 2.0 PKCE sign-in with local callback server on `http://localhost:1455/auth/callback`
//! - Automatically refreshes OAuth tokens (`https://auth.openai.com/oauth/token`)
//! - Discovers Codex models (`https://chatgpt.com/backend-api/codex/models`)
//! - Streams multi-turn responses (`https://chatgpt.com/backend-api/codex/responses`) over WinHTTP SSE

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::work::{json_str, parse_json, url_encode, Json};

/// Posted to the main window whenever OAuth state, model catalog, or streaming chat output updates.
pub const WM_CODEX: u32 = 0x8000 + 15; // WM_APP + 15

pub const DEFAULT_OPENAI_OAUTH_ISSUER: &str = "https://auth.openai.com";
pub const DEFAULT_OPENAI_OAUTH_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub const DEFAULT_OPENAI_OAUTH_SCOPE: &str = "openid profile email offline_access";
pub const DEFAULT_CODEX_CLIENT_VERSION: &str = "0.144.1";
pub const DEFAULT_LOGIN_PORT: u16 = 1455;
pub const REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
pub const DEFAULT_MODEL: &str = "gpt-5.4-mini";

pub const FALLBACK_MODELS: &[&str] = &[
    "gpt-5.4-mini",
    "gpt-5.4",
    "gpt-5.3-codex",
    "gpt-5.2-codex",
    "gpt-5.1-codex-mini",
];

const CALLBACK_SUCCESS_HTML: &str = concat!(
    "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\"/>",
    "<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"/>",
    "<title>Sign-in Complete \u{00B7} Vey</title>",
    "<style>",
    ":root{color-scheme:dark;--bg:#0a0a0c;--card:#16161a;--text:#f4f4f6;--muted:#8e8e98;--accent:#4cd964;}",
    "*{box-sizing:border-box;margin:0;padding:0;}",
    "body{min-height:100vh;display:grid;place-items:center;padding:24px;background:var(--bg);",
    "color:var(--text);font-family:'Segoe UI',-apple-system,BlinkMacSystemFont,sans-serif;}",
    ".card{width:min(100%,420px);padding:32px 28px;border-radius:22px;background:var(--card);",
    "text-align:center;box-shadow:0 24px 60px rgba(0,0,0,0.55);}",
    ".badge{width:48px;height:48px;border-radius:24px;margin:0 auto 18px;display:grid;place-items:center;",
    "background:rgba(76,217,100,0.16);color:var(--accent);font-size:22px;font-weight:700;}",
    "h1{font-size:20px;font-weight:600;margin-bottom:10px;letter-spacing:-0.01em;}",
    "p{font-size:14px;line-height:1.55;color:var(--muted);}",
    "</style></head><body><main class=\"card\">",
    "<div class=\"badge\">\u{2713}</div>",
    "<h1>Sign-in complete</h1>",
    "<p>Your ChatGPT account is connected to Vey and saved to <code>~/.codex/auth.json</code>. You can close this tab and return to Vey.</p>",
    "</main></body></html>"
);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChatMessage {
    pub role: Role,
    pub text: String,
}

pub type ModelOpt = CodexModelInfo;
pub type ChatTurn = ChatMessage;

#[derive(Clone, Debug, PartialEq)]
pub struct CodexModelInfo {
    pub slug: String,
    pub visibility: Option<String>,
    pub supported_in_api: Option<bool>,
    pub use_responses_lite: bool,
    pub support_verbosity: bool,
    pub default_verbosity: Option<String>,
    pub default_reasoning_level: Option<String>,
}

impl CodexModelInfo {
    pub fn fallback(slug: &str) -> Self {
        CodexModelInfo {
            slug: slug.to_string(),
            visibility: Some("list".into()),
            supported_in_api: Some(true),
            use_responses_lite: false,
            support_verbosity: false,
            default_verbosity: None,
            default_reasoning_level: None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CodexTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub id_token: String,
    pub account_id: String,
    pub is_fedramp: bool,
    pub email: String,
    pub plan_type: String,
    pub last_refresh_epoch_s: Option<i64>,
}

impl CodexTokens {
    pub fn is_authenticated(&self) -> bool {
        (!self.access_token.trim().is_empty() || !self.refresh_token.trim().is_empty())
            && !self.account_id.trim().is_empty()
    }

    pub fn display_label(&self) -> String {
        if !self.email.trim().is_empty() {
            if !self.plan_type.trim().is_empty() {
                format!("{} \u{00B7} {}", self.email.trim(), self.plan_type.trim())
            } else {
                self.email.trim().to_string()
            }
        } else if !self.account_id.trim().is_empty() {
            let id = self.account_id.trim();
            if id.len() > 14 {
                format!("ChatGPT \u{00B7} {}\u{2026}", &id[..10])
            } else {
                format!("ChatGPT \u{00B7} {id}")
            }
        } else {
            "ChatGPT Connected".to_string()
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChatAttachment {
    pub name: String,
    pub path: PathBuf,
    pub size_bytes: u64,
    pub text_content: Option<String>,
}

impl ChatAttachment {
    pub fn from_path(p: PathBuf) -> Result<Self, String> {
        let meta = std::fs::metadata(&p).map_err(|e| e.to_string())?;
        let name = p
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file")
            .to_string();
        let size_bytes = meta.len();
        let text_content = if size_bytes <= 512 * 1024 {
            std::fs::read_to_string(&p).ok()
        } else {
            None
        };
        Ok(ChatAttachment {
            name,
            path: p,
            size_bytes,
            text_content,
        })
    }

    pub fn size_label(&self) -> String {
        if self.size_bytes < 1024 {
            format!("{} B", self.size_bytes)
        } else if self.size_bytes < 1024 * 1024 {
            format!("{:.1} KB", self.size_bytes as f64 / 1024.0)
        } else {
            format!("{:.1} MB", self.size_bytes as f64 / (1024.0 * 1024.0))
        }
    }
}

#[derive(Clone, Default)]
pub struct CodexData {
    pub signed_in: bool,
    pub signing_in: bool,
    pub account_label: String,
    pub account_id: String,
    pub auth_path: String,
    pub models: Vec<CodexModelInfo>,
    pub selected_model: String,
    pub messages: Vec<ChatTurn>,
    pub streaming: bool,
    pub loading_models: bool,
    pub error: Option<String>,
    pub toast: Option<(String, bool)>,
    pub attachment: Option<ChatAttachment>,
}

pub enum Req {
    ReloadAuth,
    StartLogin,
    Logout,
    RefreshModels,
    AttachFile(ChatAttachment),
    SendChat {
        model: String,
        history: Vec<ChatTurn>,
    },
}

pub struct Handle {
    pub data: Arc<Mutex<CodexData>>,
    pub tx: Sender<Req>,
    #[allow(dead_code)]
    pub cancel_login: Arc<AtomicBool>,
    #[allow(dead_code)]
    pub cancel_stream: Arc<AtomicBool>,
}

pub fn start(hwnd: isize) -> Handle {
    let auth_path = resolve_codex_auth_file_path(None);
    let initial_tokens = load_codex_tokens(&auth_path).ok();
    let signed_in = initial_tokens.as_ref().is_some_and(|t| t.is_authenticated());
    let account_label = initial_tokens
        .as_ref()
        .map(|t| t.display_label())
        .unwrap_or_default();
    let account_id = initial_tokens
        .as_ref()
        .map(|t| t.account_id.clone())
        .unwrap_or_default();
    let models: Vec<CodexModelInfo> = FALLBACK_MODELS
        .iter()
        .map(|s| CodexModelInfo::fallback(s))
        .collect();

    let data = Arc::new(Mutex::new(CodexData {
        signed_in,
        signing_in: false,
        account_label,
        account_id,
        auth_path: auth_path.to_string_lossy().into_owned(),
        models,
        selected_model: DEFAULT_MODEL.to_string(),
        messages: Vec::new(),
        streaming: false,
        loading_models: false,
        error: None,
        toast: None,
        attachment: None,
    }));
    let cancel_login = Arc::new(AtomicBool::new(false));
    let cancel_stream = Arc::new(AtomicBool::new(false));
    let (tx, rx) = channel();
    let shared = data.clone();
    let cl = cancel_login.clone();
    let cs = cancel_stream.clone();
    std::thread::Builder::new()
        .name("codex".into())
        .spawn(move || worker(hwnd, shared, rx, cl, cs))
        .ok();
    if signed_in && hwnd != 0 {
        let _ = tx.send(Req::RefreshModels);
    }
    Handle {
        data,
        tx,
        cancel_login,
        cancel_stream,
    }
}

// ------------------------------------------------ SHA-256 + Base64URL + PKCE -

const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
        0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut padded = Vec::with_capacity((data.len() + 72) & !63);
    padded.extend_from_slice(data);
    padded.push(0x80);
    while (padded.len() % 64) != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in padded.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, word) in chunk.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(SHA256_K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

pub fn base64url_encode(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for ch in bytes.chunks(3) {
        let b0 = ch[0] as u32;
        let b1 = if ch.len() > 1 { ch[1] as u32 } else { 0 };
        let b2 = if ch.len() > 2 { ch[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        if ch.len() > 1 {
            out.push(T[(n >> 6) as usize & 63] as char);
        }
        if ch.len() > 2 {
            out.push(T[n as usize & 63] as char);
        }
    }
    out
}

pub fn base64url_decode(input: &str) -> Option<Vec<u8>> {
    let clean = input.trim().trim_end_matches('=');
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0u32;
    for b in clean.bytes() {
        let val = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            b' ' | b'\r' | b'\n' | b'\t' => continue,
            _ => return None,
        };
        buf = (buf << 6) | (val as u32);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xFF) as u8);
        }
    }
    Some(out)
}

fn fill_random_bytes(out: &mut [u8]) {
    #[cfg(windows)]
    {
        #[link(name = "advapi32")]
        extern "system" {
            fn SystemFunction036(buf: *mut u8, len: u32) -> u8;
        }
        let ok = unsafe { SystemFunction036(out.as_mut_ptr(), out.len() as u32) };
        if ok != 0 {
            return;
        }
    }
    // Fallback mixer using OS RandomState + high-resolution clock
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let mut pos = 0;
    let mut ctr = 0u64;
    while pos < out.len() {
        let s = RandomState::new();
        let mut h = s.build_hasher();
        h.write_u64(ctr);
        if let Ok(d) = SystemTime::now().duration_since(UNIX_EPOCH) {
            h.write_u128(d.as_nanos());
        }
        let block = sha256(&h.finish().to_le_bytes());
        let take = (out.len() - pos).min(block.len());
        out[pos..pos + take].copy_from_slice(&block[..take]);
        pos += take;
        ctr = ctr.wrapping_add(1);
    }
}

pub fn create_oauth_state() -> String {
    let mut b = [0u8; 24];
    fill_random_bytes(&mut b);
    base64url_encode(&b)
}

pub fn create_code_verifier() -> String {
    let mut b = [0u8; 48];
    fill_random_bytes(&mut b);
    base64url_encode(&b)
}

pub fn create_code_challenge(verifier: &str) -> String {
    base64url_encode(&sha256(verifier.as_bytes()))
}

pub fn build_openai_oauth_authorization_url(
    redirect_uri: &str,
    state: &str,
    code_challenge: &str,
    issuer: &str,
    client_id: &str,
    scope: &str,
) -> String {
    let base = issuer.trim_end_matches('/');
    format!(
        "{base}/oauth/authorize?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&code_challenge={}&code_challenge_method=S256&id_token_add_organizations=true&codex_cli_simplified_flow=true",
        url_encode(client_id),
        url_encode(redirect_uri),
        url_encode(scope),
        url_encode(state),
        url_encode(code_challenge),
    )
}

// -------------------------------------- JWT Claims & `~/.codex/auth.json` ----

pub fn resolve_codex_auth_file_path(explicit: Option<&str>) -> PathBuf {
    if let Some(p) = explicit {
        if !p.trim().is_empty() {
            return PathBuf::from(p.trim());
        }
    }
    if let Ok(codex_home) = std::env::var("CODEX_HOME") {
        if !codex_home.trim().is_empty() {
            return PathBuf::from(codex_home.trim()).join("auth.json");
        }
    }
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".codex").join("auth.json")
}

pub fn parse_jwt_claims(token: &str) -> Option<Json> {
    let mut parts = token.split('.');
    let _header = parts.next()?;
    let payload = parts.next()?;
    let _sig = parts.next()?;
    let decoded = base64url_decode(payload)?;
    let text = std::str::from_utf8(&decoded).ok()?;
    parse_json(text)
}

pub fn derive_account_id(claims: Option<&Json>) -> Option<String> {
    let c = claims?;
    if let Some(auth) = c.get("https://api.openai.com/auth") {
        let id = auth.str_or("chatgpt_account_id");
        if !id.trim().is_empty() {
            return Some(id);
        }
    }
    let direct = c.str_or("chatgpt_account_id");
    if !direct.trim().is_empty() {
        return Some(direct);
    }
    if let Some(orgs) = c.get("organizations").and_then(|o| o.as_arr()) {
        if let Some(first) = orgs.first() {
            let id = first.str_or("id");
            if !id.trim().is_empty() {
                return Some(id);
            }
        }
    }
    None
}

pub fn derive_chatgpt_account_is_fedramp(claims: Option<&Json>) -> bool {
    claims
        .and_then(|c| c.get("https://api.openai.com/auth"))
        .and_then(|a| a.get("chatgpt_account_is_fedramp"))
        == Some(&Json::Bool(true))
}

pub fn derive_user_email(claims: Option<&Json>) -> Option<String> {
    let c = claims?;
    let e = c.str_or("email");
    if !e.trim().is_empty() {
        return Some(e);
    }
    if let Some(prof) = c.get("https://api.openai.com/profile") {
        let pe = prof.str_or("email");
        if !pe.trim().is_empty() {
            return Some(pe);
        }
    }
    None
}

pub fn derive_plan_type(claims: Option<&Json>) -> Option<String> {
    let c = claims?;
    if let Some(auth) = c.get("https://api.openai.com/auth") {
        let p = auth.str_or("chatgpt_plan_type");
        if !p.trim().is_empty() {
            return Some(p);
        }
    }
    None
}

pub fn jwt_exp(token: &str) -> Option<i64> {
    let claims = parse_jwt_claims(token)?;
    match claims.get("exp")? {
        Json::Num(n) if n.is_finite() => Some(*n as i64),
        _ => None,
    }
}

fn now_epoch_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn should_refresh_access_token(
    access_token: &str,
    last_refresh_epoch_s: Option<i64>,
    now_s: i64,
) -> bool {
    if access_token.trim().is_empty() {
        return true;
    }
    if let Some(exp) = jwt_exp(access_token) {
        if exp <= now_s + 300 {
            return true;
        }
    }
    if let Some(last) = last_refresh_epoch_s {
        if now_s.saturating_sub(last) > 55 * 60 {
            return true;
        }
    }
    false
}

pub fn load_codex_tokens(path: &std::path::Path) -> Result<CodexTokens, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|_| format!("Codex auth file not found at {}", path.display()))?;
    parse_codex_auth_json(&raw)
}

pub fn parse_codex_auth_json(raw: &str) -> Result<CodexTokens, String> {
    let root = parse_json(raw).ok_or_else(|| "Invalid JSON in ~/.codex/auth.json".to_string())?;
    let tok_obj = root
        .get("tokens")
        .ok_or_else(|| "Missing tokens object in ~/.codex/auth.json".to_string())?;
    let access_token = tok_obj.str_or("access_token");
    let refresh_token = tok_obj.str_or("refresh_token");
    let id_token = tok_obj.str_or("id_token");
    let explicit_account_id = tok_obj.str_or("account_id");

    let id_claims = if !id_token.is_empty() {
        parse_jwt_claims(&id_token)
    } else {
        None
    };
    let access_claims = if !access_token.is_empty() {
        parse_jwt_claims(&access_token)
    } else {
        None
    };

    let account_id = if !explicit_account_id.trim().is_empty() {
        explicit_account_id
    } else {
        derive_account_id(id_claims.as_ref())
            .or_else(|| derive_account_id(access_claims.as_ref()))
            .unwrap_or_default()
    };

    if account_id.trim().is_empty() {
        return Err("Unable to determine ChatGPT account_id from ~/.codex/auth.json".into());
    }
    if access_token.trim().is_empty() && refresh_token.trim().is_empty() {
        return Err("~/.codex/auth.json does not contain usable OAuth tokens".into());
    }

    let is_fedramp = derive_chatgpt_account_is_fedramp(id_claims.as_ref())
        || derive_chatgpt_account_is_fedramp(access_claims.as_ref());
    let email = derive_user_email(id_claims.as_ref())
        .or_else(|| derive_user_email(access_claims.as_ref()))
        .unwrap_or_default();
    let plan_type = derive_plan_type(id_claims.as_ref())
        .or_else(|| derive_plan_type(access_claims.as_ref()))
        .unwrap_or_default();

    let last_refresh_epoch_s = root.get("last_refresh").and_then(|v| match v {
        Json::Num(n) => Some(*n as i64),
        Json::Str(s) => s.parse::<i64>().ok(),
        _ => None,
    });

    Ok(CodexTokens {
        access_token,
        refresh_token,
        id_token,
        account_id,
        is_fedramp,
        email,
        plan_type,
        last_refresh_epoch_s,
    })
}

pub fn save_codex_tokens(path: &std::path::Path, tokens: &CodexTokens) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let last_refresh = tokens.last_refresh_epoch_s.unwrap_or_else(now_epoch_secs);
    let body = format!(
        "{{\n  \"auth_mode\": \"chatgpt\",\n  \"tokens\": {{\n    \"id_token\": {},\n    \"access_token\": {},\n    \"refresh_token\": {},\n    \"account_id\": {}\n  }},\n  \"last_refresh\": {}\n}}\n",
        json_str(&tokens.id_token),
        json_str(&tokens.access_token),
        json_str(&tokens.refresh_token),
        json_str(&tokens.account_id),
        last_refresh,
    );
    std::fs::write(path, body).map_err(|e| format!("Failed to write {}: {e}", path.display()))
}

// ----------------------------------------- Models & Responses Normalization --

pub fn is_public_codex_model(m: &CodexModelInfo) -> bool {
    m.supported_in_api != Some(false)
}

pub fn parse_models_catalog(body: &str) -> Vec<CodexModelInfo> {
    let Some(root) = parse_json(body) else {
        return Vec::new();
    };
    let Some(arr) = root.get("models").and_then(|m| m.as_arr()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in arr {
        let slug = item.str_or("slug").trim().to_string();
        if slug.is_empty() {
            continue;
        }
        let visibility = item
            .get("visibility")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let supported_in_api = match item.get("supported_in_api") {
            Some(Json::Bool(b)) => Some(*b),
            _ => None,
        };
        let use_responses_lite = item.get("use_responses_lite") == Some(&Json::Bool(true));
        let support_verbosity = item.get("support_verbosity") == Some(&Json::Bool(true));
        let default_verbosity = item
            .get("default_verbosity")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string);
        let default_reasoning_level = item
            .get("default_reasoning_level")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string);

        let info = CodexModelInfo {
            slug,
            visibility,
            supported_in_api,
            use_responses_lite,
            support_verbosity,
            default_verbosity,
            default_reasoning_level,
        };
        if is_public_codex_model(&info) && !out.iter().any(|existing: &CodexModelInfo| existing.slug == info.slug) {
            out.push(info);
        }
    }
    out
}

/// Normalizes a multi-turn conversation into the exact JSON body expected by
/// `https://chatgpt.com/backend-api/codex/responses` (`normalizeCodexResponsesBodyInternal`).
/// Returns `(json_payload, use_responses_lite)`.
pub fn normalize_codex_responses_body(
    model: &str,
    instructions: &str,
    messages: &[ChatTurn],
    model_info: Option<&CodexModelInfo>,
) -> (String, bool) {
    let use_lite = model_info.is_some_and(|m| m.use_responses_lite);
    let instr = if instructions.trim().is_empty() {
        "You are a helpful assistant."
    } else {
        instructions.trim()
    };

    let mut input_items = Vec::with_capacity(messages.len() + 1);
    if use_lite && !instr.is_empty() {
        input_items.push(format!(
            r#"{{"role":"developer","content":[{{"type":"input_text","text":{}}}]}}"#,
            json_str(instr)
        ));
    }
    for turn in messages {
        match turn.role {
            Role::User => {
                input_items.push(format!(
                    r#"{{"role":"user","content":[{{"type":"input_text","text":{}}}]}}"#,
                    json_str(&turn.text)
                ));
            }
            Role::Assistant => {
                input_items.push(format!(
                    r#"{{"role":"assistant","content":[{{"type":"output_text","text":{}}}]}}"#,
                    json_str(&turn.text)
                ));
            }
        }
    }

    let top_instructions = if use_lite { "" } else { instr };
    let mut fields = vec![
        format!(r#""model":{}"#, json_str(model)),
        format!(r#""instructions":{}"#, json_str(top_instructions)),
        format!(r#""input":[{}]"#, input_items.join(",")),
        r#""store":false"#.to_string(),
        r#""stream":true"#.to_string(),
        r#""include":["reasoning.encrypted_content"]"#.to_string(),
    ];

    if let Some(info) = model_info {
        if info.default_reasoning_level.is_some() || info.use_responses_lite {
            let mut r_parts = Vec::new();
            if let Some(effort) = &info.default_reasoning_level {
                r_parts.push(format!(r#""effort":{}"#, json_str(effort)));
            }
            if info.use_responses_lite {
                r_parts.push(r#""context":"all_turns""#.to_string());
            }
            if !r_parts.is_empty() {
                fields.push(format!(r#""reasoning":{{{}}}"#, r_parts.join(",")));
            }
        }
        if info.support_verbosity {
            if let Some(verb) = &info.default_verbosity {
                fields.push(format!(r#""text":{{"verbosity":{}}}"#, json_str(verb)));
            }
        }
        if info.use_responses_lite {
            fields.push(r#""parallel_tool_calls":false"#.to_string());
        }
    }

    (format!("{{{}}}", fields.join(",")), use_lite)
}

/// Processes a single SSE `data:` JSON payload from `/codex/responses`.
/// Returns `Ok(true)` when `response.completed` is reached, `Ok(false)` while streaming,
/// or `Err(msg)` if the upstream returned an error event.
pub fn process_sse_data(data: &str, acc_text: &mut String) -> Result<bool, String> {
    let trimmed = data.trim();
    if trimmed.is_empty() || trimmed == "[DONE]" {
        return Ok(trimmed == "[DONE]");
    }
    let Some(parsed) = parse_json(trimmed) else {
        return Ok(false);
    };
    let ev_type = parsed.str_or("type");
    match ev_type.as_str() {
        "response.output_text.delta" => {
            let delta = parsed.str_or("delta");
            if !delta.is_empty() {
                acc_text.push_str(&delta);
            }
            Ok(false)
        }
        "response.output_text.done" => {
            if acc_text.is_empty() {
                let text = parsed.str_or("text");
                if !text.is_empty() {
                    acc_text.push_str(&text);
                }
            }
            Ok(false)
        }
        "response.completed" => {
            if acc_text.is_empty() {
                if let Some(resp) = parsed.get("response") {
                    if let Some(outputs) = resp.get("output").and_then(|o| o.as_arr()) {
                        for item in outputs {
                            if let Some(contents) = item.get("content").and_then(|c| c.as_arr()) {
                                for part in contents {
                                    let t = part.str_or("text");
                                    if !t.is_empty() {
                                        acc_text.push_str(&t);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Ok(true)
        }
        "response.failed" | "error" => {
            let msg = parsed
                .get("error")
                .map(|e| e.str_or("message"))
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    parsed
                        .get("response")
                        .and_then(|r| r.get("error"))
                        .map(|e| e.str_or("message"))
                        .filter(|s| !s.is_empty())
                })
                .unwrap_or_else(|| "Codex response stream failed".to_string());
            Err(msg)
        }
        _ => Ok(false),
    }
}

// ------------------------------------------------ Loopback OAuth Callback ----

fn extract_query_param(query: &str, key: &str) -> Option<String> {
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if k == key {
            return Some(url_decode(v));
        }
    }
    None
}

pub fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                if let (Ok(hi_s), Ok(lo_s)) = (
                    std::str::from_utf8(&bytes[i + 1..i + 3]),
                    u8::from_str_radix(
                        std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""),
                        16,
                    ),
                ) {
                    let _ = hi_s;
                    out.push(lo_s);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn write_http_response(stream: &mut std::net::TcpStream, status: &str, content_type: &str, body: &str) {
    let resp = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(resp.as_bytes());
    let _ = stream.flush();
}

fn wait_for_oauth_callback(
    expected_state: &str,
    auth_url: &str,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let v4 = SocketAddr::from(([127, 0, 0, 1], DEFAULT_LOGIN_PORT));
    let listener = TcpListener::bind(v4).map_err(|e| {
        format!("Could not bind localhost:{DEFAULT_LOGIN_PORT} for OAuth callback ({e})")
    })?;
    let _ = listener.set_nonblocking(true);

    crate::sys::open(auth_url, "");

    let deadline = Instant::now() + Duration::from_secs(300);
    while Instant::now() < deadline {
        if cancel.load(Ordering::Acquire) {
            return Err("Sign-in cancelled".into());
        }
        match listener.accept() {
            Ok((mut stream, _addr)) => {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                if n == 0 {
                    continue;
                }
                let req_str = String::from_utf8_lossy(&buf[..n]);
                let first_line = req_str.lines().next().unwrap_or("");
                let raw_target = first_line.split_whitespace().nth(1).unwrap_or("/");
                let (path, query) = raw_target.split_once('?').unwrap_or((raw_target, ""));
                if path != "/auth/callback" {
                    write_http_response(&mut stream, "404 Not Found", "text/plain; charset=utf-8", "Not found.");
                    continue;
                }
                if let Some(err) = extract_query_param(query, "error") {
                    let desc = extract_query_param(query, "error_description").unwrap_or(err);
                    write_http_response(
                        &mut stream,
                        "400 Bad Request",
                        "text/plain; charset=utf-8",
                        &format!("OpenAI OAuth error: {desc}"),
                    );
                    return Err(desc);
                }
                let code = extract_query_param(query, "code").unwrap_or_default();
                let state = extract_query_param(query, "state").unwrap_or_default();
                if code.is_empty() || state != expected_state {
                    write_http_response(
                        &mut stream,
                        "400 Bad Request",
                        "text/plain; charset=utf-8",
                        "Invalid OAuth callback state.",
                    );
                    return Err("Invalid OAuth callback state".into());
                }
                write_http_response(
                    &mut stream,
                    "200 OK",
                    "text/html; charset=utf-8",
                    CALLBACK_SUCCESS_HTML,
                );
                return Ok(code);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(format!("OAuth callback listener error: {e}")),
        }
    }
    Err("Timed out waiting for ChatGPT sign-in in browser".into())
}

// ------------------------------------------------ WinHTTP Client & Worker ----

#[cfg(windows)]
mod http {
    use std::ffi::c_void;
    use std::ptr::{null, null_mut};

    type HInternet = *mut c_void;

    const WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY: u32 = 4;
    const WINHTTP_ACCESS_TYPE_DEFAULT_PROXY: u32 = 0;
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
        ) -> HInternet;
        fn WinHttpSetTimeouts(
            h: HInternet,
            resolve_ms: i32,
            connect_ms: i32,
            send_ms: i32,
            recv_ms: i32,
        ) -> i32;
        fn WinHttpConnect(
            session: HInternet,
            server: *const u16,
            port: u16,
            reserved: u32,
        ) -> HInternet;
        fn WinHttpOpenRequest(
            connect: HInternet,
            verb: *const u16,
            object_name: *const u16,
            version: *const u16,
            referrer: *const u16,
            accept_types: *const *const u16,
            flags: u32,
        ) -> HInternet;
        fn WinHttpSendRequest(
            request: HInternet,
            headers: *const u16,
            headers_len: u32,
            optional: *const c_void,
            optional_len: u32,
            total_len: u32,
            context: usize,
        ) -> i32;
        fn WinHttpReceiveResponse(request: HInternet, reserved: *mut c_void) -> i32;
        fn WinHttpQueryHeaders(
            request: HInternet,
            info_level: u32,
            name: *const u16,
            buffer: *mut c_void,
            buffer_len: *mut u32,
            index: *mut u32,
        ) -> i32;
        fn WinHttpReadData(
            request: HInternet,
            buffer: *mut c_void,
            bytes_to_read: u32,
            bytes_read: *mut u32,
        ) -> i32;
        fn WinHttpCloseHandle(h: HInternet) -> i32;
    }

    struct HandleGuard(HInternet);
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
        session: HInternet,
    }

    impl Client {
        pub fn new() -> Self {
            let agent = wide("Vey-Codex/0.4");
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
                    WinHttpSetTimeouts(s, 8_000, 10_000, 15_000, 60_000);
                }
            }
            Client { session: s }
        }

        pub fn request(
            &self,
            method: &str,
            host: &str,
            port: u16,
            https: bool,
            path: &str,
            headers: &str,
            body: Option<&str>,
        ) -> Result<(u32, String), String> {
            let mut out = Vec::new();
            let status = self.request_stream(
                method,
                host,
                port,
                https,
                path,
                headers,
                body,
                |chunk| {
                    out.extend_from_slice(chunk);
                    true
                },
            )?;
            Ok((status, String::from_utf8_lossy(&out).into_owned()))
        }

        #[allow(clippy::too_many_arguments)]
        pub fn request_stream<F: FnMut(&[u8]) -> bool>(
            &self,
            method: &str,
            host: &str,
            port: u16,
            https: bool,
            path: &str,
            headers: &str,
            body: Option<&str>,
            mut on_chunk: F,
        ) -> Result<u32, String> {
            if self.session.is_null() {
                return Err("Could not open WinHTTP session".into());
            }
            let host_w = wide(host);
            let conn = HandleGuard(unsafe { WinHttpConnect(self.session, host_w.as_ptr(), port, 0) });
            if conn.0.is_null() {
                return Err(format!("Could not connect to {host}"));
            }
            let verb_w = wide(method);
            let path_w = wide(path);
            let flags = if https { WINHTTP_FLAG_SECURE } else { 0 };
            let req = HandleGuard(unsafe {
                WinHttpOpenRequest(conn.0, verb_w.as_ptr(), path_w.as_ptr(), null(), null(), null(), flags)
            });
            if req.0.is_null() {
                return Err("Could not create HTTP request".into());
            }
            let hdr_w = wide(headers);
            let body_bytes = body.map(|b| b.as_bytes()).unwrap_or(&[]);
            let body_ptr = if body_bytes.is_empty() {
                null()
            } else {
                body_bytes.as_ptr() as *const c_void
            };
            let len = body_bytes.len() as u32;
            let sent = unsafe {
                WinHttpSendRequest(req.0, hdr_w.as_ptr(), u32::MAX, body_ptr, len, len, 0)
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
            let mut buf = [0u8; 4096];
            loop {
                let mut n: u32 = 0;
                let ok = unsafe {
                    WinHttpReadData(req.0, buf.as_mut_ptr() as *mut c_void, buf.len() as u32, &mut n)
                };
                if ok == 0 || n == 0 {
                    break;
                }
                if !on_chunk(&buf[..n as usize]) {
                    break;
                }
            }
            Ok(status)
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
            let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_CODEX, WPARAM(0), LPARAM(0));
        }
    }
}

#[cfg(windows)]
pub fn pick_file_win32(owner_hwnd: isize) -> Option<PathBuf> {
    #[repr(C)]
    struct OpenFileNameW {
        l_struct_size: u32,
        hwnd_owner: isize,
        h_instance: isize,
        lpstr_filter: *const u16,
        lpstr_custom_filter: *mut u16,
        n_max_cust_filter: u32,
        n_filter_index: u32,
        lpstr_file: *mut u16,
        n_max_file: u32,
        lpstr_file_title: *mut u16,
        n_max_file_title: u32,
        lpstr_initial_dir: *const u16,
        lpstr_title: *const u16,
        flags: u32,
        n_file_offset: u16,
        n_file_extension: u16,
        lpstr_def_ext: *const u16,
        l_cust_data: isize,
        lpfn_hook: *const (),
        lp_template_name: *const u16,
        pv_reserved: *mut (),
        dw_reserved: u32,
        flags_ex: u32,
    }

    type GetOpenFileNameWFn = unsafe extern "system" fn(*mut OpenFileNameW) -> i32;

    unsafe {
        use windows::core::PCSTR;
        use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};

        let h_mod = match LoadLibraryA(PCSTR(b"comdlg32.dll\0".as_ptr())) {
            Ok(m) if !m.is_invalid() => m,
            _ => return None,
        };
        let proc = GetProcAddress(h_mod, PCSTR(b"GetOpenFileNameW\0".as_ptr()));
        let Some(proc) = proc else { return None };
        let get_file_fn: GetOpenFileNameWFn = std::mem::transmute(proc);

        let mut buf = vec![0u16; 1024];
        let filter: Vec<u16> = "All Files (*.*)\0*.*\0Text & Code (*.rs;*.txt;*.md;*.json;*.ts;*.py;*.toml;*.csv;*.log)\0*.rs;*.txt;*.md;*.json;*.ts;*.py;*.toml;*.csv;*.log\0\0"
            .encode_utf16()
            .collect();

        let mut ofn = OpenFileNameW {
            l_struct_size: std::mem::size_of::<OpenFileNameW>() as u32,
            hwnd_owner: owner_hwnd,
            h_instance: 0,
            lpstr_filter: filter.as_ptr(),
            lpstr_custom_filter: std::ptr::null_mut(),
            n_max_cust_filter: 0,
            n_filter_index: 1,
            lpstr_file: buf.as_mut_ptr(),
            n_max_file: buf.len() as u32,
            lpstr_file_title: std::ptr::null_mut(),
            n_max_file_title: 0,
            lpstr_initial_dir: std::ptr::null(),
            lpstr_title: std::ptr::null(),
            flags: 0x00080000 | 0x00001000 | 0x00000800, // OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST
            n_file_offset: 0,
            n_file_extension: 0,
            lpstr_def_ext: std::ptr::null(),
            l_cust_data: 0,
            lpfn_hook: std::ptr::null(),
            lp_template_name: std::ptr::null(),
            pv_reserved: std::ptr::null_mut(),
            dw_reserved: 0,
            flags_ex: 0,
        };

        if get_file_fn(&mut ofn) != 0 {
            let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            let path_str = String::from_utf16_lossy(&buf[..len]);
            if !path_str.trim().is_empty() {
                return Some(PathBuf::from(path_str));
            }
        }
        None
    }
}

#[cfg(not(windows))]
pub fn pick_file_win32(_owner_hwnd: isize) -> Option<PathBuf> {
    None
}

#[cfg(not(windows))]
fn worker(
    _hwnd: isize,
    _shared: Arc<Mutex<CodexData>>,
    _rx: Receiver<Req>,
    _cancel_login: Arc<AtomicBool>,
    _cancel_stream: Arc<AtomicBool>,
) {
}

#[cfg(windows)]
fn worker(
    hwnd: isize,
    shared: Arc<Mutex<CodexData>>,
    rx: Receiver<Req>,
    cancel_login: Arc<AtomicBool>,
    cancel_stream: Arc<AtomicBool>,
) {
    let client = http::Client::new();
    while let Ok(req) = rx.recv() {
        match req {
            Req::ReloadAuth => do_reload_auth(&client, hwnd, &shared),
            Req::StartLogin => do_login(&client, hwnd, &shared, &cancel_login),
            Req::Logout => do_logout(hwnd, &shared),
            Req::RefreshModels => do_refresh_models(&client, hwnd, &shared),
            Req::AttachFile(att) => {
                let mut d = shared.lock().unwrap();
                d.attachment = Some(att);
                drop(d);
                post_ui(hwnd);
            }
            Req::SendChat { model, history } => {
                do_send_chat(&client, hwnd, &shared, &cancel_stream, &model, &history);
            }
        }
    }
}

#[cfg(windows)]
fn exchange_oauth_code(client: &http::Client, code: &str, verifier: &str) -> Result<CodexTokens, String> {
    let form = format!(
        "grant_type=authorization_code&code={}&redirect_uri={}&client_id={}&code_verifier={}",
        url_encode(code),
        url_encode(REDIRECT_URI),
        url_encode(DEFAULT_OPENAI_OAUTH_CLIENT_ID),
        url_encode(verifier),
    );
    let headers = "Content-Type: application/x-www-form-urlencoded\r\nAccept: application/json\r\n";
    let (status, body) = client.request(
        "POST",
        "auth.openai.com",
        443,
        true,
        "/oauth/token",
        headers,
        Some(&form),
    )?;
    if !(200..=299).contains(&status) {
        return Err(format!("OAuth token exchange failed (HTTP {status})"));
    }
    parse_token_endpoint_response(&body, None)
}

#[cfg(windows)]
fn refresh_oauth_tokens(client: &http::Client, current: &CodexTokens) -> Result<CodexTokens, String> {
    if current.refresh_token.trim().is_empty() {
        return Err("Missing refresh_token; please sign in again".into());
    }
    let payload = format!(
        r#"{{"grant_type":"refresh_token","refresh_token":{},"client_id":{}}}"#,
        json_str(&current.refresh_token),
        json_str(DEFAULT_OPENAI_OAUTH_CLIENT_ID),
    );
    let headers = "Content-Type: application/json\r\nAccept: application/json\r\n";
    let (status, body) = client.request(
        "POST",
        "auth.openai.com",
        443,
        true,
        "/oauth/token",
        headers,
        Some(&payload),
    )?;
    if !(200..=299).contains(&status) {
        return Err(format!("Token refresh failed (HTTP {status})"));
    }
    parse_token_endpoint_response(&body, Some(current))
}

fn parse_token_endpoint_response(body: &str, prev: Option<&CodexTokens>) -> Result<CodexTokens, String> {
    let root = parse_json(body).ok_or_else(|| "Invalid OAuth token response JSON".to_string())?;
    let access_token = root.str_or("access_token");
    if access_token.trim().is_empty() {
        return Err("OAuth response did not include access_token".into());
    }
    let refresh_token = {
        let r = root.str_or("refresh_token");
        if !r.trim().is_empty() {
            r
        } else {
            prev.map(|p| p.refresh_token.clone()).unwrap_or_default()
        }
    };
    let id_token = {
        let id = root.str_or("id_token");
        if !id.trim().is_empty() {
            id
        } else {
            prev.map(|p| p.id_token.clone()).unwrap_or_default()
        }
    };

    let id_claims = if !id_token.is_empty() {
        parse_jwt_claims(&id_token)
    } else {
        None
    };
    let access_claims = parse_jwt_claims(&access_token);

    let account_id = derive_account_id(id_claims.as_ref())
        .or_else(|| derive_account_id(access_claims.as_ref()))
        .or_else(|| prev.map(|p| p.account_id.clone()))
        .unwrap_or_default();

    if account_id.trim().is_empty() {
        return Err("Could not derive ChatGPT account_id from OAuth tokens".into());
    }

    let is_fedramp = derive_chatgpt_account_is_fedramp(id_claims.as_ref())
        || derive_chatgpt_account_is_fedramp(access_claims.as_ref())
        || prev.is_some_and(|p| p.is_fedramp);
    let email = derive_user_email(id_claims.as_ref())
        .or_else(|| derive_user_email(access_claims.as_ref()))
        .or_else(|| prev.map(|p| p.email.clone()))
        .unwrap_or_default();
    let plan_type = derive_plan_type(id_claims.as_ref())
        .or_else(|| derive_plan_type(access_claims.as_ref()))
        .or_else(|| prev.map(|p| p.plan_type.clone()))
        .unwrap_or_default();

    Ok(CodexTokens {
        access_token,
        refresh_token,
        id_token,
        account_id,
        is_fedramp,
        email,
        plan_type,
        last_refresh_epoch_s: Some(now_epoch_secs()),
    })
}

#[cfg(windows)]
fn ensure_valid_tokens(client: &http::Client) -> Result<(PathBuf, CodexTokens), String> {
    let path = resolve_codex_auth_file_path(None);
    let mut tokens = load_codex_tokens(&path)?;
    if should_refresh_access_token(&tokens.access_token, tokens.last_refresh_epoch_s, now_epoch_secs())
        && !tokens.refresh_token.trim().is_empty()
    {
        if let Ok(refreshed) = refresh_oauth_tokens(client, &tokens) {
            let _ = save_codex_tokens(&path, &refreshed);
            tokens = refreshed;
        }
    }
    Ok((path, tokens))
}

#[cfg(windows)]
fn codex_headers(tokens: &CodexTokens, accept: &str, content_type: Option<&str>, responses_lite: bool) -> String {
    let mut h = format!(
        "Accept: {accept}\r\nAuthorization: Bearer {}\r\nchatgpt-account-id: {}\r\n",
        tokens.access_token, tokens.account_id
    );
    if tokens.is_fedramp {
        h.push_str("X-OpenAI-Fedramp: true\r\n");
    }
    if let Some(ct) = content_type {
        h.push_str(&format!("Content-Type: {ct}\r\n"));
    }
    if responses_lite {
        h.push_str("x-openai-internal-codex-responses-lite: true\r\n");
    }
    h
}

#[cfg(windows)]
fn do_reload_auth(client: &http::Client, hwnd: isize, shared: &Arc<Mutex<CodexData>>) {
    let path = resolve_codex_auth_file_path(None);
    match ensure_valid_tokens(client) {
        Ok((_, tokens)) => {
            {
                let mut d = shared.lock().unwrap();
                d.signed_in = true;
                d.signing_in = false;
                d.account_label = tokens.display_label();
                d.account_id = tokens.account_id.clone();
                d.auth_path = path.to_string_lossy().into_owned();
                d.error = None;
                d.toast = Some(("Loaded ~/.codex/auth.json".into(), true));
            }
            post_ui(hwnd);
            do_refresh_models(client, hwnd, shared);
        }
        Err(e) => {
            let mut d = shared.lock().unwrap();
            d.signed_in = false;
            d.signing_in = false;
            d.auth_path = path.to_string_lossy().into_owned();
            d.error = Some(e);
            drop(d);
            post_ui(hwnd);
        }
    }
}

#[cfg(windows)]
fn do_login(
    client: &http::Client,
    hwnd: isize,
    shared: &Arc<Mutex<CodexData>>,
    cancel_login: &Arc<AtomicBool>,
) {
    cancel_login.store(false, Ordering::Release);
    {
        let mut d = shared.lock().unwrap();
        d.signing_in = true;
        d.error = None;
    }
    post_ui(hwnd);

    let state = create_oauth_state();
    let verifier = create_code_verifier();
    let challenge = create_code_challenge(&verifier);
    let auth_url = build_openai_oauth_authorization_url(
        REDIRECT_URI,
        &state,
        &challenge,
        DEFAULT_OPENAI_OAUTH_ISSUER,
        DEFAULT_OPENAI_OAUTH_CLIENT_ID,
        DEFAULT_OPENAI_OAUTH_SCOPE,
    );

    let code = match wait_for_oauth_callback(&state, &auth_url, cancel_login) {
        Ok(c) => c,
        Err(e) => {
            let mut d = shared.lock().unwrap();
            d.signing_in = false;
            if e != "Sign-in cancelled" {
                d.error = Some(e);
            }
            drop(d);
            post_ui(hwnd);
            return;
        }
    };

    match exchange_oauth_code(client, &code, &verifier) {
        Ok(tokens) => {
            let path = resolve_codex_auth_file_path(None);
            let _ = save_codex_tokens(&path, &tokens);
            {
                let mut d = shared.lock().unwrap();
                d.signed_in = true;
                d.signing_in = false;
                d.account_label = tokens.display_label();
                d.account_id = tokens.account_id.clone();
                d.auth_path = path.to_string_lossy().into_owned();
                d.error = None;
                d.toast = Some(("Signed in with ChatGPT".into(), true));
            }
            post_ui(hwnd);
            do_refresh_models(client, hwnd, shared);
        }
        Err(e) => {
            let mut d = shared.lock().unwrap();
            d.signing_in = false;
            d.error = Some(e);
            drop(d);
            post_ui(hwnd);
        }
    }
}

#[cfg(windows)]
fn do_logout(hwnd: isize, shared: &Arc<Mutex<CodexData>>) {
    let path = resolve_codex_auth_file_path(None);
    let _ = std::fs::remove_file(&path);
    let mut d = shared.lock().unwrap();
    d.signed_in = false;
    d.signing_in = false;
    d.account_label.clear();
    d.account_id.clear();
    d.streaming = false;
    d.error = None;
    d.toast = Some(("Signed out of ChatGPT".into(), true));
    drop(d);
    post_ui(hwnd);
}

#[cfg(windows)]
fn do_refresh_models(client: &http::Client, hwnd: isize, shared: &Arc<Mutex<CodexData>>) {
    let Ok((path, mut tokens)) = ensure_valid_tokens(client) else {
        return;
    };
    {
        let mut d = shared.lock().unwrap();
        d.loading_models = true;
        d.signed_in = true;
        d.account_label = tokens.display_label();
        d.account_id = tokens.account_id.clone();
    }
    post_ui(hwnd);

    let req_path = format!(
        "/backend-api/codex/models?client_version={}",
        url_encode(DEFAULT_CODEX_CLIENT_VERSION)
    );
    let headers = codex_headers(&tokens, "application/json", None, false);
    let mut res = client.request("GET", "chatgpt.com", 443, true, &req_path, &headers, None);

    if let Ok((401, _)) = res {
        if let Ok(refreshed) = refresh_oauth_tokens(client, &tokens) {
            let _ = save_codex_tokens(&path, &refreshed);
            tokens = refreshed;
            let retry_hdr = codex_headers(&tokens, "application/json", None, false);
            res = client.request("GET", "chatgpt.com", 443, true, &req_path, &retry_hdr, None);
        }
    }

    let mut d = shared.lock().unwrap();
    d.loading_models = false;
    d.account_label = tokens.display_label();
    if let Ok((200..=299, body)) = res {
        let catalog = parse_models_catalog(&body);
        if !catalog.is_empty() {
            if !catalog.iter().any(|m| m.slug == d.selected_model) {
                if catalog.iter().any(|m| m.slug == DEFAULT_MODEL) {
                    d.selected_model = DEFAULT_MODEL.to_string();
                } else if let Some(first) = catalog.first() {
                    d.selected_model = first.slug.clone();
                }
            }
            d.models = catalog;
        }
    }
    drop(d);
    post_ui(hwnd);
}

#[cfg(windows)]
fn do_send_chat(
    client: &http::Client,
    hwnd: isize,
    shared: &Arc<Mutex<CodexData>>,
    cancel_stream: &Arc<AtomicBool>,
    model: &str,
    history: &[ChatTurn],
) {
    cancel_stream.store(false, Ordering::Release);
    let (path, mut tokens) = match ensure_valid_tokens(client) {
        Ok(pair) => pair,
        Err(e) => {
            let mut d = shared.lock().unwrap();
            d.streaming = false;
            d.error = Some(e);
            drop(d);
            post_ui(hwnd);
            return;
        }
    };

    let model_info = {
        let d = shared.lock().unwrap();
        d.models.iter().find(|m| m.slug == model).cloned()
    };

    let (payload, use_lite) = normalize_codex_responses_body(
        model,
        "You are a helpful, concise coding and productivity assistant inside the Vey desktop notch.",
        history,
        model_info.as_ref(),
    );

    let (_post_stream, post_assistant_entry) = {
        let mut d = shared.lock().unwrap();
        d.streaming = true;
        d.error = None;
        d.messages = history.to_vec();
        d.messages.push(ChatMessage {
            role: Role::Assistant,
            text: String::new(),
        });
        (d.streaming, true)
    };
    if post_assistant_entry {
        post_ui(hwnd);
    }

    for attempt in 0..2 {
        let headers = codex_headers(
            &tokens,
            "text/event-stream",
            Some("application/json; charset=utf-8"),
            use_lite,
        );

        let mut sse_buf = String::new();
        let mut raw_err_buf = Vec::new();
        let mut acc_text = String::new();
        let mut stream_err: Option<String> = None;

        let status_res = client.request_stream(
            "POST",
            "chatgpt.com",
            443,
            true,
            "/backend-api/codex/responses",
            &headers,
            Some(&payload),
            |chunk| {
                if cancel_stream.load(Ordering::Acquire) {
                    return false;
                }
                if raw_err_buf.len() < 4096 {
                    let take = (4096 - raw_err_buf.len()).min(chunk.len());
                    raw_err_buf.extend_from_slice(&chunk[..take]);
                }
                sse_buf.push_str(&String::from_utf8_lossy(chunk));
                while let Some(nl) = sse_buf.find('\n') {
                    let line = sse_buf[..nl].trim_end_matches('\r').to_string();
                    sse_buf.drain(..=nl);
                    if let Some(data_part) = line.strip_prefix("data:") {
                        let prev_len = acc_text.len();
                        match process_sse_data(data_part, &mut acc_text) {
                            Ok(_done) => {
                                if acc_text.len() != prev_len {
                                    let mut d = shared.lock().unwrap();
                                    if let Some(last) = d.messages.last_mut() {
                                        if last.role == Role::Assistant {
                                            last.text = acc_text.clone();
                                        }
                                    }
                                    drop(d);
                                    post_ui(hwnd);
                                }
                            }
                            Err(e) => {
                                stream_err = Some(e);
                                return false;
                            }
                        }
                    }
                }
                true
            },
        );

        match status_res {
            Ok(401) if attempt == 0 && !tokens.refresh_token.trim().is_empty() => {
                if let Ok(refreshed) = refresh_oauth_tokens(client, &tokens) {
                    let _ = save_codex_tokens(&path, &refreshed);
                    tokens = refreshed;
                    continue;
                }
            }
            Ok(200..=299) => {
                // Process any trailing line in sse_buf
                for line in sse_buf.lines() {
                    if let Some(data_part) = line.trim_end_matches('\r').strip_prefix("data:") {
                        let _ = process_sse_data(data_part, &mut acc_text);
                    }
                }
                let mut d = shared.lock().unwrap();
                d.streaming = false;
                if let Some(err) = stream_err {
                    d.error = Some(err);
                } else {
                    d.error = None;
                    if let Some(last) = d.messages.last_mut() {
                        if last.role == Role::Assistant {
                            if !acc_text.is_empty() {
                                last.text = acc_text;
                            } else if last.text.is_empty() && cancel_stream.load(Ordering::Acquire) {
                                last.text = "(Stopped)".into();
                            }
                        }
                    }
                }
                drop(d);
                post_ui(hwnd);
                return;
            }
            Ok(code) => {
                let raw_text = String::from_utf8_lossy(&raw_err_buf);
                let msg = parse_json(&raw_text)
                    .and_then(|j| {
                        let d = j.str_or("detail");
                        if !d.is_empty() {
                            return Some(d);
                        }
                        j.get("error").map(|e| e.str_or("message")).filter(|s| !s.is_empty())
                    })
                    .unwrap_or_else(|| format!("Codex HTTP {code}"));
                let mut d = shared.lock().unwrap();
                d.streaming = false;
                d.error = Some(msg);
                if let Some(last) = d.messages.last() {
                    if last.role == Role::Assistant && last.text.is_empty() {
                        d.messages.pop();
                    }
                }
                drop(d);
                post_ui(hwnd);
                return;
            }
            Err(e) => {
                let mut d = shared.lock().unwrap();
                d.streaming = false;
                d.error = Some(e);
                if let Some(last) = d.messages.last() {
                    if last.role == Role::Assistant && last.text.is_empty() {
                        d.messages.pop();
                    }
                }
                drop(d);
                post_ui(hwnd);
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_and_rfc7636_pkce_vectors() {
        // Empty string SHA-256
        let empty = sha256(b"");
        assert_eq!(
            empty[..4],
            [0xe3, 0xb0, 0xc4, 0x42]
        );
        // RFC 7636 Appendix B PKCE test vector
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = create_code_challenge(verifier);
        assert_eq!(challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn parses_codex_auth_json_and_jwt_claims() {
        let payload = r#"{"email":"hervey@livro.systems","exp":4102444800,"https://api.openai.com/auth":{"chatgpt_account_id":"acct_12345","chatgpt_account_is_fedramp":true,"chatgpt_plan_type":"plus"}}"#;
        let fake_jwt = format!("eyJhbGciOiJSUzI1NiJ9.{}.sig", base64url_encode(payload.as_bytes()));
        let auth_json = format!(
            r#"{{"auth_mode":"chatgpt","tokens":{{"id_token":"{jwt}","access_token":"{jwt}","refresh_token":"rt_abc"}},"last_refresh":1760000000}}"#,
            jwt = fake_jwt
        );
        let tok = parse_codex_auth_json(&auth_json).expect("valid auth.json");
        assert_eq!(tok.account_id, "acct_12345");
        assert!(tok.is_fedramp);
        assert_eq!(tok.email, "hervey@livro.systems");
        assert_eq!(tok.plan_type, "plus");
        assert_eq!(tok.display_label(), "hervey@livro.systems \u{00B7} plus");
        assert!(!should_refresh_access_token(&tok.access_token, Some(1760000000), 1760000500));
        // Should refresh when last_refresh is older than 55 minutes
        assert!(should_refresh_access_token(&tok.access_token, Some(1760000000), 1760004000));
    }

    #[test]
    fn normalizes_codex_responses_body_and_lite_mode() {
        let turns = vec![
            ChatTurn { role: Role::User, text: "Hello".into() },
            ChatTurn { role: Role::Assistant, text: "Hi there!".into() },
            ChatTurn { role: Role::User, text: "Write a test".into() },
        ];
        let info = CodexModelInfo {
            slug: "gpt-5.4-mini".into(),
            visibility: Some("list".into()),
            supported_in_api: Some(true),
            use_responses_lite: true,
            support_verbosity: true,
            default_verbosity: Some("medium".into()),
            default_reasoning_level: Some("high".into()),
        };
        let (body, lite) = normalize_codex_responses_body("gpt-5.4-mini", "Be concise.", &turns, Some(&info));
        assert!(lite);
        let parsed = parse_json(&body).expect("valid normalized JSON");
        assert_eq!(parsed.str_or("model"), "gpt-5.4-mini");
        assert_eq!(parsed.str_or("instructions"), "");
        let input = parsed.get("input").and_then(|i| i.as_arr()).unwrap();
        assert_eq!(input.len(), 4); // developer instructions + 3 turns
        assert_eq!(input[0].str_or("role"), "developer");
        assert_eq!(input[1].str_or("role"), "user");
        assert_eq!(input[2].str_or("role"), "assistant");
        assert_eq!(parsed.get("parallel_tool_calls"), Some(&Json::Bool(false)));
    }

    #[test]
    fn parses_sse_deltas_and_completion() {
        let mut acc = String::new();
        assert_eq!(
            process_sse_data(r#"{"type":"response.output_text.delta","delta":"Hello "}"#, &mut acc),
            Ok(false)
        );
        assert_eq!(
            process_sse_data(r#"{"type":"response.output_text.delta","delta":"world!"}"#, &mut acc),
            Ok(false)
        );
        assert_eq!(
            process_sse_data(r#"{"type":"response.completed","response":{"id":"resp_1"}}"#, &mut acc),
            Ok(true)
        );
        assert_eq!(acc, "Hello world!");
    }
}
