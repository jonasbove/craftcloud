//! Plain HTTP values. `core` builds requests and reads responses; it never sends anything. The
//! desktop client sends them with `ureq`, the web frontend with the browser's `fetch`.

/// The methods the CraftCloud client uses (WebDAV adds PROPFIND, MKCOL, COPY, MOVE).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Delete,
    Propfind,
    Mkcol,
    Copy,
    Move,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Delete => "DELETE",
            Self::Propfind => "PROPFIND",
            Self::Mkcol => "MKCOL",
            Self::Copy => "COPY",
            Self::Move => "MOVE",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: Method,
    /// Absolute URL.
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpRequest {
    pub fn new(method: Method, url: impl Into<String>) -> Self {
        Self { method, url: url.into(), headers: Vec::new(), body: Vec::new() }
    }

    pub fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push((name.to_owned(), value.into()));
        self
    }

    pub fn json(self, value: &serde_json::Value) -> Self {
        let mut req = self.header("Content-Type", "application/json");
        req.body = value.to_string().into_bytes();
        req
    }

    pub fn body(mut self, body: Vec<u8>) -> Self {
        self.body = body;
        self
    }

    /// Whether a browser must send `X-CSRF-Token` with this request: OxiCloud checks it on every
    /// state-changing request a cookie session makes (its `csrf` middleware).
    pub fn needs_csrf(&self) -> bool {
        !matches!(self.method, Method::Get | Method::Propfind)
    }

    /// The value of a header (case-insensitive name), if set.
    pub fn header_value(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn new(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self { status, headers: Vec::new(), body: body.into() }
    }

    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}
