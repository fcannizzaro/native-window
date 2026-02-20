import { createCollection } from "@tanstack/react-db";
import { nativeWindowCollectionOptions } from "@fcannizzaro/native-window-tsdb/client";

type Todo = { id: string; text: string; done: boolean };

export const todoCollection = createCollection(
  nativeWindowCollectionOptions<Todo>({
    id: "todos",
    channel: "tsdb:todos",
    getKey: (todo) => todo.id,
  }),
);
