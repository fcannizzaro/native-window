/* eslint-disable */
// Auto-generated type declarations for the native addon.
// These will be overwritten by `napi build` but serve as
// a reference during development.

export interface WindowOptions {
  /** Window title. Default: "" */
  title?: string;
  /** Inner width in logical pixels. Default: 800 */
  width?: number;
  /** Inner height in logical pixels. Default: 600 */
  height?: number;
  /** X position in screen coordinates */
  x?: number;
  /** Y position in screen coordinates */
  y?: number;
  /** Minimum inner width */
  minWidth?: number;
  /** Minimum inner height */
  minHeight?: number;
  /** Maximum inner width */
  maxWidth?: number;
  /** Maximum inner height */
  maxHeight?: number;
  /** Allow resizing. Default: true */
  resizable?: boolean;
  /** Show window decorations (title bar, borders). Default: true */
  decorations?: boolean;
  /** Transparent window background. Default: false */
  transparent?: boolean;
  /** Always on top of other windows. Default: false */
  alwaysOnTop?: boolean;
  /** Initially visible. Default: true */
  visible?: boolean;
  /** Enable devtools. Default: false */
  devtools?: boolean;
  /**
   * Content Security Policy to inject at document start.
   * When set, a `<meta http-equiv="Content-Security-Policy">` tag is injected
   * before any page scripts run.
   *
   * @example `"default-src 'self'; script-src 'self' 'unsafe-inline'"`
   */
  csp?: string;
  /**
   * Trusted origins for IPC messages at the native layer.
   * When set, only messages whose source URL origin matches one of these
   * entries are forwarded to the host. Messages from other origins are
   * silently dropped.
   *
   * Each entry should be a full origin string (scheme + host + optional port),
   * e.g. `"https://example.com"`. No trailing slash.
   *
   * @security Defense-in-depth. For application-level origin filtering,
   * use `trustedOrigins` in `createChannel()` from `native-window-ipc`.
   *
   * @example `["https://myapp.com", "https://cdn.myapp.com"]`
   */
  trustedOrigins?: string[];
  /**
   * Allowed hosts for navigation restriction.
   * When set and non-empty, ALL navigations (including `loadUrl()`, link
   * clicks, form submissions, and redirects) are restricted to URLs whose
   * host matches one of these patterns.
   *
   * Supports wildcard prefixes: `"*.example.com"` matches any subdomain
   * of `example.com` and `example.com` itself.
   *
   * Internal navigations (`about:blank`, `loadHtml()` content) are always
   * permitted. When unset or empty, all hosts are allowed.
   *
   * @example `["myapp.com", "*.cdn.myapp.com"]`
   */
  allowedHosts?: string[];
  /**
   * Allow the webview to access the camera when requested.
   * Default: false (all camera requests are denied).
   * @note Not yet enforced in the wry backend. The OS default (prompt user) applies.
   */
  allowCamera?: boolean;
  /**
   * Allow the webview to access the microphone when requested.
   * Default: false (all microphone requests are denied).
   * @note Not yet enforced in the wry backend. The OS default (prompt user) applies.
   */
  allowMicrophone?: boolean;
  /**
   * Allow the webview to use the File System Access API
   * (`showOpenFilePicker`, `showSaveFilePicker`, `showDirectoryPicker`).
   * Default: false (all file system access requests are denied).
   * @note Not yet enforced in the wry backend. The OS default applies.
   */
  allowFileSystem?: boolean;
  /**
   * Allow the webview to access geolocation when requested.
   * Default: false (navigator.geolocation is removed from the page).
   */
  allowGeolocation?: boolean;
}

export class NativeWindow {
  constructor(options?: WindowOptions);

  /** Unique window ID */
  readonly id: number;

  // Content loading
  loadUrl(url: string): void;
  loadHtml(html: string): void;
  evaluateJs(script: string): void;
  postMessage(message: string): void;

  // Window control
  setTitle(title: string): void;
  setSize(width: number, height: number): void;
  setMinSize(width: number, height: number): void;
  setMaxSize(width: number, height: number): void;
  setPosition(x: number, y: number): void;
  setResizable(resizable: boolean): void;
  setDecorations(decorations: boolean): void;
  setAlwaysOnTop(alwaysOnTop: boolean): void;

  // Window state
  show(): void;
  hide(): void;
  close(): void;
  focus(): void;
  maximize(): void;
  minimize(): void;
  unmaximize(): void;
  reload(): void;

  // Event handlers
  onMessage(callback: (message: string, sourceUrl: string) => void): void;
  onClose(callback: () => void): void;
  onResize(callback: (width: number, height: number) => void): void;
  onMove(callback: (x: number, y: number) => void): void;
  onFocus(callback: () => void): void;
  onBlur(callback: () => void): void;
  onPageLoad(
    callback: (event: "started" | "finished", url: string) => void,
  ): void;
  onTitleChanged(callback: (title: string) => void): void;
  onReload(callback: () => void): void;
  onNavigationBlocked(callback: (url: string) => void): void;

  // Cookie access
  getCookies(url?: string): void;
  onCookies(callback: (cookies: string) => void): void;
}

/** Initialize the native window system. Must be called once before creating any windows. */
export function init(): void;

/** Process pending native UI events. Call periodically (~16ms) to keep windows responsive. */
export function pumpEvents(): void;

/** Information about the native webview runtime. */
export interface RuntimeInfo {
  /** Whether the webview runtime is available. */
  available: boolean;
  /** The version string of the runtime, if detected. */
  version?: string;
  /** The current platform: "macos", "windows", "linux", or "unsupported". */
  platform: "macos" | "windows" | "linux" | "unsupported";
}

/**
 * Check if the native webview runtime is available.
 *
 * - **macOS**: Always returns available (WKWebView is a system framework).
 * - **Windows**: Detects WebView2 via `GetAvailableCoreWebView2BrowserVersionString`.
 * - **Linux**: Always returns available (WebKitGTK is required at build time).
 * - **Other**: Returns `{ available: false, platform: "unsupported" }`.
 */
export function checkRuntime(): RuntimeInfo;

/**
 * Ensure the native webview runtime is available, installing it if necessary.
 *
 * - **macOS**: Returns immediately (WKWebView is always available).
 * - **Windows**: If WebView2 is missing, downloads the Evergreen Bootstrapper
 *   (~2MB) from Microsoft and runs it silently. Throws on failure.
 * - **Linux**: Returns immediately (WebKitGTK is required at build time).
 * - **Other**: Throws an error.
 *
 * @security This function downloads and executes a Microsoft-signed binary
 * from the internet (Windows only). Authenticode signature verification is
 * performed before execution; unverified binaries are never run.
 *
 * **Do not call in an elevated (Administrator) context without explicit user
 * consent.** The silent installer applies system-wide. Prefer calling
 * {@link checkRuntime} first to avoid unnecessary network requests when the
 * runtime is already present.
 */
export function ensureRuntime(): RuntimeInfo;

/**
 * Escape a string for safe embedding inside a JavaScript string literal.
 * Handles backslashes, quotes, newlines, null bytes, closing `</script>` tags,
 * and Unicode line/paragraph separators (U+2028, U+2029).
 *
 * @security Use this when interpolating untrusted input into `win.unsafe.evaluateJs()` calls.
 *
 * @example
 * ```ts
 * import { sanitizeForJs } from "native-window";
 *
 * const userInput = 'He said "hello"\n<script>alert(1)</script>';
 * win.unsafe.evaluateJs(`display("${sanitizeForJs(userInput)}")`);
 * ```
 */
export function sanitizeForJs(input: string): string;
