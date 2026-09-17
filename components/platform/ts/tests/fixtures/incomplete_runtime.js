// A bundle whose runtime is missing an entry: `subscribe` is not a function.
//
// The bridge must name the entry rather than fail later, when the first
// materialized signal tries to push.

globalThis.__waterui_runtime = {
  installHost: () => {},
  uninstallHost: () => {},
  mount: () => {},
  isSignal: () => false,
  isAccessor: () => false,
  read: (value) => value,
  write: () => {},
  subscribe: "not a function",
  toSignal: (value) => value,
  toAccessor: (value) => () => value,
  createSignal: (value) => () => value,
  createMemo: (compute) => compute,
  makeCallback: () => () => {},
  modules: {},
};
