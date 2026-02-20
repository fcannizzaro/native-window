use std::collections::HashMap;

use napi::threadsafe_function::ThreadsafeFunctionCallMode;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{define_class, msg_send, ClassType, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSEvent, NSEventMask,
    NSFloatingWindowLevel, NSNormalWindowLevel, NSRunningApplication, NSWindow,
    NSWindowDelegate, NSWindowStyleMask,
};
use objc2_foundation::{
    ns_string, NSDate, NSDefaultRunLoopMode, NSNotification, NSObjectProtocol, NSPoint, NSRect,
    NSSize, NSString,
};
use objc2_web_kit::{
    WKNavigation, WKNavigationAction, WKNavigationDelegate, WKScriptMessage,
    WKScriptMessageHandler, WKUserContentController, WKUserScript, WKUserScriptInjectionTime,
    WKWebView, WKWebViewConfiguration,
};

use crate::events::WindowEventHandlers;
use crate::options::WindowOptions;
use crate::window_manager::Command;

/// Maximum IPC message size in bytes (10 MB). Messages exceeding this
/// are silently dropped to prevent memory exhaustion from the webview.
const MAX_MESSAGE_SIZE: usize = 10 * 1024 * 1024;

/// Tracks the last loaded content for reliable reload.
enum LoadedContent {
    Url,
    Html(String),
}

/// A window entry containing the native window and webview.
struct WindowEntry {
    window: Retained<NSWindow>,
    webview: Retained<WKWebView>,
    /// Prevent the delegate from being deallocated while the window is alive.
    _close_delegate: Retained<ProtocolObject<dyn NSWindowDelegate>>,
    /// Prevent the navigation delegate from being deallocated while the window is alive.
    _nav_delegate: Retained<ProtocolObject<dyn WKNavigationDelegate>>,
    /// Last loaded content for reload support.
    loaded_content: Option<LoadedContent>,
}

/// macOS platform state.
pub struct MacOSPlatform {
    windows: HashMap<u32, WindowEntry>,
    mtm: MainThreadMarker,
}

// IPC message handler delegate
define_class!(
    #[unsafe(super(objc2::runtime::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "IPCMessageHandler"]
    #[ivars = u32] // window id
    struct IPCMessageHandler;

    unsafe impl NSObjectProtocol for IPCMessageHandler {}

    unsafe impl WKScriptMessageHandler for IPCMessageHandler {
        #[unsafe(method(userContentController:didReceiveScriptMessage:))]
        fn did_receive_script_message(
            &self,
            _controller: &WKUserContentController,
            message: &WKScriptMessage,
        ) {
            let window_id = *self.ivars();
            let body = unsafe { message.body() };
            // Convert the message body to a string
            let body_str: Retained<NSString> = unsafe { msg_send![&body, description] };
            let message_string = body_str.to_string();

            // Drop oversized messages to prevent memory exhaustion
            if message_string.len() > MAX_MESSAGE_SIZE {
                return;
            }

            // Extract source URL from frameInfo → request → URL
            let source_url: String = unsafe {
                let frame_info: *const objc2::runtime::AnyObject = msg_send![message, frameInfo];
                if frame_info.is_null() {
                    String::new()
                } else {
                    let request: *const objc2::runtime::AnyObject = msg_send![frame_info, request];
                    if request.is_null() {
                        String::new()
                    } else {
                        let url: *const objc2::runtime::AnyObject = msg_send![request, URL];
                        if url.is_null() {
                            String::new()
                        } else {
                            let abs: Retained<NSString> = msg_send![url, absoluteString];
                            abs.to_string()
                        }
                    }
                }
            };

            use crate::window_manager::{MANAGER, PENDING_MESSAGES, is_origin_trusted};

            // Check trusted origins at the native layer (defense-in-depth)
            if !is_origin_trusted(window_id, &source_url) {
                return;
            }

            MANAGER.with(|m| {
                match m.try_borrow() {
                    Ok(mgr) => {
                        if let Some(handlers) = mgr.event_handlers.get(&window_id) {
                            if let Some(ref cb) = handlers.on_message {
                                cb.call(
                                    (message_string.clone(), source_url.clone()),
                                    ThreadsafeFunctionCallMode::NonBlocking,
                                );
                            }
                        }
                    }
                    Err(_) => {
                        // RefCell is already borrowed by pump_events — defer this message
                        PENDING_MESSAGES.with(|p| {
                            p.borrow_mut().push((window_id, message_string.clone(), source_url.clone()));
                        });
                    }
                }
            });
        }
    }
);

impl IPCMessageHandler {
    fn new(mtm: MainThreadMarker, window_id: u32) -> Retained<Self> {
        let handler = Self::alloc(mtm).set_ivars(window_id);
        unsafe { msg_send![super(handler), init] }
    }
}

// Window close delegate — fires on_close callback when the user closes the window
define_class!(
    #[unsafe(super(objc2::runtime::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "WindowCloseDelegate"]
    #[ivars = u32] // window id
    struct WindowCloseDelegate;

    unsafe impl NSObjectProtocol for WindowCloseDelegate {}

    unsafe impl NSWindowDelegate for WindowCloseDelegate {
        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, _notification: &NSNotification) {
            let window_id = *self.ivars();

            use crate::window_manager::{MANAGER, PENDING_CLOSES};

            MANAGER.with(|m| {
                match m.try_borrow() {
                    Ok(mgr) => {
                        if let Some(handlers) = mgr.event_handlers.get(&window_id) {
                            if let Some(ref cb) = handlers.on_close {
                                cb.call((), ThreadsafeFunctionCallMode::NonBlocking);
                            }
                        }
                    }
                    Err(_) => {
                        // RefCell is already borrowed by pump_events — defer this close event
                        PENDING_CLOSES.with(|p| {
                            p.borrow_mut().push(window_id);
                        });
                    }
                }
            });
        }
    }
);

impl WindowCloseDelegate {
    fn new(mtm: MainThreadMarker, window_id: u32) -> Retained<Self> {
        let delegate = Self::alloc(mtm).set_ivars(window_id);
        unsafe { msg_send![super(delegate), init] }
    }
}

thread_local! {
    /// Stores the last loaded HTML content per window id for the navigation delegate.
    /// Separate from MANAGER to avoid RefCell borrow conflicts during event dispatch.
    static LOADED_HTML: std::cell::RefCell<HashMap<u32, String>> = std::cell::RefCell::new(HashMap::new());
}

// Navigation delegate — intercepts reload navigations for HTML-loaded content
define_class!(
    #[unsafe(super(objc2::runtime::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "NavigationDelegate"]
    #[ivars = u32] // window id
    struct NavigationDelegate;

    unsafe impl NSObjectProtocol for NavigationDelegate {}

    unsafe impl WKNavigationDelegate for NavigationDelegate {
        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        fn decide_policy(
            &self,
            web_view: &WKWebView,
            navigation_action: &WKNavigationAction,
            decision_handler: &block2::Block<dyn Fn(objc2_web_kit::WKNavigationActionPolicy)>,
        ) {
            let window_id = *self.ivars();
            let nav_type: isize = unsafe { msg_send![navigation_action, navigationType] };

            // WKNavigationType.reload == 3
            if nav_type == 3 {
                let html = LOADED_HTML.with(|h| h.borrow().get(&window_id).cloned());
                if let Some(html) = html {
                    // Cancel the native reload — it would show a blank page for HTML content
                    decision_handler.call((objc2_web_kit::WKNavigationActionPolicy::Cancel,));
                    // Re-apply the stored HTML with synthetic base URL
                    unsafe {
                        let html_string = NSString::from_str(&html);
                        let base_url_str = NSString::from_str("https://native-window.local/");
                        let base_url: Option<Retained<objc2_foundation::NSURL>> =
                            msg_send![objc2_foundation::NSURL::class(), URLWithString: &*base_url_str];
                        let base_url_ptr = base_url
                            .as_deref()
                            .map(|u| u as *const objc2_foundation::NSURL)
                            .unwrap_or(std::ptr::null());
                        let _: Option<Retained<WKNavigation>> =
                            msg_send![web_view, loadHTMLString: &*html_string, baseURL: base_url_ptr];
                    }
                    // Defer on_reload event to be flushed after pump_events
                    crate::window_manager::PENDING_RELOADS.with(|p| {
                        p.borrow_mut().push(window_id);
                    });
                    return;
                }
            }

            // Block dangerous URL schemes (javascript:, file:, data:, blob:)
            let should_block = unsafe {
                let request: *const objc2::runtime::AnyObject = msg_send![navigation_action, request];
                if !request.is_null() {
                    let url: *const objc2::runtime::AnyObject = msg_send![request, URL];
                    if !url.is_null() {
                        let scheme: Option<Retained<NSString>> = msg_send![url, scheme];
                        if let Some(scheme) = scheme {
                            let s = scheme.to_string().to_lowercase();
                            s == "javascript" || s == "file" || s == "data" || s == "blob"
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                } else {
                    false
                }
            };

            if should_block {
                decision_handler.call((objc2_web_kit::WKNavigationActionPolicy::Cancel,));
            } else {
                decision_handler.call((objc2_web_kit::WKNavigationActionPolicy::Allow,));
            }
        }
    }
);

impl NavigationDelegate {
    fn new(mtm: MainThreadMarker, window_id: u32) -> Retained<Self> {
        let delegate = Self::alloc(mtm).set_ivars(window_id);
        unsafe { msg_send![super(delegate), init] }
    }
}

impl MacOSPlatform {
    pub fn new() -> napi::Result<Self> {
        let mtm = MainThreadMarker::new().ok_or_else(|| {
            napi::Error::from_reason("Must be called from the main thread")
        })?;

        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);

        // Activate the application
        {
            let current_app = NSRunningApplication::currentApplication();
            #[allow(deprecated)]
            current_app.activateWithOptions(
                objc2_app_kit::NSApplicationActivationOptions::ActivateIgnoringOtherApps,
            );
        }

        Ok(Self {
            windows: HashMap::new(),
            mtm,
        })
    }

    pub fn create_window(
        &mut self,
        id: u32,
        options: &WindowOptions,
        handlers: &mut WindowEventHandlers,
    ) -> napi::Result<()> {
        let width = options.width.unwrap_or(800.0);
        let height = options.height.unwrap_or(600.0);
        let title = options
            .title
            .as_deref()
            .unwrap_or("");
        let resizable = options.resizable.unwrap_or(true);
        let decorations = options.decorations.unwrap_or(true);
        let transparent = options.transparent.unwrap_or(false);
        let always_on_top = options.always_on_top.unwrap_or(false);
        let visible = options.visible.unwrap_or(true);
        let devtools = options.devtools.unwrap_or(false);

        // Build style mask
        let mut style = NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable
            | NSWindowStyleMask::Titled;

        if resizable {
            style |= NSWindowStyleMask::Resizable;
        }

        if !decorations {
            style = NSWindowStyleMask::Borderless;
            if resizable {
                style |= NSWindowStyleMask::Resizable;
            }
        }

        let frame = NSRect::new(
            NSPoint::new(
                options.x.unwrap_or(100.0),
                options.y.unwrap_or(100.0),
            ),
            NSSize::new(width, height),
        );

        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(self.mtm),
                frame,
                style,
                NSBackingStoreType::Buffered,
                false,
            )
        };

        window.setTitle(&NSString::from_str(title));

        if let (Some(min_w), Some(min_h)) = (options.min_width, options.min_height) {
            window.setMinSize(NSSize::new(min_w, min_h));
        }

        if let (Some(max_w), Some(max_h)) = (options.max_width, options.max_height) {
            window.setMaxSize(NSSize::new(max_w, max_h));
        }

        if always_on_top {
            window.setLevel(NSFloatingWindowLevel);
        }

        if transparent {
            window.setOpaque(false);
            window.setBackgroundColor(Some(
                &objc2_app_kit::NSColor::clearColor(),
            ));
        }

        // Create WKWebView configuration
        let config =
            unsafe { WKWebViewConfiguration::new(self.mtm) };

        // Set up IPC handler
        let content_controller = unsafe { config.userContentController() };
        let ipc_handler = IPCMessageHandler::new(self.mtm, id);
        let ipc_handler_proto =
            ProtocolObject::from_retained(ipc_handler);
        unsafe {
            content_controller
                .addScriptMessageHandler_name(&ipc_handler_proto, ns_string!("ipc"));
        }

        // Inject IPC bridge script: capture the native postMessage reference immediately
        // and create window.ipc as a frozen, non-writable alias so both platforms use the same API.
        // Capturing early prevents page scripts from intercepting the pathway.
        let bridge_script = NSString::from_str(
            "(function(){var _post=window.webkit.messageHandlers.ipc.postMessage.bind(window.webkit.messageHandlers.ipc);Object.defineProperty(window,'ipc',{value:Object.freeze({postMessage:function(msg){_post(msg)}}),writable:false,configurable:false})})();",
        );
        let user_script = unsafe {
            WKUserScript::initWithSource_injectionTime_forMainFrameOnly(
                WKUserScript::alloc(self.mtm),
                &bridge_script,
                WKUserScriptInjectionTime::AtDocumentStart,
                true,
            )
        };
        unsafe {
            content_controller.addUserScript(&user_script);
        }

        // Inject Content-Security-Policy meta tag if configured
        if let Some(ref csp) = options.csp {
            // Escape single quotes in the CSP value for safe JS string embedding
            let escaped_csp = csp.replace('\\', "\\\\").replace('\'', "\\'");
            let csp_script_str = format!(
                "document.addEventListener('DOMContentLoaded',function(){{var m=document.createElement('meta');m.httpEquiv='Content-Security-Policy';m.content='{}';document.head.insertBefore(m,document.head.firstChild)}},{{once:true}});",
                escaped_csp
            );
            let csp_script = NSString::from_str(&csp_script_str);
            let csp_user_script = unsafe {
                WKUserScript::initWithSource_injectionTime_forMainFrameOnly(
                    WKUserScript::alloc(self.mtm),
                    &csp_script,
                    WKUserScriptInjectionTime::AtDocumentStart,
                    true,
                )
            };
            unsafe {
                content_controller.addUserScript(&csp_user_script);
            }
        }

        // Create webview
        let content_rect = window.contentRectForFrameRect(window.frame());
        let webview = unsafe {
            WKWebView::initWithFrame_configuration(WKWebView::alloc(self.mtm), content_rect, &config)
        };

        // Enable Safari Web Inspector (macOS 13.3+)
        if devtools {
            unsafe {
                let _: () = msg_send![&webview, setInspectable: true];
            }
        }

        // Make webview fill the window
        webview.setAutoresizingMask(
            objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable
                | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable,
        );

        // Set webview as the window's content view
        window.setContentView(Some(&webview));

        // Set window close delegate to fire on_close callback
        let close_delegate = WindowCloseDelegate::new(self.mtm, id);
        let close_delegate_proto = ProtocolObject::from_retained(close_delegate);
        window.setDelegate(Some(&close_delegate_proto));

        // Set navigation delegate to intercept reload for HTML content
        let nav_delegate = NavigationDelegate::new(self.mtm, id);
        let nav_delegate_proto: Retained<ProtocolObject<dyn WKNavigationDelegate>> =
            ProtocolObject::from_retained(nav_delegate);
        unsafe {
            webview.setNavigationDelegate(Some(&nav_delegate_proto));
        }

        if visible {
            window.makeKeyAndOrderFront(None);
        }

        let _ = handlers; // handlers are stored in the manager's event_handlers map

        self.windows.insert(
            id,
            WindowEntry {
                window,
                webview,
                _close_delegate: close_delegate_proto,
                _nav_delegate: nav_delegate_proto,
                loaded_content: None,
            },
        );

        Ok(())
    }

    pub fn load_url(&mut self, id: u32, url: &str) -> napi::Result<()> {
        let entry = self
            .windows
            .get(&id)
            .ok_or_else(|| napi::Error::from_reason(format!("Window {} not found", id)))?;

        unsafe {
            let url_string = NSString::from_str(url);
            let nsurl: Retained<objc2_foundation::NSURL> =
                msg_send![objc2_foundation::NSURL::class(), URLWithString: &*url_string];
            let request: Retained<objc2_foundation::NSURLRequest> =
                msg_send![objc2_foundation::NSURLRequest::class(), requestWithURL: &*nsurl];
            let _: Option<Retained<WKNavigation>> = msg_send![&entry.webview, loadRequest: &*request];
        }

        let entry = self.windows.get_mut(&id).unwrap();
        entry.loaded_content = Some(LoadedContent::Url);
        LOADED_HTML.with(|h| h.borrow_mut().remove(&id));
        Ok(())
    }

    pub fn load_html(&mut self, id: u32, html: &str) -> napi::Result<()> {
        let entry = self
            .windows
            .get(&id)
            .ok_or_else(|| napi::Error::from_reason(format!("Window {} not found", id)))?;

        unsafe {
            let html_string = NSString::from_str(html);
            // Use a synthetic base URL to establish a proper security origin
            // instead of about:blank (null base URL weakens same-origin policy).
            let base_url_str = NSString::from_str("https://native-window.local/");
            let base_url: Option<Retained<objc2_foundation::NSURL>> =
                msg_send![objc2_foundation::NSURL::class(), URLWithString: &*base_url_str];
            let base_url_ptr = base_url
                .as_deref()
                .map(|u| u as *const objc2_foundation::NSURL)
                .unwrap_or(std::ptr::null());
            let _: Option<Retained<WKNavigation>> = msg_send![&entry.webview, loadHTMLString: &*html_string, baseURL: base_url_ptr];
        }

        let entry = self.windows.get_mut(&id).unwrap();
        entry.loaded_content = Some(LoadedContent::Html(html.to_string()));
        LOADED_HTML.with(|h| h.borrow_mut().insert(id, html.to_string()));
        Ok(())
    }

    pub fn evaluate_js(&self, id: u32, script: &str) -> napi::Result<()> {
        let entry = self
            .windows
            .get(&id)
            .ok_or_else(|| napi::Error::from_reason(format!("Window {} not found", id)))?;

        unsafe {
            let js_string = NSString::from_str(script);
            let null_handler: *const block2::Block<dyn Fn(*const objc2::runtime::AnyObject, *const objc2::runtime::AnyObject)> =
                std::ptr::null();
            let _: () = msg_send![&entry.webview, evaluateJavaScript: &*js_string, completionHandler: null_handler];
        }
        Ok(())
    }

    pub fn set_title(&self, id: u32, title: &str) -> napi::Result<()> {
        let entry = self
            .windows
            .get(&id)
            .ok_or_else(|| napi::Error::from_reason(format!("Window {} not found", id)))?;
        entry.window.setTitle(&NSString::from_str(title));
        Ok(())
    }

    pub fn set_size(&self, id: u32, width: f64, height: f64) -> napi::Result<()> {
        let entry = self
            .windows
            .get(&id)
            .ok_or_else(|| napi::Error::from_reason(format!("Window {} not found", id)))?;
        let frame = entry.window.frame();
        let new_frame = NSRect::new(frame.origin, NSSize::new(width, height));
        entry
            .window
            .setFrame_display(new_frame, true);
        Ok(())
    }

    pub fn set_min_size(&self, id: u32, width: f64, height: f64) -> napi::Result<()> {
        let entry = self
            .windows
            .get(&id)
            .ok_or_else(|| napi::Error::from_reason(format!("Window {} not found", id)))?;
        entry.window.setMinSize(NSSize::new(width, height));
        Ok(())
    }

    pub fn set_max_size(&self, id: u32, width: f64, height: f64) -> napi::Result<()> {
        let entry = self
            .windows
            .get(&id)
            .ok_or_else(|| napi::Error::from_reason(format!("Window {} not found", id)))?;
        entry.window.setMaxSize(NSSize::new(width, height));
        Ok(())
    }

    pub fn set_position(&self, id: u32, x: f64, y: f64) -> napi::Result<()> {
        let entry = self
            .windows
            .get(&id)
            .ok_or_else(|| napi::Error::from_reason(format!("Window {} not found", id)))?;
        entry
            .window
            .setFrameOrigin(NSPoint::new(x, y));
        Ok(())
    }

    pub fn set_resizable(&self, id: u32, resizable: bool) -> napi::Result<()> {
        let entry = self
            .windows
            .get(&id)
            .ok_or_else(|| napi::Error::from_reason(format!("Window {} not found", id)))?;
        let mut style = entry.window.styleMask();
        if resizable {
            style |= NSWindowStyleMask::Resizable;
        } else {
            style &= !NSWindowStyleMask::Resizable;
        }
        entry.window.setStyleMask(style);
        Ok(())
    }

    pub fn set_decorations(&self, id: u32, decorations: bool) -> napi::Result<()> {
        let entry = self
            .windows
            .get(&id)
            .ok_or_else(|| napi::Error::from_reason(format!("Window {} not found", id)))?;
        let resizable = entry
            .window
            .styleMask()
            .contains(NSWindowStyleMask::Resizable);

        let mut style = if decorations {
            NSWindowStyleMask::Closable
                | NSWindowStyleMask::Miniaturizable
                | NSWindowStyleMask::Titled
        } else {
            NSWindowStyleMask::Borderless
        };

        if resizable {
            style |= NSWindowStyleMask::Resizable;
        }

        entry.window.setStyleMask(style);
        Ok(())
    }

    pub fn set_always_on_top(&self, id: u32, always_on_top: bool) -> napi::Result<()> {
        let entry = self
            .windows
            .get(&id)
            .ok_or_else(|| napi::Error::from_reason(format!("Window {} not found", id)))?;
        if always_on_top {
            entry.window.setLevel(NSFloatingWindowLevel);
        } else {
            entry.window.setLevel(NSNormalWindowLevel);
        }
        Ok(())
    }

    pub fn show(&self, id: u32) -> napi::Result<()> {
        let entry = self
            .windows
            .get(&id)
            .ok_or_else(|| napi::Error::from_reason(format!("Window {} not found", id)))?;
        entry.window.makeKeyAndOrderFront(None);
        Ok(())
    }

    pub fn hide(&self, id: u32) -> napi::Result<()> {
        let entry = self
            .windows
            .get(&id)
            .ok_or_else(|| napi::Error::from_reason(format!("Window {} not found", id)))?;
        entry.window.orderOut(None);
        Ok(())
    }

    pub fn close(&mut self, id: u32) -> napi::Result<()> {
        let entry = self
            .windows
            .remove(&id)
            .ok_or_else(|| napi::Error::from_reason(format!("Window {} not found", id)))?;
        entry.window.close();
        LOADED_HTML.with(|h| h.borrow_mut().remove(&id));
        // Clean up event handlers to prevent memory leaks
        crate::window_manager::with_manager(|mgr| {
            mgr.remove_event_handlers(id);
        });
        Ok(())
    }

    pub fn focus(&self, id: u32) -> napi::Result<()> {
        let entry = self
            .windows
            .get(&id)
            .ok_or_else(|| napi::Error::from_reason(format!("Window {} not found", id)))?;
        entry.window.makeKeyAndOrderFront(None);
        Ok(())
    }

    pub fn maximize(&self, id: u32) -> napi::Result<()> {
        let entry = self
            .windows
            .get(&id)
            .ok_or_else(|| napi::Error::from_reason(format!("Window {} not found", id)))?;
        if !entry.window.isZoomed() {
            entry.window.zoom(None);
        }
        Ok(())
    }

    pub fn minimize(&self, id: u32) -> napi::Result<()> {
        let entry = self
            .windows
            .get(&id)
            .ok_or_else(|| napi::Error::from_reason(format!("Window {} not found", id)))?;
        entry.window.miniaturize(None);
        Ok(())
    }

    pub fn unmaximize(&self, id: u32) -> napi::Result<()> {
        let entry = self
            .windows
            .get(&id)
            .ok_or_else(|| napi::Error::from_reason(format!("Window {} not found", id)))?;
        if entry.window.isZoomed() {
            entry.window.zoom(None);
        }
        Ok(())
    }

    /// Reload the current page in the webview.
    pub fn reload(&mut self, id: u32) -> napi::Result<()> {
        let entry = self
            .windows
            .get(&id)
            .ok_or_else(|| napi::Error::from_reason(format!("Window {} not found", id)))?;
        match &entry.loaded_content {
            Some(LoadedContent::Html(html)) => {
                let html = html.clone();
                self.load_html(id, &html)
            }
            _ => {
                // URL or no content — use native reload
                unsafe {
                    let _: () = msg_send![&entry.webview, reload];
                }
                Ok(())
            }
        }
    }

    /// Pump the macOS event loop: process all pending events without blocking.
    pub fn pump_events(&mut self) {
        unsafe {
            let app = NSApplication::sharedApplication(self.mtm);
            loop {
                let event: Option<Retained<NSEvent>> = app
                    .nextEventMatchingMask_untilDate_inMode_dequeue(
                        NSEventMask::Any,
                        Some(&NSDate::distantPast()),
                        NSDefaultRunLoopMode,
                        true,
                    );
                match event {
                    Some(evt) => {
                        // Intercept Cmd+R to reload the focused webview
                        if !self.handle_key_shortcut(&evt) {
                            app.sendEvent(&evt);
                        }
                    }
                    None => break,
                }
            }
        }
    }

    /// Check for keyboard shortcuts and handle them.
    /// Returns `true` if the event was consumed.
    fn handle_key_shortcut(&mut self, event: &NSEvent) -> bool {
        unsafe {
            // NSEventType::KeyDown == 10
            let event_type: usize = msg_send![event, type];
            if event_type != 10 {
                return false;
            }

            let modifier_flags: usize = msg_send![event, modifierFlags];
            let key_code: u16 = msg_send![event, keyCode];

            // NSEventModifierFlagCommand = 1 << 20 = 0x100000
            let cmd_pressed = (modifier_flags & 0x100000) != 0;
            // Exclude Shift(0x20000), Ctrl(0x40000), Alt/Opt(0x80000)
            let other_modifiers = (modifier_flags & 0xE0000) != 0;

            // keyCode 15 = 'R' on macOS (hardware virtual key code)
            if cmd_pressed && !other_modifiers && key_code == 15 {
                let app = NSApplication::sharedApplication(self.mtm);
                if let Some(key_window) = app.keyWindow() {
                    // Find which managed window is focused
                    let target_id = self.windows.iter().find_map(|(id, entry)| {
                        if *entry.window == *key_window {
                            Some(*id)
                        } else {
                            None
                        }
                    });
                    if let Some(id) = target_id {
                        let _ = self.reload(id);
                        // Defer the on_reload event to be flushed after pump_events
                        crate::window_manager::PENDING_RELOADS.with(|p| {
                            p.borrow_mut().push(id);
                        });
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Process a command.
    pub fn process_command(
        &mut self,
        cmd: Command,
        event_handlers: &mut std::collections::HashMap<u32, WindowEventHandlers>,
    ) -> napi::Result<()> {
        match cmd {
            Command::CreateWindow { id, options } => {
                let handlers = event_handlers
                    .get_mut(&id)
                    .ok_or_else(|| napi::Error::from_reason("Handler entry missing"))?;
                self.create_window(id, &options, handlers)
            }
            Command::LoadURL { id, url } => self.load_url(id, &url),
            Command::LoadHTML { id, html } => self.load_html(id, &html),
            Command::EvaluateJS { id, script } => self.evaluate_js(id, &script),
            Command::SetTitle { id, title } => self.set_title(id, &title),
            Command::SetSize { id, width, height } => self.set_size(id, width, height),
            Command::SetMinSize { id, width, height } => self.set_min_size(id, width, height),
            Command::SetMaxSize { id, width, height } => self.set_max_size(id, width, height),
            Command::SetPosition { id, x, y } => self.set_position(id, x, y),
            Command::SetResizable { id, resizable } => self.set_resizable(id, resizable),
            Command::SetDecorations { id, decorations } => self.set_decorations(id, decorations),
            Command::SetAlwaysOnTop { id, always_on_top } => {
                self.set_always_on_top(id, always_on_top)
            }
            Command::Show { id } => self.show(id),
            Command::Hide { id } => self.hide(id),
            Command::Close { id } => self.close(id),
            Command::Focus { id } => self.focus(id),
            Command::Maximize { id } => self.maximize(id),
            Command::Minimize { id } => self.minimize(id),
            Command::Unmaximize { id } => self.unmaximize(id),
            Command::Reload { id } => {
                let result = self.reload(id);
                if result.is_ok() {
                    if let Some(handlers) = event_handlers.get(&id) {
                        if let Some(ref cb) = handlers.on_reload {
                            cb.call((), ThreadsafeFunctionCallMode::NonBlocking);
                        }
                    }
                }
                result
            }
        }
    }
}
