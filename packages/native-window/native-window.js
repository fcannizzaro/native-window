/* eslint-disable */
// JS loader for the native addon.
// Tries per-platform npm packages first (production), then local .node files (development).
const { platform, arch } = process;

const platforms = {
  "darwin-arm64": {
    pkg: "@fcannizzaro/native-window-darwin-arm64",
    file: "native-window.darwin-arm64.node",
  },
  "darwin-x64": {
    pkg: "@fcannizzaro/native-window-darwin-x64",
    file: "native-window.darwin-x64.node",
  },
  "win32-x64": {
    pkg: "@fcannizzaro/native-window-win32-x64-msvc",
    file: "native-window.win32-x64-msvc.node",
  },
  "win32-arm64": {
    pkg: "@fcannizzaro/native-window-win32-arm64-msvc",
    file: "native-window.win32-arm64-msvc.node",
  },
};

const key = `${platform}-${arch}`;
const entry = platforms[key];

if (!entry) {
  throw new Error(`Unsupported platform: ${key}`);
}

const tryRequire = (id) => {
  try {
    return require(id);
  } catch {
    return null;
  }
};

const nativeBinding = tryRequire(`./${entry.file}`) ?? tryRequire(entry.pkg);

if (!nativeBinding) {
  throw new Error(
    `Failed to load native binding for platform: ${key}. ` +
    `Ensure the correct platform package is installed or the .node file exists.`,
  );
}

module.exports = nativeBinding;
