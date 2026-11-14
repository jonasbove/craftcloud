//! The address of a CraftCloud server, as a person types it into the desktop client.

use std::fmt;

use url::Url;

/// A CraftCloud server's base URL: `http` or `https`, a host, an optional port and path, no query,
/// no fragment, no user info, and no trailing slash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerUrl(Url);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerUrlError {
    Empty,
    /// The text could not be read as a URL.
    Invalid,
    /// A scheme other than http or https, e.g. `ftp://`.
    Scheme(String),
    /// `user:password@` in the address; sign-in happens in the browser instead.
    Credentials,
    /// `?query` or `#fragment`, which never belong in a server address.
    Extra,
}

impl fmt::Display for ServerUrlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("Enter the address of your CraftCloud server."),
            Self::Invalid => f.write_str("That doesn't look like a server address."),
            Self::Scheme(s) => write!(f, "Use an http:// or https:// address, not {s}://."),
            Self::Credentials => f.write_str("Leave out the user name and password; you sign in next."),
            Self::Extra => f.write_str("Leave out anything after ? or #."),
        }
    }
}

impl std::error::Error for ServerUrlError {}

impl ServerUrl {
    /// Reads what a person typed. Without a scheme, `https://` is assumed.
    pub fn parse(input: &str) -> Result<Self, ServerUrlError> {
        let text = input.trim();
        if text.is_empty() {
            return Err(ServerUrlError::Empty);
        }
        let with_scheme = if text.contains("://") { text.to_owned() } else { format!("https://{text}") };
        let mut url = Url::parse(&with_scheme).map_err(|_| ServerUrlError::Invalid)?;
        match url.scheme() {
            "http" | "https" => {}
            other => return Err(ServerUrlError::Scheme(other.to_owned())),
        }
        if url.host_str().is_none_or(str::is_empty) {
            return Err(ServerUrlError::Invalid);
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(ServerUrlError::Credentials);
        }
        if url.query().is_some() || url.fragment().is_some() {
            return Err(ServerUrlError::Extra);
        }
        let path = url.path().trim_end_matches('/').to_owned();
        url.set_path(&path);
        Ok(Self(url))
    }

    /// Whether traffic to this server is unencrypted (worth a warning outside a private network).
    pub fn is_plain_http(&self) -> bool {
        self.0.scheme() == "http"
    }

    /// The URL of an endpoint below the server's base, e.g. `join("craftcloud/config.json")`.
    pub fn join(&self, path: &str) -> String {
        format!("{}/{}", self.as_str(), path.trim_start_matches('/'))
    }

    /// The base URL without a trailing slash.
    pub fn as_str(&self) -> &str {
        self.0.as_str().trim_end_matches('/')
    }
}

impl fmt::Display for ServerUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::{ServerUrl, ServerUrlError};

    fn ok(input: &str) -> String {
        ServerUrl::parse(input).unwrap().to_string()
    }

    #[test]
    fn assumes_https_and_drops_trailing_slashes() {
        assert_eq!(ok("cloud.example.org"), "https://cloud.example.org");
        assert_eq!(ok("  https://cloud.example.org/  "), "https://cloud.example.org");
        assert_eq!(ok("cloud.example.org/craftcloud//"), "https://cloud.example.org/craftcloud");
    }

    #[test]
    fn keeps_http_ports_and_paths() {
        let url = ServerUrl::parse("http://cloud.example.org:8090").unwrap();
        assert_eq!(url.as_str(), "http://cloud.example.org:8090");
        assert!(url.is_plain_http());
        assert_eq!(url.join("/craftcloud/config.json"), "http://cloud.example.org:8090/craftcloud/config.json");
    }

    #[test]
    fn rejects_what_is_not_a_server_address() {
        assert_eq!(ServerUrl::parse("   "), Err(ServerUrlError::Empty));
        assert_eq!(ServerUrl::parse("ftp://cloud.example.org"), Err(ServerUrlError::Scheme("ftp".into())));
        assert_eq!(ServerUrl::parse("https://mira:secret@cloud.example.org"), Err(ServerUrlError::Credentials));
        assert_eq!(ServerUrl::parse("https://cloud.example.org/?a=b"), Err(ServerUrlError::Extra));
        assert_eq!(ServerUrl::parse("https://"), Err(ServerUrlError::Invalid));
    }
}
