use std::collections::HashMap;

use crate::events::WindowEventHandlers;
use crate::options::WindowOptions;

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
    /// Per-window trusted origins for IPC message filtering.
    /// When an entry exists and the Vec is non-empty, only IPC messages
    /// from matching origins are forwarded to JS.
    pub trusted_origins: HashMap<u32, Vec<String>>,
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
            trusted_origins: HashMap::new(),
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

    /// Remove event handlers and trusted origins for a closed window to prevent memory leaks.
    #[allow(dead_code)] // Used by macOS platform, not by Windows
    pub fn remove_event_handlers(&mut self, id: u32) {
        self.event_handlers.remove(&id);
        self.trusted_origins.remove(&id);
    }
}

thread_local! {
    pub static MANAGER: std::cell::RefCell<WindowManager> = std::cell::RefCell::new(WindowManager::new());
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
}

/// Execute a closure with mutable access to the global window manager.
pub fn with_manager<F, R>(f: F) -> R
where
    F: FnOnce(&mut WindowManager) -> R,
{
    MANAGER.with(|m| f(&mut m.borrow_mut()))
}

/// Extract the origin (scheme + host + port) from a URL string.
/// Returns `None` for malformed URLs or URLs without a valid origin.
/// Used for native-layer IPC origin validation.
pub fn extract_origin(url: &str) -> Option<String> {
    // Find "://" to split scheme from the rest
    let scheme_end = url.find("://")?;
    let scheme = &url[..scheme_end];
    if scheme.is_empty() {
        return None;
    }
    let rest = &url[scheme_end + 3..];
    // Extract host (+ optional port), stopping at '/' or '?' or '#' or end
    let host_end = rest
        .find(|c: char| c == '/' || c == '?' || c == '#')
        .unwrap_or(rest.len());
    let host_port = &rest[..host_end];
    if host_port.is_empty() {
        return None;
    }
    Some(format!("{}://{}", scheme, host_port))
}

/// Check if a source URL's origin matches any of the trusted origins for a window.
/// Returns `true` if:
///   - No trusted origins are configured for this window (allow all), or
///   - The source URL's origin matches one of the trusted origins.
pub fn is_origin_trusted(window_id: u32, source_url: &str) -> bool {
    MANAGER.with(|m| {
        match m.try_borrow() {
            Ok(mgr) => {
                if let Some(origins) = mgr.trusted_origins.get(&window_id) {
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
            }
            Err(_) => true, // Can't borrow = allow (deferred messages checked later)
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
