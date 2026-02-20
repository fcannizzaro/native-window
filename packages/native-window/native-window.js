/* eslint-disable */
// JS loader for the native addon.
// Tries per-platform npm packages first (production), then local .node files (development).

const { platform, arch } = process;

// Per-platform npm package names (installed via optionalDependencies)
const platformPackages = {
  "darwin-arm64": "@fcannizzaro/native-window-darwin-arm64",
  "darwin-x64": "@fcannizzaro/native-window-darwin-x64",
  "win32-x64": "@fcannizzaro/native-window-win32-x64-msvc",
  "win32-arm64": "@fcannizzaro/native-window-win32-arm64-msvc",
};

// Local .node file names (produced by `napi build --platform`)
const platformFiles = {
  "darwin-arm64": "native-window.darwin-arm64.node",
  "darwin-x64": "native-window.darwin-x64.node",
  "win32-x64": "native-window.win32-x64-msvc.node",
  "win32-arm64": "native-window.win32-arm64-msvc.node",
};

const key = `${platform}-${arch}`;
let nativeBinding;

// 1. Try per-platform npm package (installed via optionalDependencies)
const packageName = platformPackages[key];
if (packageName) {
  try {
    nativeBinding = require(packageName);
  } catch {}
}

// 2. Try local .node file (produced by `napi build` without --platform)
if (!nativeBinding) {
  try {
    nativeBinding = require("./native-window.node");
  } catch {}
}

// 3. Try local platform-specific .node file (produced by `napi build --platform`)
if (!nativeBinding) {
  const file = platformFiles[key];
  if (file) {
    try {
      nativeBinding = require(`./${file}`);
    } catch {}
  }
}

if (!nativeBinding) {
  throw new Error(
    `Failed to load native binding for platform: ${key}. ` +
    `Ensure the correct platform package is installed or the .node file exists.`
  );
}

module.exports = nativeBinding;
