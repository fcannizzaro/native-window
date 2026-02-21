#[macro_use]
extern crate napi_derive;

mod events;
mod options;
mod platform;
mod runtime;
mod window;
mod window_manager;

// Re-export runtime functions so napi picks them up
pub use runtime::*;

use napi::threadsafe_function::ThreadsafeFunctionCallMode;
use window_manager::{
    is_origin_trusted, with_manager, PENDING_BLURS, PENDING_CLOSES, PENDING_FOCUSES,
    PENDING_MESSAGES, PENDING_MOVES, PENDING_NAVIGATION_BLOCKED, PENDING_PAGE_LOADS,
    PENDING_RELOADS, PENDING_RESIZE_CALLBACKS,
};

/// Initialize the native window system.
/// Must be called once before creating any windows.
#[napi]
pub fn init() -> napi::Result<()> {
    with_manager(|mgr| {
        if mgr.initialized {
            return Ok(());
        }

        #[cfg(target_os = "macos")]
        {
            mgr.platform = Some(platform::macos::MacOSPlatform::new()?);
        }

        #[cfg(target_os = "windows")]
        {
            mgr.platform = Some(platform::windows::WindowsPlatform::new()?);
        }

        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            return Err(napi::Error::from_reason(
                "Unsupported platform. Only macOS and Windows are supported.",
            ));
        }

        mgr.initialized = true;
        Ok(())
    })
}

/// Process pending native UI events and execute queued commands.
/// Call this periodically (e.g., every 16ms via setInterval) to keep
/// the native windows responsive.
#[napi]
pub fn pump_events() -> napi::Result<()> {
    // ── macOS path ──────────────────────────────────────────────
    // macOS uses a single with_manager borrow for the entire function.
    // Delegate callbacks use try_borrow() and defer to PENDING_* buffers.
    #[cfg(target_os = "macos")]
    {
        with_manager(|mgr| {
            if !mgr.initialized {
                return Err(napi::Error::from_reason(
                    "Native window system not initialized. Call init() first.",
                ));
            }

            let commands = mgr.drain_commands();
            for cmd in commands {
                if let Some(ref mut platform) = mgr.platform {
                    platform.process_command(cmd, &mut mgr.event_handlers)?;
                }
            }

            if let Some(ref mut platform) = mgr.platform {
                platform.pump_events();
            }

            // Flush deferred callbacks
            flush_pending_callbacks(&mgr.event_handlers);

            Ok(())
        })
    }

    // ── Windows path ────────────────────────────────────────────
    // Windows requires split borrows because:
    // 1. process_command(CreateWindow) calls init_webview2() which uses
    //    wait_for_async_operation — a blocking GetMessageA loop. The
    //    completion callback needs with_manager() to store the controller.
    // 2. pump_events() dispatches messages via DispatchMessageW, which
    //    fires wnd_proc callbacks that also need MANAGER access.
    //
    // By temporarily extracting platform + event_handlers from MANAGER,
    // Phase 2 runs without holding the MANAGER borrow, so callbacks
    // inside wait_for_async_operation and DispatchMessageW can access
    // MANAGER directly via with_manager().
    #[cfg(target_os = "windows")]
    {
        // Phase 1: drain commands and temporarily extract state
        let (commands, mut platform, mut event_handlers) = with_manager(|mgr| {
            if !mgr.initialized {
                return Err(napi::Error::from_reason(
                    "Native window system not initialized. Call init() first.",
                ));
            }
            Ok((
                mgr.drain_commands(),
                mgr.platform.take(),
                std::mem::take(&mut mgr.event_handlers),
            ))
        })?;

        // Phase 2: process commands + pump OS messages (MANAGER not borrowed)
        // Callbacks inside wait_for_async_operation and DispatchMessageW can
        // now call with_manager() successfully.
        let result = if let Some(ref mut plat) = platform {
            let mut first_err: Option<napi::Error> = None;
            for cmd in commands {
                if let Err(e) = plat.process_command(cmd, &mut event_handlers) {
                    eprintln!("[native-window] Command failed: {}", e);
                    if first_err.is_none() {
                        first_err = Some(e);
                    }
                    // Continue processing remaining commands
                }
            }

            plat.pump_events();

            // Flush deferred SetBounds (from WM_SIZE during CreateWindowExW)
            plat.flush_deferred();

            match first_err {
                Some(e) => Err(e),
                None => Ok(()),
            }
        } else {
            Ok(())
        };

        // Phase 3: put state back and flush deferred callbacks
        with_manager(|mgr| {
            mgr.platform = platform;
            mgr.event_handlers = event_handlers;

            flush_pending_callbacks(&mgr.event_handlers);
        });

        result
    }
}

/// Flush all pending callback buffers that were deferred during pump_events.
fn flush_pending_callbacks(
    event_handlers: &std::collections::HashMap<u32, crate::events::WindowEventHandlers>,
) {
    // Flush any IPC messages that were deferred during pump_events
    let pending: Vec<(u32, String, String)> =
        PENDING_MESSAGES.with(|p| std::mem::take(&mut *p.borrow_mut()));
    for (window_id, message, source_url) in pending {
        // Re-check trusted origins for deferred messages
        let trusted = is_origin_trusted(window_id, &source_url);
        if !trusted {
            continue;
        }
        if let Some(handlers) = event_handlers.get(&window_id) {
            if let Some(ref cb) = handlers.on_message {
                cb.call((message, source_url), ThreadsafeFunctionCallMode::NonBlocking);
            }
        }
    }

    // Flush any close events that were deferred during pump_events
    let pending_closes: Vec<u32> =
        PENDING_CLOSES.with(|p| std::mem::take(&mut *p.borrow_mut()));
    for window_id in pending_closes {
        if let Some(handlers) = event_handlers.get(&window_id) {
            if let Some(ref cb) = handlers.on_close {
                cb.call((), ThreadsafeFunctionCallMode::NonBlocking);
            }
        }
    }

    // Flush any reload events that were deferred during pump_events (keyboard shortcuts)
    let pending_reloads: Vec<u32> =
        PENDING_RELOADS.with(|p| std::mem::take(&mut *p.borrow_mut()));
    for window_id in pending_reloads {
        if let Some(handlers) = event_handlers.get(&window_id) {
            if let Some(ref cb) = handlers.on_reload {
                cb.call((), ThreadsafeFunctionCallMode::NonBlocking);
            }
        }
    }

    // Flush any resize callback events that were deferred during pump_events
    let pending_resize_cbs: Vec<(u32, f64, f64)> =
        PENDING_RESIZE_CALLBACKS.with(|p| std::mem::take(&mut *p.borrow_mut()));
    for (window_id, width, height) in pending_resize_cbs {
        if let Some(handlers) = event_handlers.get(&window_id) {
            if let Some(ref cb) = handlers.on_resize {
                cb.call((width, height), ThreadsafeFunctionCallMode::NonBlocking);
            }
        }
    }

    // Flush any move callback events that were deferred during pump_events
    let pending_moves: Vec<(u32, f64, f64)> =
        PENDING_MOVES.with(|p| std::mem::take(&mut *p.borrow_mut()));
    for (window_id, x, y) in pending_moves {
        if let Some(handlers) = event_handlers.get(&window_id) {
            if let Some(ref cb) = handlers.on_move {
                cb.call((x, y), ThreadsafeFunctionCallMode::NonBlocking);
            }
        }
    }

    // Flush any focus events that were deferred during pump_events
    let pending_focuses: Vec<u32> =
        PENDING_FOCUSES.with(|p| std::mem::take(&mut *p.borrow_mut()));
    for window_id in pending_focuses {
        if let Some(handlers) = event_handlers.get(&window_id) {
            if let Some(ref cb) = handlers.on_focus {
                cb.call((), ThreadsafeFunctionCallMode::NonBlocking);
            }
        }
    }

    // Flush any blur events that were deferred during pump_events
    let pending_blurs: Vec<u32> =
        PENDING_BLURS.with(|p| std::mem::take(&mut *p.borrow_mut()));
    for window_id in pending_blurs {
        if let Some(handlers) = event_handlers.get(&window_id) {
            if let Some(ref cb) = handlers.on_blur {
                cb.call((), ThreadsafeFunctionCallMode::NonBlocking);
            }
        }
    }

    // Flush any page load events that were deferred during pump_events
    let pending_page_loads: Vec<(u32, String, String)> =
        PENDING_PAGE_LOADS.with(|p| std::mem::take(&mut *p.borrow_mut()));
    for (window_id, event_type, url) in pending_page_loads {
        if let Some(handlers) = event_handlers.get(&window_id) {
            if let Some(ref cb) = handlers.on_page_load {
                cb.call(
                    (event_type, url),
                    ThreadsafeFunctionCallMode::NonBlocking,
                );
            }
        }
    }

    // Flush any navigation-blocked events that were deferred during pump_events
    let pending_nav_blocked: Vec<(u32, String)> =
        PENDING_NAVIGATION_BLOCKED.with(|p| std::mem::take(&mut *p.borrow_mut()));
    for (window_id, url) in pending_nav_blocked {
        if let Some(handlers) = event_handlers.get(&window_id) {
            if let Some(ref cb) = handlers.on_navigation_blocked {
                cb.call(url, ThreadsafeFunctionCallMode::NonBlocking);
            }
        }
    }

}
