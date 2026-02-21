use napi_derive::napi;

/// Options for creating a new native window.
///
/// Security: When loading untrusted content, use the `csp` field to restrict
/// what the page can do. Without a CSP, loaded content can execute inline
/// scripts and load resources from any origin.
#[napi(object)]
#[derive(Debug, Clone)]
pub struct WindowOptions {
    /// Window title. Default: ""
    pub title: Option<String>,
    /// Inner width in logical pixels. Default: 800
    pub width: Option<f64>,
    /// Inner height in logical pixels. Default: 600
    pub height: Option<f64>,
    /// X position in screen coordinates
    pub x: Option<f64>,
    /// Y position in screen coordinates
    pub y: Option<f64>,
    /// Minimum inner width
    pub min_width: Option<f64>,
    /// Minimum inner height
    pub min_height: Option<f64>,
    /// Maximum inner width
    pub max_width: Option<f64>,
    /// Maximum inner height
    pub max_height: Option<f64>,
    /// Allow resizing. Default: true
    pub resizable: Option<bool>,
    /// Show window decorations (title bar, borders). Default: true
    pub decorations: Option<bool>,
    /// Transparent window background. Default: false
    pub transparent: Option<bool>,
    /// Always on top of other windows. Default: false
    pub always_on_top: Option<bool>,
    /// Initially visible. Default: true
    pub visible: Option<bool>,
    /// Enable devtools. Default: false
    pub devtools: Option<bool>,
    /// Content Security Policy to inject via a `<meta>` tag at document start.
    /// When set, a `<meta http-equiv="Content-Security-Policy" content="...">` tag
    /// is injected before any page scripts run. This restricts what the loaded
    /// content can do (e.g. block inline scripts, limit resource origins).
    ///
    /// Example: `"default-src 'self'; script-src 'self' 'unsafe-inline'"`
    pub csp: Option<String>,
    /// Trusted origins for IPC messages at the native layer.
    /// When set, only messages whose source URL origin matches one of these
    /// entries are forwarded to the host. Messages from other origins are
    /// silently dropped. Each entry should be a full origin string, e.g.
    /// `"https://example.com"` (scheme + host + optional port, no trailing slash).
    ///
    /// This is a defense-in-depth mechanism. For application-level origin
    /// filtering, use the `trustedOrigins` option in `createChannel()`.
    pub trusted_origins: Option<Vec<String>>,
    /// Allowed hosts for navigation restriction.
    /// When set and non-empty, ALL navigations (programmatic and user-initiated)
    /// are restricted to URLs whose host matches one of these patterns.
    /// Supports wildcard prefixes: `"*.example.com"` matches any subdomain of
    /// example.com (and example.com itself). When unset or empty, all hosts
    /// are allowed.
    ///
    /// Internal navigations (`about:blank`, `https://native-window.local/`) are
    /// always permitted regardless of this setting.
    pub allowed_hosts: Option<Vec<String>>,
}

impl Default for WindowOptions {
    fn default() -> Self {
        Self {
            title: None,
            width: None,
            height: None,
            x: None,
            y: None,
            min_width: None,
            min_height: None,
            max_width: None,
            max_height: None,
            resizable: None,
            decorations: None,
            transparent: None,
            always_on_top: None,
            visible: None,
            devtools: None,
            csp: None,
            trusted_origins: None,
            allowed_hosts: None,
        }
    }
}
