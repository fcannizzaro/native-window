// Re-export the native bindings.
// After building with `bun run build`, the native addon generates
// `native-window.${platform}.node` and a JS loader.
//
// This file provides the TypeScript entry point.

import {
  init,
  pumpEvents,
  NativeWindow as _NativeWindow,
  checkRuntime,
  ensureRuntime,
} from "./native-window.js";

export { checkRuntime, ensureRuntime };

export type { WindowOptions, RuntimeInfo } from "./native-window.js";

// ---------------------------------------------------------------------------
// Auto-init / auto-pump state
// ---------------------------------------------------------------------------

let _pump: ReturnType<typeof setInterval> | null = null;
let _windowCount = 0;

function ensureInit() {
  if (_pump) return;
  init();
  _pump = setInterval(() => {
    try {
      pumpEvents();
    } catch (e) {
      console.error("[native-window] pumpEvents() error:", e);
    }
  }, 16);
}

function stopPump() {
  if (_pump) {
    clearInterval(_pump);
    _pump = null;
  }
}

// ---------------------------------------------------------------------------
// Unsafe namespace
// ---------------------------------------------------------------------------

/**
 * Operations that execute arbitrary code in the webview context.
 * Grouped under {@link NativeWindow.unsafe} to signal injection risk.
 *
 * @security Never pass unsanitized user input to these methods.
 * Use {@link sanitizeForJs} to escape strings before embedding them in
 * script code.
 */
export interface UnsafeNamespace {
  /**
   * Evaluate arbitrary JavaScript in the webview context.
   * Fire-and-forget — there is no return value.
   * Use `postMessage`/`onMessage` to send results back.
   *
   * @security **Injection risk.** Never pass unsanitized user input directly.
   * Use {@link sanitizeForJs} to escape strings before embedding them in
   * script code.
   */
  evaluateJs(script: string): void;
}

// ---------------------------------------------------------------------------
// Cookie types
// ---------------------------------------------------------------------------

/**
 * Information about a cookie from the native cookie store.
 * Includes `HttpOnly` cookies that are invisible to `document.cookie`.
 *
 * @example
 * ```ts
 * win.onCookies((cookies) => {
 *   for (const c of cookies) {
 *     console.log(c.name, c.value, c.httpOnly);
 *   }
 * });
 * win.getCookies("https://example.com");
 * ```
 */
export interface CookieInfo {
  /** Cookie name. */
  name: string;
  /** Cookie value. */
  value: string;
  /** Domain the cookie belongs to. */
  domain: string;
  /** Path the cookie is restricted to. */
  path: string;
  /** Whether the cookie is HttpOnly (inaccessible to JS). */
  httpOnly: boolean;
  /** Whether the cookie requires HTTPS. */
  secure: boolean;
  /** SameSite policy: "none", "lax", or "strict". */
  sameSite: string;
  /** Expiry as Unix timestamp (seconds). -1 for session cookies. */
  expires: number;
}

// ---------------------------------------------------------------------------
// NativeWindow wrapper – auto-init, auto-pump, auto-stop
// ---------------------------------------------------------------------------

type WindowOptions = import("./native-window.js").WindowOptions;

/**
 * A native OS window with an embedded webview.
 *
 * Automatically initializes the native subsystem and starts pumping
 * events on first construction. Stops the pump when all windows close.
 */
export class NativeWindow {
  /** @internal */
  private _native: InstanceType<typeof _NativeWindow>;
  /** @internal */
  private _closed = false;
  /** @internal */
  private _unsafe?: UnsafeNamespace;

  constructor(options?: WindowOptions) {
    ensureInit();
    _windowCount++;
    this._native = new _NativeWindow(options);

    // Register a default close handler to track window count.
    this._native.onClose(() => this._handleClose());
  }

  /** @internal */
  private _handleClose() {
    if (this._closed) return;
    this._closed = true;
    _windowCount--;
    if (_windowCount <= 0) {
      _windowCount = 0;
      stopPump();
    }
    this._userCloseCallback?.();
  }

  /**
   * Throws if the window has been closed.
   * @internal
   */
  private _ensureOpen(): void {
    if (this._closed) {
      throw new Error("Window is closed");
    }
  }

  // ---- onClose with user callback support ----

  private _userCloseCallback?: () => void;

  /**
   * Register a handler for the window close event.
   * The pump is automatically stopped when all windows are closed.
   *
   * Calling this multiple times replaces the previous handler.
   */
  onClose(callback: () => void): void {
    if (this._userCloseCallback) {
      console.warn(
        "NativeWindow: onClose() called multiple times. The previous handler will be replaced.",
      );
    }
    this._userCloseCallback = callback;
  }

  // ---- Getters ----

  /** Unique window ID */
  get id(): number {
    return this._native.id;
  }

  // ---- Content loading ----

  loadUrl(url: string): void {
    this._ensureOpen();
    this._native.loadUrl(url);
  }

  /**
   * Load raw HTML content into the webview.
   *
   * @security **Injection risk.** Never interpolate unsanitized user input
   * into HTML strings. Use a dedicated sanitization library such as
   * [DOMPurify](https://github.com/cure53/DOMPurify) or
   * [sanitize-html](https://github.com/apostrophecms/sanitize-html) to
   * sanitize untrusted content before embedding it.
   */
  loadHtml(html: string): void {
    this._ensureOpen();
    this._native.loadHtml(html);
  }

  postMessage(message: string): void {
    this._ensureOpen();
    this._native.postMessage(message);
  }

  // ---- Unsafe operations ----

  /**
   * Namespace for operations that require extra care to avoid injection risks.
   * Methods under `unsafe` execute arbitrary code in the webview context.
   *
   * @security Never pass unsanitized user input to these methods.
   * Use {@link sanitizeForJs} to escape strings before embedding them in
   * script code.
   */
  get unsafe(): UnsafeNamespace {
    this._ensureOpen();
    if (!this._unsafe) {
      // Capture `this` to re-check _closed on each call, preventing
      // use-after-close via a previously cached UnsafeNamespace reference.
      const self = this;
      this._unsafe = {
        evaluateJs(script: string): void {
          self._ensureOpen();
          self._native.evaluateJs(script);
        },
      };
    }
    return this._unsafe;
  }

  // ---- Window control ----

  setTitle(title: string): void {
    this._ensureOpen();
    this._native.setTitle(title);
  }

  setSize(width: number, height: number): void {
    this._ensureOpen();
    this._native.setSize(width, height);
  }

  setMinSize(width: number, height: number): void {
    this._ensureOpen();
    this._native.setMinSize(width, height);
  }

  setMaxSize(width: number, height: number): void {
    this._ensureOpen();
    this._native.setMaxSize(width, height);
  }

  setPosition(x: number, y: number): void {
    this._ensureOpen();
    this._native.setPosition(x, y);
  }

  setResizable(resizable: boolean): void {
    this._ensureOpen();
    this._native.setResizable(resizable);
  }

  setDecorations(decorations: boolean): void {
    this._ensureOpen();
    this._native.setDecorations(decorations);
  }

  setAlwaysOnTop(alwaysOnTop: boolean): void {
    this._ensureOpen();
    this._native.setAlwaysOnTop(alwaysOnTop);
  }

  // ---- Window state ----

  show(): void {
    this._ensureOpen();
    this._native.show();
  }

  hide(): void {
    this._ensureOpen();
    this._native.hide();
  }

  close(): void {
    this._ensureOpen();
    this._closed = true;
    this._native.close();
  }

  focus(): void {
    this._ensureOpen();
    this._native.focus();
  }

  maximize(): void {
    this._ensureOpen();
    this._native.maximize();
  }

  minimize(): void {
    this._ensureOpen();
    this._native.minimize();
  }

  unmaximize(): void {
    this._ensureOpen();
    this._native.unmaximize();
  }

  reload(): void {
    this._ensureOpen();
    this._native.reload();
  }

  // ---- Event handlers ----

  /**
   * Register a handler for messages from the webview.
   *
   * @security **No origin filtering.** The raw `onMessage` API does not
   * enforce origin restrictions. If your webview navigates to untrusted
   * URLs, validate the `sourceUrl` parameter before processing messages.
   * For automatic origin filtering, use `createChannel()` with the
   * `trustedOrigins` option from `native-window-ipc`.
   *
   * @security **No rate limiting.** Messages from the webview are delivered
   * without throttling. A malicious page can flood the host with messages.
   * Consider implementing application-level rate limiting if loading
   * untrusted content.
   */
  onMessage(callback: (message: string, sourceUrl: string) => void): void {
    this._ensureOpen();
    this._native.onMessage(callback);
  }

  onResize(callback: (width: number, height: number) => void): void {
    this._ensureOpen();
    this._native.onResize(callback);
  }

  onMove(callback: (x: number, y: number) => void): void {
    this._ensureOpen();
    this._native.onMove(callback);
  }

  onFocus(callback: () => void): void {
    this._ensureOpen();
    this._native.onFocus(callback);
  }

  onBlur(callback: () => void): void {
    this._ensureOpen();
    this._native.onBlur(callback);
  }

  onPageLoad(
    callback: (event: "started" | "finished", url: string) => void,
  ): void {
    this._ensureOpen();
    this._native.onPageLoad(callback);
  }

  onTitleChanged(callback: (title: string) => void): void {
    this._ensureOpen();
    this._native.onTitleChanged(callback);
  }

  onReload(callback: () => void): void {
    this._ensureOpen();
    this._native.onReload(callback);
  }

  // ---- Cookie access ----

  /**
   * Validate and parse a raw cookies JSON array from the native layer.
   * Returns a cleaned {@link CookieInfo} array or `null` if the payload
   * is malformed.
   *
   * @internal
   */
  private _validateCookies(raw: string): CookieInfo[] | null {
    let parsed: unknown;
    try {
      parsed = JSON.parse(raw);
    } catch {
      return null;
    }
    if (!Array.isArray(parsed)) return null;

    const cookies: CookieInfo[] = [];
    for (const item of parsed) {
      if (typeof item !== "object" || item === null) continue;
      const obj = item as Record<string, unknown>;
      if (typeof obj.name !== "string" || typeof obj.value !== "string") {
        continue;
      }
      if (typeof obj.domain !== "string" || typeof obj.path !== "string") {
        continue;
      }
      if (typeof obj.httpOnly !== "boolean" || typeof obj.secure !== "boolean") {
        continue;
      }
      if (typeof obj.sameSite !== "string" || typeof obj.expires !== "number") {
        continue;
      }
      cookies.push({
        name: obj.name as string,
        value: obj.value as string,
        domain: obj.domain as string,
        path: obj.path as string,
        httpOnly: obj.httpOnly as boolean,
        secure: obj.secure as boolean,
        sameSite: obj.sameSite as string,
        expires: obj.expires as number,
      });
    }
    return cookies;
  }

  /**
   * Query cookies from the native cookie store.
   *
   * Returns a Promise that resolves with validated {@link CookieInfo} objects,
   * including `HttpOnly` cookies that are invisible to `document.cookie`.
   *
   * - **macOS**: Uses `WKHTTPCookieStore.getAllCookies` with client-side
   *   URL filtering (domain + path match).
   * - **Windows**: Uses `ICoreWebView2CookieManager.GetCookies` which
   *   filters by URI natively.
   *
   * @param url  If provided, only cookies matching this URL are returned.
   *             If omitted, all cookies in the webview's cookie store are returned.
   *
   * @example
   * ```ts
   * const cookies = await win.getCookies("https://example.com");
   * const session = cookies.find((c) => c.name === "session_id");
   * if (session) console.log("Session:", session.value, "HttpOnly:", session.httpOnly);
   * ```
   */
  getCookies(url?: string): Promise<CookieInfo[]> {
    this._ensureOpen();
    return new Promise((resolve, reject) => {
      this._native.onCookies((raw: string) => {
        const validated = this._validateCookies(raw);
        if (validated) {
          resolve(validated);
        } else {
          reject(new Error("Failed to parse cookie response"));
        }
      });
      this._native.getCookies(url);
    });
  }
}

// ---------------------------------------------------------------------------
// Legacy convenience helper
// ---------------------------------------------------------------------------

/**
 * Convenience function: initializes the native system and starts
 * pumping events at the given interval (default 16ms ~ 60fps).
 * Returns a cleanup function that stops the pump.
 *
 * @deprecated Use `new NativeWindow()` directly — init and pump are automatic.
 */
export function run(intervalMs = 16): () => void {
  init();
  const timer = setInterval(() => {
    try {
      pumpEvents();
    } catch (e) {
      console.error("[native-window] pumpEvents() error:", e);
    }
  }, intervalMs);
  return () => clearInterval(timer);
}

// ---------------------------------------------------------------------------
// Security helpers
// ---------------------------------------------------------------------------

/**
 * Escape a string for safe embedding inside a JavaScript string literal.
 * Handles backslashes, quotes, backticks, template expressions, newlines,
 * null bytes, closing `</script>` tags, and Unicode line/paragraph
 * separators (U+2028, U+2029).
 *
 * @security Use this when interpolating untrusted input into
 * {@link NativeWindow.unsafe} `evaluateJs()` calls to prevent script injection.
 *
 * @example
 * ```ts
 * import { NativeWindow, sanitizeForJs } from "native-window";
 *
 * const userInput = 'He said "hello"\n<script>alert(1)</script>';
 * win.unsafe.evaluateJs(`display("${sanitizeForJs(userInput)}")`);
 * ```
 */
export function sanitizeForJs(input: string): string {
  return input
    .replace(/\\/g, "\\\\")
    .replace(/'/g, "\\'")
    .replace(/"/g, '\\"')
    .replace(/`/g, "\\`")
    .replace(/\$\{/g, "\\${")
    .replace(/\n/g, "\\n")
    .replace(/\r/g, "\\r")
    .replace(/\0/g, "\\0")
    .replace(/\u2028/g, "\\u2028")
    .replace(/\u2029/g, "\\u2029")
    .replace(/<\/script>/gi, "<\\/script>");
}
