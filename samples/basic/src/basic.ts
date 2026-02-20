/**
 * Basic example: raw IPC with native-window.
 *
 * @security **Example only.** Do not use in production without proper input
 * validation and sanitization. This sample echoes messages for demonstration
 * purposes; a real application should validate all incoming data.
 *
 * Run: bun samples/src/basic.ts
 */
import { NativeWindow } from "@fcannizzaro/native-window";

// Create a window (init + event pump start automatically)
const win = new NativeWindow({
  title: "Basic Demo",
  width: 1024,
  height: 768,
  decorations: true,
  devtools: true, // disable in production
});

// Load HTML content with raw IPC bridge
win.loadHtml(`
<!DOCTYPE html>
<html>
<head>
  <meta charset="utf-8">
  <style>
    * { margin: 0; padding: 0; box-sizing: border-box; }
    body {
      font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
      background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
      color: white;
      display: flex;
      flex-direction: column;
      align-items: center;
      justify-content: center;
      height: 100vh;
      gap: 20px;
    }
    h1 { font-size: 2.5em; text-shadow: 0 2px 4px rgba(0,0,0,0.3); }
    p { font-size: 1.2em; opacity: 0.9; }
    button {
      padding: 12px 24px;
      font-size: 1.1em;
      border: 2px solid white;
      background: transparent;
      color: white;
      border-radius: 8px;
      cursor: pointer;
      transition: all 0.2s;
    }
    button:hover { background: white; color: #764ba2; }
    #messages {
      margin-top: 20px;
      padding: 16px;
      background: rgba(0,0,0,0.2);
      border-radius: 8px;
      min-width: 300px;
      max-height: 200px;
      overflow-y: auto;
    }
    .msg { padding: 4px 0; font-family: monospace; }
  </style>
</head>
<body>
  <h1>Native Webview</h1>
  <p>Running inside a native OS window via native-window</p>
  <button onclick="sendMessage()">Send Message to Bun</button>
  <button onclick="sendCounter()">Send Counter</button>
  <div id="messages"><div class="msg">Ready.</div></div>

  <script>
    let counter = 0;

    function sendMessage() {
      window.ipc.postMessage('Hello from the webview!');
      log('Sent: Hello from the webview!');
    }

    function sendCounter() {
      counter++;
      window.ipc.postMessage(JSON.stringify({ type: 'counter', value: counter }));
      log('Sent counter: ' + counter);
    }

    // Handle messages from Bun/Node
    window.__native_message__ = function(msg) {
      log('Received from Bun: ' + msg);
    };

    function log(text) {
      const div = document.getElementById('messages');
      const el = document.createElement('div');
      el.className = 'msg';
      el.textContent = '> ' + text;
      div.appendChild(el);
      div.scrollTop = div.scrollHeight;
    }
  </script>
</body>
</html>
`);

// Handle messages from the webview
win.onMessage((message: string) => {
  console.log("[Bun] Received from webview:", message);

  // Sanitize before echoing: truncate to a safe length and strip HTML tags.
  // In production, use a proper validation/sanitization library.
  const MAX_LENGTH = 1024;
  const sanitized = message
    .slice(0, MAX_LENGTH)
    .replace(/<[^>]*>/g, "");

  win.postMessage(`Echo: ${sanitized}`);
});

// Handle window close
win.onClose(() => {
  console.log("[Bun] Window closed");
  process.exit(0);
});

console.log("[Bun] Native window created. Close the window to exit.");
