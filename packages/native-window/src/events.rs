use napi::threadsafe_function::{ErrorStrategy, ThreadsafeFunction};

/// Callback for string messages from the webview IPC: (message, source_url).
pub type MessageCallback = ThreadsafeFunction<(String, String), ErrorStrategy::Fatal>;

/// Callback for window close events.
pub type CloseCallback = ThreadsafeFunction<(), ErrorStrategy::Fatal>;

/// Callback for resize events: (width, height).
pub type ResizeCallback = ThreadsafeFunction<(f64, f64), ErrorStrategy::Fatal>;

/// Callback for move events: (x, y).
pub type MoveCallback = ThreadsafeFunction<(f64, f64), ErrorStrategy::Fatal>;

/// Callback for focus/blur events (no payload).
pub type FocusCallback = ThreadsafeFunction<(), ErrorStrategy::Fatal>;

/// Callback for page load events: (event_type, url)
/// event_type is "started" or "finished"
pub type PageLoadCallback = ThreadsafeFunction<(String, String), ErrorStrategy::Fatal>;

/// Callback for document title change events.
pub type TitleChangedCallback = ThreadsafeFunction<String, ErrorStrategy::Fatal>;

/// Callback for reload events (no payload).
pub type ReloadCallback = ThreadsafeFunction<(), ErrorStrategy::Fatal>;

/// Callback for cookie query results (JSON payload string).
/// The payload is a JSON array of cookie objects.
pub type CookiesCallback = ThreadsafeFunction<String, ErrorStrategy::Fatal>;

/// Callback for blocked navigation events: (url).
pub type NavigationBlockedCallback = ThreadsafeFunction<String, ErrorStrategy::Fatal>;

/// Stored event handlers for a window.
pub struct WindowEventHandlers {
    pub on_message: Option<MessageCallback>,
    pub on_close: Option<CloseCallback>,
    pub on_resize: Option<ResizeCallback>,
    pub on_move: Option<MoveCallback>,
    pub on_focus: Option<FocusCallback>,
    pub on_blur: Option<FocusCallback>,
    pub on_page_load: Option<PageLoadCallback>,
    pub on_title_changed: Option<TitleChangedCallback>,
    pub on_reload: Option<ReloadCallback>,
    pub on_cookies: Option<CookiesCallback>,
    pub on_navigation_blocked: Option<NavigationBlockedCallback>,
}

impl WindowEventHandlers {
    pub fn new() -> Self {
        Self {
            on_message: None,
            on_close: None,
            on_resize: None,
            on_move: None,
            on_focus: None,
            on_blur: None,
            on_page_load: None,
            on_title_changed: None,
            on_reload: None,
            on_cookies: None,
            on_navigation_blocked: None,
        }
    }
}
