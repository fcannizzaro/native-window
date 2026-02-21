use std::collections::HashMap;

use napi::threadsafe_function::ThreadsafeFunctionCallMode;

use crate::events::WindowEventHandlers;
use crate::options::WindowOptions;
use crate::window_manager::Command;

/// Maximum IPC message size in bytes (10 MB). Messages exceeding this
/// are silently dropped to prevent memory exhaustion from the webview.
const MAX_MESSAGE_SIZE: usize = 10 * 1024 * 1024;

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::*;

use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::{
    CreateCoreWebView2EnvironmentCompletedHandler,
    CreateCoreWebView2ControllerCompletedHandler,
};

/// Tracks the last loaded content for reliable reload.
enum LoadedContent {
    Url,
    Html(String),
}

/// A window entry containing the native window handle and webview controller.
struct WindowEntry {
    hwnd: HWND,
    controller: Option<ICoreWebView2Controller>,
    webview: Option<ICoreWebView2>,
    /// Last loaded content for reload support.
    loaded_content: Option<LoadedContent>,
    /// Optional Content-Security-Policy to inject at document start.
    csp: Option<String>,
    /// Whether devtools are enabled for this window.
    devtools: bool,
}

/// Windows platform state.
pub struct WindowsPlatform {
    windows: HashMap<u32, WindowEntry>,
    class_registered: bool,
    /// Maps HWND to window id for the window procedure.
    hwnd_to_id: HashMap<isize, u32>,
}

// Store a global reference so the window proc can look up IDs.
// This is safe because we only access it from the main thread.
thread_local! {
    static HWND_MAP: std::cell::RefCell<HashMap<isize, u32>> = std::cell::RefCell::new(HashMap::new());
    /// Deferred SetBounds calls: (window_id, hwnd as isize).
    static PENDING_RESIZES: std::cell::RefCell<Vec<(u32, isize)>> = std::cell::RefCell::new(Vec::new());
    /// Temporary storage for WebView2 init results passed from the completion callback
    /// back to init_webview2(). Drained immediately after wait_for_async_operation returns.
    static WEBVIEW_INIT_RESULT: std::cell::RefCell<Option<(ICoreWebView2Controller, ICoreWebView2)>> = std::cell::RefCell::new(None);
    /// Window IDs with pending programmatic Navigate/NavigateToString calls.
    /// The NavigationStarting handler checks and removes entries to skip
    /// scheme blocking for our own navigations.
    static PROGRAMMATIC_NAV: std::cell::RefCell<std::collections::HashSet<u32>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
}

impl WindowsPlatform {
    pub fn new() -> napi::Result<Self> {
        // Initialize COM for WebView2
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .ok()
                .map_err(|e| napi::Error::from_reason(format!("COM init failed: {}", e)))?;
        }

        Ok(Self {
            windows: HashMap::new(),
            class_registered: false,
            hwnd_to_id: HashMap::new(),
        })
    }

    fn ensure_class_registered(&mut self) -> napi::Result<()> {
        if self.class_registered {
            return Ok(());
        }

        unsafe {
            let hinstance = GetModuleHandleW(None)
                .map_err(|e| napi::Error::from_reason(format!("GetModuleHandle failed: {}", e)))?;

            let class_name = w!("NativeWindowClass");

            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(Self::wnd_proc),
                hInstance: hinstance.into(),
                hCursor: LoadCursorW(None, IDC_ARROW)
                    .unwrap_or_default(),
                hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as _),
                lpszClassName: class_name,
                ..Default::default()
            };

            RegisterClassExW(&wc);
            self.class_registered = true;
        }

        Ok(())
    }

    unsafe extern "system" fn wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_DESTROY => {
                HWND_MAP.with(|map| {
                    let mut map = map.borrow_mut();
                    if let Some(id) = map.remove(&(hwnd.0 as isize)) {
                        use crate::window_manager::{MANAGER, PENDING_CLOSES};
                        MANAGER.with(|m| {
                            match m.try_borrow() {
                                Ok(mgr) if mgr.platform.is_some() => {
                                    if let Some(handlers) = mgr.event_handlers.get(&id) {
                                        if let Some(ref cb) = handlers.on_close {
                                            cb.call((), ThreadsafeFunctionCallMode::NonBlocking);
                                        }
                                    }
                                }
                                _ => {
                                    // MANAGER is borrowed or state is temporarily extracted — defer
                                    PENDING_CLOSES.with(|p| p.borrow_mut().push(id));
                                }
                            }
                        });
                    }
                });
                LRESULT(0)
            }
            WM_SIZE => {
                let width = (lparam.0 & 0xFFFF) as f64;
                let height = ((lparam.0 >> 16) & 0xFFFF) as f64;

                HWND_MAP.with(|map| {
                    let map = map.borrow();
                    if let Some(&id) = map.get(&(hwnd.0 as isize)) {
                        use crate::window_manager::{MANAGER, PENDING_RESIZE_CALLBACKS};
                        MANAGER.with(|m| {
                            match m.try_borrow() {
                                Ok(mgr) if mgr.platform.is_some() => {
                                    // Resize webview controller immediately
                                    if let Some(ref platform) = mgr.platform {
                                        if let Some(entry) = platform.windows.get(&id) {
                                            if let Some(ref controller) = entry.controller {
                                                let mut rect = RECT::default();
                                                let _ = GetClientRect(hwnd, &mut rect);
                                                let _ = controller.SetBounds(rect);
                                            }
                                        }
                                    }
                                    // Fire resize callback immediately
                                    if let Some(handlers) = mgr.event_handlers.get(&id) {
                                        if let Some(ref cb) = handlers.on_resize {
                                            cb.call(
                                                (width, height),
                                                ThreadsafeFunctionCallMode::NonBlocking,
                                            );
                                        }
                                    }
                                }
                                _ => {
                                    // MANAGER is borrowed or state is temporarily extracted — defer
                                    PENDING_RESIZES.with(|p| {
                                        p.borrow_mut().push((id, hwnd.0 as isize));
                                    });
                                    PENDING_RESIZE_CALLBACKS.with(|p| {
                                        p.borrow_mut().push((id, width, height));
                                    });
                                }
                            }
                        });
                    }
                });
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_MOVE => {
                let x = (lparam.0 & 0xFFFF) as i16 as f64;
                let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as f64;

                HWND_MAP.with(|map| {
                    let map = map.borrow();
                    if let Some(&id) = map.get(&(hwnd.0 as isize)) {
                        use crate::window_manager::{MANAGER, PENDING_MOVES};
                        MANAGER.with(|m| {
                            match m.try_borrow() {
                                Ok(mgr) if mgr.platform.is_some() => {
                                    if let Some(handlers) = mgr.event_handlers.get(&id) {
                                        if let Some(ref cb) = handlers.on_move {
                                            cb.call((x, y), ThreadsafeFunctionCallMode::NonBlocking);
                                        }
                                    }
                                }
                                _ => {
                                    // MANAGER is borrowed or state is temporarily extracted — defer
                                    PENDING_MOVES.with(|p| p.borrow_mut().push((id, x, y)));
                                }
                            }
                        });
                    }
                });
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_SETFOCUS => {
                HWND_MAP.with(|map| {
                    let map = map.borrow();
                    if let Some(&id) = map.get(&(hwnd.0 as isize)) {
                        use crate::window_manager::{MANAGER, PENDING_FOCUSES};
                        MANAGER.with(|m| {
                            match m.try_borrow() {
                                Ok(mgr) if mgr.platform.is_some() => {
                                    if let Some(handlers) = mgr.event_handlers.get(&id) {
                                        if let Some(ref cb) = handlers.on_focus {
                                            cb.call((), ThreadsafeFunctionCallMode::NonBlocking);
                                        }
                                    }
                                }
                                _ => {
                                    // MANAGER is borrowed or state is temporarily extracted — defer
                                    PENDING_FOCUSES.with(|p| p.borrow_mut().push(id));
                                }
                            }
                        });
                    }
                });
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_KILLFOCUS => {
                HWND_MAP.with(|map| {
                    let map = map.borrow();
                    if let Some(&id) = map.get(&(hwnd.0 as isize)) {
                        use crate::window_manager::{MANAGER, PENDING_BLURS};
                        MANAGER.with(|m| {
                            match m.try_borrow() {
                                Ok(mgr) if mgr.platform.is_some() => {
                                    if let Some(handlers) = mgr.event_handlers.get(&id) {
                                        if let Some(ref cb) = handlers.on_blur {
                                            cb.call((), ThreadsafeFunctionCallMode::NonBlocking);
                                        }
                                    }
                                }
                                _ => {
                                    // MANAGER is borrowed or state is temporarily extracted — defer
                                    PENDING_BLURS.with(|p| p.borrow_mut().push(id));
                                }
                            }
                        });
                    }
                });
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    pub fn create_window(
        &mut self,
        id: u32,
        options: &WindowOptions,
        _handlers: &mut WindowEventHandlers,
    ) -> napi::Result<()> {
        self.ensure_class_registered()?;

        let width = options.width.unwrap_or(800.0) as i32;
        let height = options.height.unwrap_or(600.0) as i32;
        let x = options.x.map(|v| v as i32).unwrap_or(CW_USEDEFAULT);
        let y = options.y.map(|v| v as i32).unwrap_or(CW_USEDEFAULT);
        let title = options.title.as_deref().unwrap_or("");
        let decorations = options.decorations.unwrap_or(true);
        let resizable = options.resizable.unwrap_or(true);
        let visible = options.visible.unwrap_or(true);
        let always_on_top = options.always_on_top.unwrap_or(false);
        let devtools = options.devtools.unwrap_or(false);

        let mut style = WS_OVERLAPPEDWINDOW;
        if !decorations {
            style = WS_POPUP | WS_SYSMENU;
        }
        if !resizable {
            style &= !WS_THICKFRAME & !WS_MAXIMIZEBOX;
        }
        if visible {
            style |= WS_VISIBLE;
        }

        let title_wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();

        let hwnd = unsafe {
            CreateWindowExW(
                if always_on_top {
                    WS_EX_TOPMOST
                } else {
                    WINDOW_EX_STYLE::default()
                },
                w!("NativeWindowClass"),
                PCWSTR(title_wide.as_ptr()),
                style,
                x,
                y,
                width,
                height,
                None,
                None,
                GetModuleHandleW(None).unwrap_or_default(),
                None,
            )
            .map_err(|e| napi::Error::from_reason(format!("CreateWindow failed: {}", e)))?
        };

        // Store HWND -> id mapping
        HWND_MAP.with(|map| {
            map.borrow_mut().insert(hwnd.0 as isize, id);
        });
        self.hwnd_to_id.insert(hwnd.0 as isize, id);

        // Store the entry (webview will be created asynchronously)
        self.windows.insert(
            id,
            WindowEntry {
                hwnd,
                controller: None,
                webview: None,
                loaded_content: None,
                csp: options.csp.clone(),
                devtools,
            },
        );

        // Create WebView2 environment and controller
        self.init_webview2(id)?;

        Ok(())
    }

    fn init_webview2(&mut self, id: u32) -> napi::Result<()> {
        let entry = self
            .windows
            .get(&id)
            .ok_or_else(|| napi::Error::from_reason(format!("Window {} not found", id)))?;
        let hwnd = entry.hwnd;
        let csp = entry.csp.clone();
        let devtools = entry.devtools;

        unsafe {
            CreateCoreWebView2EnvironmentCompletedHandler::wait_for_async_operation(
                // Launch the async environment creation
                Box::new(|handler| {
                    CreateCoreWebView2Environment(&handler)?;
                    Ok(())
                }),
                // Handle environment creation completion
                Box::new(move |error_code, env| {
                    error_code?;
                    let env = env.ok_or_else(|| windows::core::Error::from(E_FAIL))?;

                    CreateCoreWebView2ControllerCompletedHandler::wait_for_async_operation(
                        // Launch the async controller creation
                        Box::new(move |handler| {
                            env.CreateCoreWebView2Controller(hwnd, &handler)?;
                            Ok(())
                        }),
                        // Handle controller creation completion
                        Box::new(move |error_code, controller| {
                            error_code?;
                            let controller = controller
                                .ok_or_else(|| windows::core::Error::from(E_FAIL))?;

                            // Resize to fill the window
                            let mut rect = RECT::default();
                            GetClientRect(hwnd, &mut rect)?;
                            controller.SetBounds(rect)?;
                            controller.SetIsVisible(true)?;

                            // Set up IPC message handler
                            let webview = controller.CoreWebView2()?;

                            // Apply WebView2 settings
                            if let Ok(settings) = webview.Settings() {
                                let _ = settings.SetAreDevToolsEnabled(devtools);

                                // Harden WebView2 surface
                                let _ = settings.SetAreDefaultContextMenusEnabled(false);
                                let _ = settings.SetIsStatusBarEnabled(false);
                                let _ = settings.SetIsBuiltInErrorPageEnabled(false);
                            }

                            // Add web message received handler
                            let mut token = std::mem::zeroed();
                            let _ = webview.add_WebMessageReceived(
                                &webview2_com::WebMessageReceivedEventHandler::create(
                                    Box::new(move |_webview, args| {
                                        if let Some(args) = args {
                                            let mut message = PWSTR::null();
                                            args.TryGetWebMessageAsString(&mut message)?;
                                            let msg = message.to_string()?;
                                            CoTaskMemFree(Some(message.0 as *const _));

                                            // Drop oversized messages to prevent memory exhaustion
                                            if msg.len() > MAX_MESSAGE_SIZE {
                                                return Ok(());
                                            }

                                            // Extract source URL from the event args
                                            let source_url = {
                                                let mut source = PWSTR::null();
                                                match args.Source(&mut source) {
                                                    Ok(()) => {
                                                        let url = source.to_string().unwrap_or_default();
                                                        if !source.is_null() {
                                                            CoTaskMemFree(Some(source.0 as *const _));
                                                        }
                                                        url
                                                    }
                                                    Err(_) => String::new(),
                                                }
                                            };

                                            // Check trusted origins (defense-in-depth)
                                            if !crate::window_manager::is_origin_trusted(id, &source_url) {
                                                return Ok(());
                                            }

                                            // Fire message callback
                                            crate::window_manager::MANAGER.with(|m| {
                                                match m.try_borrow() {
                                                    Ok(mgr) if mgr.platform.is_some() => {
                                                        if let Some(handlers) =
                                                            mgr.event_handlers.get(&id)
                                                        {
                                                            if let Some(ref cb) = handlers.on_message {
                                                                cb.call(
                                                                    (msg, source_url),
                                                                    ThreadsafeFunctionCallMode::NonBlocking,
                                                                );
                                                            }
                                                        }
                                                    }
                                                    _ => {
                                                        // MANAGER is borrowed or state is temporarily extracted — defer
                                                        crate::window_manager::PENDING_MESSAGES.with(|p| {
                                                            p.borrow_mut().push((id, msg, source_url));
                                                        });
                                                    }
                                                }
                                            });
                                        }
                                        Ok(())
                                    }),
                                ),
                                &mut token,
                            );

                            // Inject IPC bridge script: capture the native postMessage reference
                            // immediately and create window.ipc as a frozen, non-writable alias.
                            // Capturing early prevents page scripts from intercepting the pathway.
                            let _ = webview.AddScriptToExecuteOnDocumentCreated(
                                w!("(function(){var _post=window.chrome.webview.postMessage.bind(window.chrome.webview);Object.defineProperty(window,'ipc',{value:Object.freeze({postMessage:function(msg){_post(msg)}}),writable:false,configurable:false})})();"),
                                None,
                            );

                            // Inject Content-Security-Policy meta tag if configured
                            if let Some(ref csp_value) = csp {
                                let escaped_csp = csp_value.replace('\\', "\\\\").replace('\'', "\\'");
                                let csp_script = format!(
                                    "document.addEventListener('DOMContentLoaded',function(){{var m=document.createElement('meta');m.httpEquiv='Content-Security-Policy';m.content='{}';document.head.insertBefore(m,document.head.firstChild)}},{{once:true}});",
                                    escaped_csp
                                );
                                let csp_wide: Vec<u16> = csp_script.encode_utf16().chain(std::iter::once(0)).collect();
                                let _ = webview.AddScriptToExecuteOnDocumentCreated(
                                    PCWSTR(csp_wide.as_ptr()),
                                    None,
                                );
                            }

                            // Block dangerous URL schemes and fire onPageLoad("started")
                            let nav_start_id = id;
                            let mut nav_token = std::mem::zeroed();
                            let _ = webview.add_NavigationStarting(
                                &webview2_com::NavigationStartingEventHandler::create(
                                    Box::new(move |_webview, args| {
                                        if let Some(args) = args {
                                            let mut uri = PWSTR::null();
                                            args.Uri(&mut uri)?;
                                            let url = uri.to_string().unwrap_or_default();
                                            if !uri.is_null() {
                                                CoTaskMemFree(Some(uri.0 as *const _));
                                            }

                                            // Fire onPageLoad("started", url)
                                            crate::window_manager::PENDING_PAGE_LOADS.with(|p| {
                                                p.borrow_mut().push((
                                                    nav_start_id,
                                                    "started".to_string(),
                                                    url.clone(),
                                                ));
                                            });

                                            // Check allowedHosts restriction — applies to ALL navigations
                                            if !crate::window_manager::is_host_allowed(nav_start_id, &url) {
                                                args.SetCancel(true)?;
                                                crate::window_manager::PENDING_NAVIGATION_BLOCKED.with(|p| {
                                                    p.borrow_mut().push((nav_start_id, url));
                                                });
                                                return Ok(());
                                            }

                                            // Block dangerous URL schemes
                                            // for non-programmatic navigations only.
                                            let dominated = PROGRAMMATIC_NAV.with(|f| f.borrow_mut().remove(&nav_start_id));
                                            if !dominated {
                                                let lower = url.to_lowercase();
                                                if lower.starts_with("javascript:")
                                                    || lower.starts_with("file:")
                                                    || lower.starts_with("data:")
                                                    || lower.starts_with("blob:")
                                                {
                                                    args.SetCancel(true)?;
                                                }
                                            }
                                        }
                                        Ok(())
                                    }),
                                ),
                                &mut nav_token,
                            );

                            // Track navigation completion for onPageLoad("finished")
                            let nav_complete_id = id;
                            let mut nav_completed_token = std::mem::zeroed();
                            let _ = webview.add_NavigationCompleted(
                                &webview2_com::NavigationCompletedEventHandler::create(
                                    Box::new(move |webview, _args| {
                                        // Get current URL from the webview
                                        let url = if let Some(ref wv) = webview {
                                            let mut source = PWSTR::null();
                                            match wv.Source(&mut source) {
                                                Ok(()) => {
                                                    let u = source.to_string().unwrap_or_default();
                                                    if !source.is_null() {
                                                        CoTaskMemFree(Some(source.0 as *const _));
                                                    }
                                                    u
                                                }
                                                Err(_) => String::new(),
                                            }
                                        } else {
                                            String::new()
                                        };

                                        // Fire onPageLoad("finished", url)
                                        crate::window_manager::PENDING_PAGE_LOADS.with(|p| {
                                            p.borrow_mut().push((
                                                nav_complete_id,
                                                "finished".to_string(),
                                                url,
                                            ));
                                        });

                                        Ok(())
                                    }),
                                ),
                                &mut nav_completed_token,
                            );

                            // Pass controller and webview back via thread-local.
                            // init_webview2() will pick this up after wait_for_async_operation returns.
                            WEBVIEW_INIT_RESULT.with(|r| {
                                *r.borrow_mut() = Some((controller, webview));
                            });

                            Ok(())
                        }),
                    ).map_err(|e| windows::core::Error::new(E_FAIL, format!("{}", e)))?;
                    Ok(())
                }),
            )
            .map_err(|e| napi::Error::from_reason(format!("WebView2 init failed: {}", e)))?;
        }

        // Retrieve the controller and webview from the completion callback
        let init_result = WEBVIEW_INIT_RESULT.with(|r| r.borrow_mut().take());
        if let Some((controller, webview)) = init_result {
            if let Some(entry) = self.windows.get_mut(&id) {
                entry.controller = Some(controller);
                entry.webview = Some(webview);
            }
        }

        Ok(())
    }

    pub fn load_url(&mut self, id: u32, url: &str) -> napi::Result<()> {
        let entry = self
            .windows
            .get_mut(&id)
            .ok_or_else(|| napi::Error::from_reason(format!("Window {} not found", id)))?;

        if let Some(ref webview) = entry.webview {
            let url_wide: Vec<u16> = url.encode_utf16().chain(std::iter::once(0)).collect();
            PROGRAMMATIC_NAV.with(|f| { f.borrow_mut().insert(id); });
            unsafe {
                webview
                    .Navigate(PCWSTR(url_wide.as_ptr()))
                    .map_err(|e| napi::Error::from_reason(format!("Navigate failed: {}", e)))?;
            }
        }

        entry.loaded_content = Some(LoadedContent::Url);
        Ok(())
    }

    pub fn load_html(&mut self, id: u32, html: &str) -> napi::Result<()> {
        let entry = self
            .windows
            .get_mut(&id)
            .ok_or_else(|| napi::Error::from_reason(format!("Window {} not found", id)))?;

        if let Some(ref webview) = entry.webview {
            let html_wide: Vec<u16> = html.encode_utf16().chain(std::iter::once(0)).collect();
            // Note: NavigateToString loads content at about:blank origin.
            // This weakens same-origin isolation. Consider using
            // SetVirtualHostNameToFolderMapping for proper origin isolation
            // in a future release.
            PROGRAMMATIC_NAV.with(|f| { f.borrow_mut().insert(id); });
            unsafe {
                webview
                    .NavigateToString(PCWSTR(html_wide.as_ptr()))
                    .map_err(|e| {
                        napi::Error::from_reason(format!("NavigateToString failed: {}", e))
                    })?;
            }
        }

        entry.loaded_content = Some(LoadedContent::Html(html.to_string()));
        Ok(())
    }

    pub fn evaluate_js(&self, id: u32, script: &str) -> napi::Result<()> {
        let entry = self
            .windows
            .get(&id)
            .ok_or_else(|| napi::Error::from_reason(format!("Window {} not found", id)))?;

        if let Some(ref webview) = entry.webview {
            let script_wide: Vec<u16> = script.encode_utf16().chain(std::iter::once(0)).collect();
            unsafe {
                webview
                    .ExecuteScript(PCWSTR(script_wide.as_ptr()), None)
                    .map_err(|e| {
                        napi::Error::from_reason(format!("ExecuteScript failed: {}", e))
                    })?;
            }
        }
        Ok(())
    }

    pub fn set_title(&self, id: u32, title: &str) -> napi::Result<()> {
        let entry = self
            .windows
            .get(&id)
            .ok_or_else(|| napi::Error::from_reason(format!("Window {} not found", id)))?;
        let title_wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
        unsafe {
            SetWindowTextW(entry.hwnd, PCWSTR(title_wide.as_ptr()))
                .map_err(|e| napi::Error::from_reason(format!("SetWindowText failed: {}", e)))?;
        }
        Ok(())
    }

    pub fn set_size(&self, id: u32, width: f64, height: f64) -> napi::Result<()> {
        let entry = self
            .windows
            .get(&id)
            .ok_or_else(|| napi::Error::from_reason(format!("Window {} not found", id)))?;
        unsafe {
            SetWindowPos(
                entry.hwnd,
                None,
                0,
                0,
                width as i32,
                height as i32,
                SWP_NOMOVE | SWP_NOZORDER,
            )
            .map_err(|e| napi::Error::from_reason(format!("SetWindowPos failed: {}", e)))?;
        }
        Ok(())
    }

    pub fn set_position(&self, id: u32, x: f64, y: f64) -> napi::Result<()> {
        let entry = self
            .windows
            .get(&id)
            .ok_or_else(|| napi::Error::from_reason(format!("Window {} not found", id)))?;
        unsafe {
            SetWindowPos(
                entry.hwnd,
                None,
                x as i32,
                y as i32,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER,
            )
            .map_err(|e| napi::Error::from_reason(format!("SetWindowPos failed: {}", e)))?;
        }
        Ok(())
    }

    pub fn set_always_on_top(&self, id: u32, always_on_top: bool) -> napi::Result<()> {
        let entry = self
            .windows
            .get(&id)
            .ok_or_else(|| napi::Error::from_reason(format!("Window {} not found", id)))?;
        unsafe {
            let insert_after = if always_on_top {
                HWND_TOPMOST
            } else {
                HWND_NOTOPMOST
            };
            SetWindowPos(
                entry.hwnd,
                insert_after,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE,
            )
            .map_err(|e| napi::Error::from_reason(format!("SetWindowPos failed: {}", e)))?;
        }
        Ok(())
    }

    pub fn show(&self, id: u32) -> napi::Result<()> {
        let entry = self
            .windows
            .get(&id)
            .ok_or_else(|| napi::Error::from_reason(format!("Window {} not found", id)))?;
        unsafe {
            let _ = ShowWindow(entry.hwnd, SW_SHOW);
        }
        Ok(())
    }

    pub fn hide(&self, id: u32) -> napi::Result<()> {
        let entry = self
            .windows
            .get(&id)
            .ok_or_else(|| napi::Error::from_reason(format!("Window {} not found", id)))?;
        unsafe {
            let _ = ShowWindow(entry.hwnd, SW_HIDE);
        }
        Ok(())
    }

    pub fn close(&mut self, id: u32) -> napi::Result<()> {
        let entry = self
            .windows
            .remove(&id)
            .ok_or_else(|| napi::Error::from_reason(format!("Window {} not found", id)))?;
        HWND_MAP.with(|map| {
            map.borrow_mut().remove(&(entry.hwnd.0 as isize));
        });
        self.hwnd_to_id.remove(&(entry.hwnd.0 as isize));
        unsafe {
            let _ = DestroyWindow(entry.hwnd);
        }
        // Clean up security config stored in separate thread-locals.
        // Event handler cleanup is done by the caller (process_command)
        // because during Phase 2, MANAGER's event_handlers are extracted.
        crate::window_manager::TRUSTED_ORIGINS_MAP.with(|o| {
            o.borrow_mut().remove(&id);
        });
        crate::window_manager::ALLOWED_HOSTS_MAP.with(|h| {
            h.borrow_mut().remove(&id);
        });
        Ok(())
    }

    pub fn focus(&self, id: u32) -> napi::Result<()> {
        let entry = self
            .windows
            .get(&id)
            .ok_or_else(|| napi::Error::from_reason(format!("Window {} not found", id)))?;
        unsafe {
            let _ = SetForegroundWindow(entry.hwnd);
            let _ = SetFocus(entry.hwnd);
        }
        Ok(())
    }

    pub fn maximize(&self, id: u32) -> napi::Result<()> {
        let entry = self
            .windows
            .get(&id)
            .ok_or_else(|| napi::Error::from_reason(format!("Window {} not found", id)))?;
        unsafe {
            let _ = ShowWindow(entry.hwnd, SW_MAXIMIZE);
        }
        Ok(())
    }

    pub fn minimize(&self, id: u32) -> napi::Result<()> {
        let entry = self
            .windows
            .get(&id)
            .ok_or_else(|| napi::Error::from_reason(format!("Window {} not found", id)))?;
        unsafe {
            let _ = ShowWindow(entry.hwnd, SW_MINIMIZE);
        }
        Ok(())
    }

    pub fn unmaximize(&self, id: u32) -> napi::Result<()> {
        let entry = self
            .windows
            .get(&id)
            .ok_or_else(|| napi::Error::from_reason(format!("Window {} not found", id)))?;
        unsafe {
            let _ = ShowWindow(entry.hwnd, SW_RESTORE);
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
                if let Some(ref webview) = entry.webview {
                    unsafe {
                        webview
                            .Reload()
                            .map_err(|e| napi::Error::from_reason(format!("Reload failed: {}", e)))?;
                    }
                }
                Ok(())
            }
        }
    }

    /// Flush deferred platform operations (SetBounds) that were queued during pump_events.
    pub fn flush_deferred(&mut self) {
        // Flush deferred SetBounds calls from WM_SIZE
        let pending: Vec<(u32, isize)> =
            PENDING_RESIZES.with(|p| std::mem::take(&mut *p.borrow_mut()));
        for (id, hwnd_val) in pending {
            if let Some(entry) = self.windows.get(&id) {
                if let Some(ref controller) = entry.controller {
                    let hwnd = HWND(hwnd_val as *mut _);
                    unsafe {
                        let mut rect = RECT::default();
                        let _ = GetClientRect(hwnd, &mut rect);
                        let _ = controller.SetBounds(rect);
                    }
                }
            }
        }
    }

    /// Pump the Windows message loop: process all pending messages without blocking.
    pub fn pump_events(&mut self) {
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
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
            Command::SetMinSize { id: _, .. } => Ok(()), // TODO: implement via WM_GETMINMAXINFO
            Command::SetMaxSize { id: _, .. } => Ok(()), // TODO: implement via WM_GETMINMAXINFO
            Command::SetPosition { id, x, y } => self.set_position(id, x, y),
            Command::SetResizable { id: _, .. } => Ok(()), // TODO: toggle WS_THICKFRAME
            Command::SetDecorations { id: _, .. } => Ok(()), // TODO: toggle window styles
            Command::SetAlwaysOnTop { id, always_on_top } => {
                self.set_always_on_top(id, always_on_top)
            }
            Command::Show { id } => self.show(id),
            Command::Hide { id } => self.hide(id),
            Command::Close { id } => {
                let result = self.close(id);
                if result.is_ok() {
                    event_handlers.remove(&id);
                }
                result
            }
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
            Command::GetCookies { id, url } => {
                let entry = self
                    .windows
                    .get(&id)
                    .ok_or_else(|| napi::Error::from_reason(format!("Window {} not found", id)))?;

                if let Some(ref webview) = entry.webview {
                    if let Some(on_cookies) = event_handlers
                        .get(&id)
                        .and_then(|h| h.on_cookies.as_ref())
                    {
                        let tsfn = on_cookies.clone();
                        let uri = url.unwrap_or_default();
                        let uri_wide: Vec<u16> =
                            uri.encode_utf16().chain(std::iter::once(0)).collect();

                        unsafe {
                            let webview2: ICoreWebView2_2 = webview.cast().map_err(|e| {
                                napi::Error::from_reason(format!(
                                    "WebView2 v2 not available: {}",
                                    e
                                ))
                            })?;
                            let cookie_manager =
                                webview2.CookieManager().map_err(|e| {
                                    napi::Error::from_reason(format!(
                                        "CookieManager failed: {}",
                                        e
                                    ))
                                })?;

                            let _ = cookie_manager.GetCookies(
                                PCWSTR(uri_wide.as_ptr()),
                                &webview2_com::GetCookiesCompletedHandler::create(
                                    Box::new(move |result, cookie_list| {
                                        result?;
                                        if let Some(cookie_list) = cookie_list {
                                            let mut count = 0u32;
                                            cookie_list.Count(&mut count)?;

                                            let mut json_parts: Vec<String> =
                                                Vec::with_capacity(count as usize);

                                            for i in 0..count {
                                                let cookie =
                                                    cookie_list.GetValueAtIndex(i)?;

                                                let mut name = PWSTR::null();
                                                cookie.Name(&mut name)?;
                                                let name_str =
                                                    name.to_string().unwrap_or_default();
                                                if !name.is_null() {
                                                    CoTaskMemFree(Some(
                                                        name.0 as *const _,
                                                    ));
                                                }

                                                let mut value = PWSTR::null();
                                                cookie.Value(&mut value)?;
                                                let value_str =
                                                    value.to_string().unwrap_or_default();
                                                if !value.is_null() {
                                                    CoTaskMemFree(Some(
                                                        value.0 as *const _,
                                                    ));
                                                }

                                                let mut domain = PWSTR::null();
                                                cookie.Domain(&mut domain)?;
                                                let domain_str = domain
                                                    .to_string()
                                                    .unwrap_or_default();
                                                if !domain.is_null() {
                                                    CoTaskMemFree(Some(
                                                        domain.0 as *const _,
                                                    ));
                                                }

                                                let mut path = PWSTR::null();
                                                cookie.Path(&mut path)?;
                                                let path_str =
                                                    path.to_string().unwrap_or_default();
                                                if !path.is_null() {
                                                    CoTaskMemFree(Some(
                                                        path.0 as *const _,
                                                    ));
                                                }

                                                let mut http_only = BOOL::default();
                                                cookie.IsHttpOnly(&mut http_only)?;

                                                let mut secure = BOOL::default();
                                                cookie.IsSecure(&mut secure)?;

                                                let mut expires = 0.0f64;
                                                cookie.Expires(&mut expires)?;

                                                let mut same_site =
                                                    COREWEBVIEW2_COOKIE_SAME_SITE_KIND(0);
                                                cookie.SameSite(&mut same_site)?;
                                                let same_site_str = if same_site
                                                    == COREWEBVIEW2_COOKIE_SAME_SITE_KIND_LAX
                                                {
                                                    "lax"
                                                } else if same_site
                                                    == COREWEBVIEW2_COOKIE_SAME_SITE_KIND_STRICT
                                                {
                                                    "strict"
                                                } else {
                                                    "none"
                                                };

                                                json_parts.push(format!(
                                                    "{{\"name\":{},\"value\":{},\"domain\":{},\"path\":{},\"httpOnly\":{},\"secure\":{},\"sameSite\":\"{}\",\"expires\":{}}}",
                                                    crate::window_manager::json_escape(&name_str),
                                                    crate::window_manager::json_escape(&value_str),
                                                    crate::window_manager::json_escape(&domain_str),
                                                    crate::window_manager::json_escape(&path_str),
                                                    http_only.as_bool(),
                                                    secure.as_bool(),
                                                    same_site_str,
                                                    expires as i64,
                                                ));
                                            }

                                            let json =
                                                format!("[{}]", json_parts.join(","));
                                            tsfn.call(
                                                json,
                                                ThreadsafeFunctionCallMode::NonBlocking,
                                            );
                                        }
                                        Ok(())
                                    }),
                                ),
                            );
                        }
                    }
                }
                Ok(())
            }
        }
    }
}
