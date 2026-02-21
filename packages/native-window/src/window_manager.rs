use std::collections::HashMap;

use crate::events::WindowEventHandlers;
use crate::options::WindowOptions;

// ── Permission flags ───────────────────────────────────────────

/// Per-window permission flags for platform callbacks.
/// All fields default to `false` (deny).
#[derive(Debug, Clone, Copy)]
pub struct PermissionFlags {
    pub allow_camera: bool,
    pub allow_microphone: bool,
    #[allow(dead_code)] // Only read in Windows platform code
    pub allow_file_system: bool,
}

impl Default for PermissionFlags {
    fn default() -> Self {
        Self {
            allow_camera: false,
            allow_microphone: false,
            allow_file_system: false,
        }
    }
}

/// Read the permission flags for a window. Returns default (deny-all) if not found.
pub fn get_permissions(window_id: u32) -> PermissionFlags {
    PERMISSIONS_MAP.with(|p| {
        p.borrow()
            .get(&window_id)
            .copied()
            .unwrap_or_default()
    })
}

/// Commands that can be sent to the window manager for execution during pump.
pub enum Command {
    CreateWindow {
        id: u32,
        options: WindowOptions,
    },
    LoadURL {
        id: u32,
        url: String,
    },
    LoadHTML {
        id: u32,
        html: String,
    },
    EvaluateJS {
        id: u32,
        script: String,
    },
    SetTitle {
        id: u32,
        title: String,
    },
    SetSize {
        id: u32,
        width: f64,
        height: f64,
    },
    #[allow(dead_code)]
    SetMinSize {
        id: u32,
        width: f64,
        height: f64,
    },
    #[allow(dead_code)]
    SetMaxSize {
        id: u32,
        width: f64,
        height: f64,
    },
    SetPosition {
        id: u32,
        x: f64,
        y: f64,
    },
    #[allow(dead_code)]
    SetResizable {
        id: u32,
        resizable: bool,
    },
    #[allow(dead_code)]
    SetDecorations {
        id: u32,
        decorations: bool,
    },
    SetAlwaysOnTop {
        id: u32,
        always_on_top: bool,
    },
    Show {
        id: u32,
    },
    Hide {
        id: u32,
    },
    Close {
        id: u32,
    },
    Focus {
        id: u32,
    },
    Maximize {
        id: u32,
    },
    Minimize {
        id: u32,
    },
    Unmaximize {
        id: u32,
    },
    Reload {
        id: u32,
    },
    GetCookies {
        id: u32,
        url: Option<String>,
    },
}

/// Global window manager state. Lives in thread_local storage.
pub struct WindowManager {
    pub next_id: u32,
    pub command_queue: Vec<Command>,
    pub event_handlers: HashMap<u32, WindowEventHandlers>,
    pub initialized: bool,
    #[cfg(target_os = "macos")]
    pub platform: Option<super::platform::macos::MacOSPlatform>,
    #[cfg(target_os = "windows")]
    pub platform: Option<super::platform::windows::WindowsPlatform>,
}

/// Maximum number of commands in the queue before logging a warning.
/// Commands are still accepted to avoid silently dropping operations.
const MAX_COMMAND_QUEUE: usize = 10_000;

impl WindowManager {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            command_queue: Vec::new(),
            event_handlers: HashMap::new(),
            initialized: false,
            platform: None,
        }
    }

    pub fn allocate_id(&mut self) -> napi::Result<u32> {
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).ok_or_else(|| {
            napi::Error::from_reason("Window ID space exhausted (u32 overflow)")
        })?;
        self.event_handlers.insert(id, WindowEventHandlers::new());
        Ok(id)
    }

    pub fn push_command(&mut self, cmd: Command) {
        if self.command_queue.len() >= MAX_COMMAND_QUEUE {
            eprintln!(
                "[native-window] Warning: command queue has {} entries (limit: {}). \
                 Possible runaway loop or missing pumpEvents() call.",
                self.command_queue.len(),
                MAX_COMMAND_QUEUE
            );
        }
        self.command_queue.push(cmd);
    }

    pub fn drain_commands(&mut self) -> Vec<Command> {
        std::mem::take(&mut self.command_queue)
    }

    /// Remove event handlers and security config for a closed window to prevent memory leaks.
    #[allow(dead_code)] // Used by macOS platform, not by Windows
    pub fn remove_event_handlers(&mut self, id: u32) {
        self.event_handlers.remove(&id);
        TRUSTED_ORIGINS_MAP.with(|o| {
            o.borrow_mut().remove(&id);
        });
        ALLOWED_HOSTS_MAP.with(|h| {
            h.borrow_mut().remove(&id);
        });
        PERMISSIONS_MAP.with(|p| {
            p.borrow_mut().remove(&id);
        });
    }
}

thread_local! {
    pub static MANAGER: std::cell::RefCell<WindowManager> = std::cell::RefCell::new(WindowManager::new());
    /// Per-window trusted origins for IPC message filtering.
    /// Stored outside MANAGER so the macOS NavigationDelegate / IPC handler
    /// can read them while MANAGER is mutably borrowed by pump_events.
    pub static TRUSTED_ORIGINS_MAP: std::cell::RefCell<HashMap<u32, Vec<String>>> = std::cell::RefCell::new(HashMap::new());
    /// Per-window allowed hosts for navigation restriction.
    /// Stored outside MANAGER so the macOS NavigationDelegate can read them
    /// while MANAGER is mutably borrowed by pump_events.
    pub static ALLOWED_HOSTS_MAP: std::cell::RefCell<HashMap<u32, Vec<String>>> = std::cell::RefCell::new(HashMap::new());
    /// Per-window permission flags for platform callbacks.
    /// Stored outside MANAGER so macOS WKUIDelegate / Windows PermissionRequested
    /// handlers can read them while MANAGER is mutably borrowed by pump_events.
    pub static PERMISSIONS_MAP: std::cell::RefCell<HashMap<u32, PermissionFlags>> = std::cell::RefCell::new(HashMap::new());
    /// Buffer for IPC messages that arrive while MANAGER is already borrowed (reentrant calls).
    /// Each entry: (window_id, message, source_url).
    pub static PENDING_MESSAGES: std::cell::RefCell<Vec<(u32, String, String)>> = std::cell::RefCell::new(Vec::new());
    /// Buffer for window close events that arrive while MANAGER is already borrowed.
    pub static PENDING_CLOSES: std::cell::RefCell<Vec<u32>> = std::cell::RefCell::new(Vec::new());
    /// Buffer for reload events triggered by keyboard shortcuts during pump_events.
    pub static PENDING_RELOADS: std::cell::RefCell<Vec<u32>> = std::cell::RefCell::new(Vec::new());
    /// Buffer for resize callback events deferred during pump_events.
    /// Each entry: (window_id, width, height).
    pub static PENDING_RESIZE_CALLBACKS: std::cell::RefCell<Vec<(u32, f64, f64)>> = std::cell::RefCell::new(Vec::new());
    /// Buffer for move callback events deferred during pump_events.
    /// Each entry: (window_id, x, y).
    pub static PENDING_MOVES: std::cell::RefCell<Vec<(u32, f64, f64)>> = std::cell::RefCell::new(Vec::new());
    /// Buffer for focus events deferred during pump_events.
    pub static PENDING_FOCUSES: std::cell::RefCell<Vec<u32>> = std::cell::RefCell::new(Vec::new());
    /// Buffer for blur events deferred during pump_events.
    pub static PENDING_BLURS: std::cell::RefCell<Vec<u32>> = std::cell::RefCell::new(Vec::new());
    /// Buffer for page load events deferred during pump_events: (window_id, event_type, url).
    /// event_type is "started" or "finished".
    pub static PENDING_PAGE_LOADS: std::cell::RefCell<Vec<(u32, String, String)>> = std::cell::RefCell::new(Vec::new());
    /// Buffer for navigation-blocked events deferred during pump_events: (window_id, url).
    pub static PENDING_NAVIGATION_BLOCKED: std::cell::RefCell<Vec<(u32, String)>> = std::cell::RefCell::new(Vec::new());
}

/// Execute a closure with mutable access to the global window manager.
pub fn with_manager<F, R>(f: F) -> R
where
    F: FnOnce(&mut WindowManager) -> R,
{
    MANAGER.with(|m| f(&mut m.borrow_mut()))
}

/// Extract the origin (scheme + host + port) from a URL string using the
/// WHATWG URL Standard (`url` crate). Returns `None` for malformed URLs or
/// URLs with opaque origins (e.g. `file:`, `data:`, `blob:`, custom schemes).
///
/// The returned origin string is fully normalized:
///   - Scheme and host are lowercased
///   - Default ports are stripped (80 for http, 443 for https)
///   - Userinfo is stripped
///   - IPv6 addresses are handled correctly
pub fn extract_origin(raw: &str) -> Option<String> {
    let parsed = url::Url::parse(raw).ok()?;
    let origin = parsed.origin();
    let serialized = origin.ascii_serialization();
    // Opaque origins serialize as "null" — treat as no valid origin.
    if serialized == "null" {
        return None;
    }
    Some(serialized)
}

/// Check if a source URL's origin matches any of the trusted origins for a window.
/// Returns `true` if:
///   - No trusted origins are configured for this window (allow all), or
///   - The source URL's origin matches one of the trusted origins.
pub fn is_origin_trusted(window_id: u32, source_url: &str) -> bool {
    TRUSTED_ORIGINS_MAP.with(|o| {
        let map = o.borrow();
        if let Some(origins) = map.get(&window_id) {
            if origins.is_empty() {
                return true;
            }
            match extract_origin(source_url) {
                Some(origin) => origins.contains(&origin),
                None => false, // Malformed URL = untrusted
            }
        } else {
            true // No trusted_origins configured = allow all
        }
    })
}

// ── Navigation host restriction ────────────────────────────────

/// Extract the host (without port) from a URL string.
/// Returns `None` for URLs without a host (e.g., `about:blank`, `data:` URIs).
fn extract_host(url: &str) -> Option<&str> {
    let scheme_end = url.find("://")?;
    let rest = &url[scheme_end + 3..];
    // Strip userinfo (user:pass@) if present
    let after_at = match rest.find('@') {
        Some(i) => &rest[i + 1..],
        None => rest,
    };
    // Find end of host (before /, ?, #, or end)
    let host_end = after_at
        .find(|c: char| c == '/' || c == '?' || c == '#')
        .unwrap_or(after_at.len());
    let host_port = &after_at[..host_end];
    if host_port.is_empty() {
        return None;
    }
    // Strip port — handle IPv6 [::1]:port
    let host = if host_port.starts_with('[') {
        let bracket_end = host_port.find(']').unwrap_or(host_port.len());
        &host_port[..=bracket_end]
    } else {
        match host_port.rfind(':') {
            Some(i) => &host_port[..i],
            None => host_port,
        }
    };
    if host.is_empty() {
        None
    } else {
        Some(host)
    }
}

/// Check if a URL's host is permitted by the window's `allowedHosts` list.
/// Returns `true` if:
///   - No `allowed_hosts` are configured for this window (allow all)
///   - The URL is an internal URL (`about:blank`, `native-window.local`)
///   - The URL's host matches one of the allowed patterns
///
/// Pattern matching (case-insensitive):
///   - Exact: `"example.com"` matches only `example.com`
///   - Wildcard: `"*.example.com"` matches `sub.example.com`,
///     `a.b.example.com`, AND `example.com` itself
pub fn is_host_allowed(window_id: u32, url: &str) -> bool {
    // Internal URLs are always allowed
    let lower = url.to_lowercase();
    if lower.starts_with("about:") || lower.contains("native-window.local") {
        return true;
    }

    ALLOWED_HOSTS_MAP.with(|h| {
        let map = h.borrow();
        if let Some(hosts) = map.get(&window_id) {
            if hosts.is_empty() {
                return true;
            }
            match extract_host(url) {
                Some(host) => {
                    let host_lower = host.to_lowercase();
                    hosts.iter().any(|pattern| {
                        let p = pattern.to_lowercase();
                        if let Some(suffix) = p.strip_prefix('*') {
                            // "*.example.com" → suffix = ".example.com"
                            // Match: host ends with ".example.com"
                            //    OR: host equals "example.com" (strip leading dot)
                            host_lower.ends_with(suffix)
                                || suffix
                                    .strip_prefix('.')
                                    .map_or(false, |bare| host_lower == bare)
                        } else {
                            host_lower == p
                        }
                    })
                }
                None => false, // No host extractable = blocked
            }
        } else {
            true // No allowed_hosts configured = allow all
        }
    })
}

// ── JSON helpers ────────────────────────────────────────────────

/// Escape a string for safe embedding as a JSON string value.
/// The returned string includes surrounding double quotes.
pub fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
