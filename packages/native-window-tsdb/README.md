# @fcannizzaro/native-window-tsdb

[![npm](https://img.shields.io/npm/v/@fcannizzaro/native-window-tsdb)](https://www.npmjs.com/package/@fcannizzaro/native-window-tsdb)

> [!WARNING]
> This project is in **alpha**. APIs may change without notice.

[TanStack DB](https://tanstack.com/db) collection adapter for [native-window](https://github.com/fcannizzaro/native-window) IPC. Sync data between the host process and webview collections over the native IPC bridge.

## Install

```bash
bun add @fcannizzaro/native-window-tsdb
```

## Overview

Two entry points for the two sides of the bridge:

| Entry | Side | Export |
|-------|------|--------|
| `@fcannizzaro/native-window-tsdb` | Host (Bun/Node) | `createDataSource` |
| `@fcannizzaro/native-window-tsdb/client` | Webview | `nativeWindowCollectionOptions` |

## Host side (Bun/Node)

Create a data source and sync data to the webview:

```ts
import { createDataSource } from "@fcannizzaro/native-window-tsdb";

const ds = createDataSource(win, {
  channel: "todos",
  getKey: (item) => item.id,
});

// Single operations
ds.insert({ id: 1, text: "Buy milk", done: false });
ds.update({ id: 1, text: "Buy milk", done: true });
ds.delete({ id: 1 });

// Batch multiple operations in a single message
ds.batch((b) => {
  b.insert({ id: 1, text: "Buy milk", done: false });
  b.insert({ id: 2, text: "Walk dog", done: false });
  b.update({ id: 1, text: "Buy milk", done: true });
});

// Send a full snapshot (replaces all data)
ds.sync([
  { id: 1, text: "Buy milk", done: true },
  { id: 2, text: "Walk dog", done: false },
]);
```

## Webview side (TanStack DB collection)

Use `nativeWindowCollectionOptions` with TanStack DB's `createCollection`:

```ts
import { createCollection } from "@tanstack/db";
import { nativeWindowCollectionOptions } from "@fcannizzaro/native-window-tsdb/client";

const todos = createCollection(
  nativeWindowCollectionOptions({
    id: "todos",
    channel: "todos",
    getKey: (item) => item.id,
  }),
);
```

Works with `useLiveQuery` from `@tanstack/react-db`:

```tsx
import { useLiveQuery } from "@tanstack/react-db";

function TodoList() {
  const todos = useLiveQuery((q) => q.from({ todos }).many());

  return (
    <ul>
      {todos.map((t) => (
        <li key={t.id}>{t.text}</li>
      ))}
    </ul>
  );
}
```

## Wire Protocol

Operations use a single-letter discriminated union over the `{ $ch, p }` IPC envelope:

| Type | `t` | Payload |
|------|-----|---------|
| Insert | `"i"` | `{ t: "i", k: TKey, d: T }` |
| Update | `"u"` | `{ t: "u", k: TKey, d: T }` |
| Delete | `"d"` | `{ t: "d", k: TKey }` |
| Snapshot | `"s"` | `{ t: "s", d: T[] }` |
| Batch | `"b"` | `{ t: "b", o: SyncOp[] }` |

## Documentation

Full documentation at [native.fcannizzaro.com](https://native-window.fcannizzaro.com)

## License

MIT
