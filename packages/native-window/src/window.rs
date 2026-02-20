use napi::bindgen_prelude::*;
use napi::threadsafe_function::{ErrorStrategy, ThreadSafeCallContext, ThreadsafeFunction};
use napi::JsFunction;
use napi_derive::napi;

use crate::options::WindowOptions;
use crate::window_manager::{with_manager, Command};

/// A native OS window with an embedded webview.
#[napi]
pub struct NativeWindow {
    id: u32,
}

#[napi]
impl NativeWindow {
    /// Create a new native window with the given options.
    /// The window is created asynchronously during the next `pumpEvents()` call.
    #[napi(constructor)]
    pub fn new(options: Option<WindowOptions>) -> Result<Self> {
        let opts = options.unwrap_or_default();

        let id = with_manager(|mgr| {
            if !mgr.initialized {
                return Err(napi::Error::from_reason(
                    "Native window system not initialized. Call init() first.",
                ));
            }
            let id = mgr.allocate_id()?;
            // Store trusted origins for native-layer IPC filtering
            if let Some(ref origins) = opts.trusted_origins {
                if !origins.is_empty() {
                    mgr.trusted_origins.insert(id, origins.clone());
                }
            }
            mgr.push_command(Command::CreateWindow {
                id,
                options: opts,
            });
            Ok(id)
        })?;

        Ok(Self { id })
    }

    /// Get the unique window ID.
    #[napi(getter)]
    pub fn id(&self) -> u32 {
        self.id
    }

    // ---- Content loading ----

    /// Load a URL in the webview.
    /// Blocks `javascript:`, `file:`, `data:`, and `blob:` URLs for security.
    #[napi]
    pub fn load_url(&self, url: String) -> Result<()> {
        let lower = url.trim().to_lowercase();
        if lower.starts_with("javascript:")
            || lower.starts_with("file:")
            || lower.starts_with("data:")
            || lower.starts_with("blob:")
        {
            return Err(napi::Error::from_reason(
                "Blocked: javascript:, file:, data:, and blob: URLs are not allowed in loadUrl(). \
                 Use evaluateJs() for script execution.",
            ));
        }
        with_manager(|mgr| {
            mgr.push_command(Command::LoadURL {
                id: self.id,
                url,
            });
        });
        Ok(())
    }

    /// Load an HTML string directly in the webview.
    #[napi]
    pub fn load_html(&self, html: String) -> Result<()> {
        with_manager(|mgr| {
            mgr.push_command(Command::LoadHTML {
                id: self.id,
                html,
            });
        });
        Ok(())
    }

    /// Execute JavaScript code in the webview context.
    /// This is fire-and-forget; use onMessage to receive results.
    #[napi]
    pub fn evaluate_js(&self, script: String) -> Result<()> {
        with_manager(|mgr| {
            mgr.push_command(Command::EvaluateJS {
                id: self.id,
                script,
            });
        });
        Ok(())
    }

    /// Send a message to the webview.
    /// This calls `window.__native_message__(msg)` in the webview context.
    #[napi]
    pub fn post_message(&self, message: String) -> Result<()> {
        let escaped = message
            .replace('\\', "\\\\")
            .replace('\'', "\\'")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\0', "\\0")
            .replace('\u{2028}', "\\u2028")
            .replace('\u{2029}', "\\u2029")
            .replace("</", "<\\/");
        let script = format!("if(window.__native_message__)window.__native_message__('{}');", escaped);
        with_manager(|mgr| {
            mgr.push_command(Command::EvaluateJS {
                id: self.id,
                script,
            });
        });
        Ok(())
    }

    // ---- Window control ----

    /// Set the window title.
    #[napi]
    pub fn set_title(&self, title: String) -> Result<()> {
        with_manager(|mgr| {
            mgr.push_command(Command::SetTitle {
                id: self.id,
                title,
            });
        });
        Ok(())
    }

    /// Set the window size in logical pixels.
    #[napi]
    pub fn set_size(&self, width: f64, height: f64) -> Result<()> {
        with_manager(|mgr| {
            mgr.push_command(Command::SetSize {
                id: self.id,
                width,
                height,
            });
        });
        Ok(())
    }

    /// Set the minimum window size.
    #[napi]
    pub fn set_min_size(&self, width: f64, height: f64) -> Result<()> {
        with_manager(|mgr| {
            mgr.push_command(Command::SetMinSize {
                id: self.id,
                width,
                height,
            });
        });
        Ok(())
    }

    /// Set the maximum window size.
    #[napi]
    pub fn set_max_size(&self, width: f64, height: f64) -> Result<()> {
        with_manager(|mgr| {
            mgr.push_command(Command::SetMaxSize {
                id: self.id,
                width,
                height,
            });
        });
        Ok(())
    }

    /// Set the window position in screen coordinates.
    #[napi]
    pub fn set_position(&self, x: f64, y: f64) -> Result<()> {
        with_manager(|mgr| {
            mgr.push_command(Command::SetPosition {
                id: self.id,
                x,
                y,
            });
        });
        Ok(())
    }

    /// Set whether the window is resizable.
    #[napi]
    pub fn set_resizable(&self, resizable: bool) -> Result<()> {
        with_manager(|mgr| {
            mgr.push_command(Command::SetResizable {
                id: self.id,
                resizable,
            });
        });
        Ok(())
    }

    /// Set whether the window has decorations (title bar, borders).
    #[napi]
    pub fn set_decorations(&self, decorations: bool) -> Result<()> {
        with_manager(|mgr| {
            mgr.push_command(Command::SetDecorations {
                id: self.id,
                decorations,
            });
        });
        Ok(())
    }

    /// Set whether the window is always on top.
    #[napi]
    pub fn set_always_on_top(&self, always_on_top: bool) -> Result<()> {
        with_manager(|mgr| {
            mgr.push_command(Command::SetAlwaysOnTop {
                id: self.id,
                always_on_top: always_on_top,
            });
        });
        Ok(())
    }

    /// Show the window.
    #[napi]
    pub fn show(&self) -> Result<()> {
        with_manager(|mgr| {
            mgr.push_command(Command::Show { id: self.id });
        });
        Ok(())
    }

    /// Hide the window.
    #[napi]
    pub fn hide(&self) -> Result<()> {
        with_manager(|mgr| {
            mgr.push_command(Command::Hide { id: self.id });
        });
        Ok(())
    }

    /// Close and destroy the window.
    #[napi]
    pub fn close(&self) -> Result<()> {
        with_manager(|mgr| {
            mgr.push_command(Command::Close { id: self.id });
        });
        Ok(())
    }

    /// Focus the window.
    #[napi]
    pub fn focus(&self) -> Result<()> {
        with_manager(|mgr| {
            mgr.push_command(Command::Focus { id: self.id });
        });
        Ok(())
    }

    /// Maximize the window.
    #[napi]
    pub fn maximize(&self) -> Result<()> {
        with_manager(|mgr| {
            mgr.push_command(Command::Maximize { id: self.id });
        });
        Ok(())
    }

    /// Minimize the window.
    #[napi]
    pub fn minimize(&self) -> Result<()> {
        with_manager(|mgr| {
            mgr.push_command(Command::Minimize { id: self.id });
        });
        Ok(())
    }

    /// Restore the window from maximized state.
    #[napi]
    pub fn unmaximize(&self) -> Result<()> {
        with_manager(|mgr| {
            mgr.push_command(Command::Unmaximize { id: self.id });
        });
        Ok(())
    }

    /// Reload the current page in the webview.
    #[napi]
    pub fn reload(&self) -> Result<()> {
        with_manager(|mgr| {
            mgr.push_command(Command::Reload { id: self.id });
        });
        Ok(())
    }

    // ---- Event handlers ----

    /// Register a handler for IPC messages from the webview.
    /// In the webview, call `window.ipc.postMessage(string)` to send messages.
    /// The callback receives the message string and the source page URL.
    #[napi(ts_args_type = "callback: (message: string, sourceUrl: string) => void")]
    pub fn on_message(&self, callback: JsFunction) -> Result<()> {
        let tsfn: ThreadsafeFunction<(String, String), ErrorStrategy::Fatal> =
            callback.create_threadsafe_function(0, |ctx: ThreadSafeCallContext<(String, String)>| {
                let message = ctx.env.create_string(&ctx.value.0)?;
                let source_url = ctx.env.create_string(&ctx.value.1)?;
                Ok(vec![message, source_url])
            })?;

        with_manager(|mgr| {
            if let Some(handlers) = mgr.event_handlers.get_mut(&self.id) {
                handlers.on_message = Some(tsfn);
            }
        });
        Ok(())
    }

    /// Register a handler for the window close event.
    #[napi(ts_args_type = "callback: () => void")]
    pub fn on_close(&self, callback: JsFunction) -> Result<()> {
        let tsfn: ThreadsafeFunction<(), ErrorStrategy::Fatal> =
            callback.create_threadsafe_function(0, |ctx: ThreadSafeCallContext<()>| {
                ctx.env.get_undefined().map(|v| vec![v])
            })?;

        with_manager(|mgr| {
            if let Some(handlers) = mgr.event_handlers.get_mut(&self.id) {
                handlers.on_close = Some(tsfn);
            }
        });
        Ok(())
    }

    /// Register a handler for window resize events.
    #[napi(ts_args_type = "callback: (width: number, height: number) => void")]
    pub fn on_resize(&self, callback: JsFunction) -> Result<()> {
        let tsfn: ThreadsafeFunction<(f64, f64), ErrorStrategy::Fatal> =
            callback.create_threadsafe_function(0, |ctx: ThreadSafeCallContext<(f64, f64)>| {
                let width = ctx.env.create_double(ctx.value.0)?;
                let height = ctx.env.create_double(ctx.value.1)?;
                Ok(vec![width, height])
            })?;

        with_manager(|mgr| {
            if let Some(handlers) = mgr.event_handlers.get_mut(&self.id) {
                handlers.on_resize = Some(tsfn);
            }
        });
        Ok(())
    }

    /// Register a handler for window move events.
    #[napi(ts_args_type = "callback: (x: number, y: number) => void")]
    pub fn on_move(&self, callback: JsFunction) -> Result<()> {
        let tsfn: ThreadsafeFunction<(f64, f64), ErrorStrategy::Fatal> =
            callback.create_threadsafe_function(0, |ctx: ThreadSafeCallContext<(f64, f64)>| {
                let x = ctx.env.create_double(ctx.value.0)?;
                let y = ctx.env.create_double(ctx.value.1)?;
                Ok(vec![x, y])
            })?;

        with_manager(|mgr| {
            if let Some(handlers) = mgr.event_handlers.get_mut(&self.id) {
                handlers.on_move = Some(tsfn);
            }
        });
        Ok(())
    }

    /// Register a handler for window focus events.
    #[napi(ts_args_type = "callback: () => void")]
    pub fn on_focus(&self, callback: JsFunction) -> Result<()> {
        let tsfn: ThreadsafeFunction<(), ErrorStrategy::Fatal> =
            callback.create_threadsafe_function(0, |ctx: ThreadSafeCallContext<()>| {
                ctx.env.get_undefined().map(|v| vec![v])
            })?;

        with_manager(|mgr| {
            if let Some(handlers) = mgr.event_handlers.get_mut(&self.id) {
                handlers.on_focus = Some(tsfn);
            }
        });
        Ok(())
    }

    /// Register a handler for window blur (lost focus) events.
    #[napi(ts_args_type = "callback: () => void")]
    pub fn on_blur(&self, callback: JsFunction) -> Result<()> {
        let tsfn: ThreadsafeFunction<(), ErrorStrategy::Fatal> =
            callback.create_threadsafe_function(0, |ctx: ThreadSafeCallContext<()>| {
                ctx.env.get_undefined().map(|v| vec![v])
            })?;

        with_manager(|mgr| {
            if let Some(handlers) = mgr.event_handlers.get_mut(&self.id) {
                handlers.on_blur = Some(tsfn);
            }
        });
        Ok(())
    }

    /// Register a handler for page load events.
    #[napi(ts_args_type = "callback: (event: 'started' | 'finished', url: string) => void")]
    pub fn on_page_load(&self, callback: JsFunction) -> Result<()> {
        let tsfn: ThreadsafeFunction<(String, String), ErrorStrategy::Fatal> =
            callback.create_threadsafe_function(0, |ctx: ThreadSafeCallContext<(String, String)>| {
                let event_type = ctx.env.create_string(&ctx.value.0)?;
                let url = ctx.env.create_string(&ctx.value.1)?;
                Ok(vec![event_type, url])
            })?;

        with_manager(|mgr| {
            if let Some(handlers) = mgr.event_handlers.get_mut(&self.id) {
                handlers.on_page_load = Some(tsfn);
            }
        });
        Ok(())
    }

    /// Register a handler for document title change events.
    #[napi(ts_args_type = "callback: (title: string) => void")]
    pub fn on_title_changed(&self, callback: JsFunction) -> Result<()> {
        let tsfn: ThreadsafeFunction<String, ErrorStrategy::Fatal> =
            callback.create_threadsafe_function(0, |ctx: ThreadSafeCallContext<String>| {
                ctx.env.create_string(ctx.value.as_str()).map(|v| vec![v])
            })?;

        with_manager(|mgr| {
            if let Some(handlers) = mgr.event_handlers.get_mut(&self.id) {
                handlers.on_title_changed = Some(tsfn);
            }
        });
        Ok(())
    }

    /// Register a handler for the window reload event.
    #[napi(ts_args_type = "callback: () => void")]
    pub fn on_reload(&self, callback: JsFunction) -> Result<()> {
        let tsfn: ThreadsafeFunction<(), ErrorStrategy::Fatal> =
            callback.create_threadsafe_function(0, |ctx: ThreadSafeCallContext<()>| {
                ctx.env.get_undefined().map(|v| vec![v])
            })?;

        with_manager(|mgr| {
            if let Some(handlers) = mgr.event_handlers.get_mut(&self.id) {
                handlers.on_reload = Some(tsfn);
            }
        });
        Ok(())
    }
}
