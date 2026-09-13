// The bundled page's module script. The conformance case reads
// `document.title` and `globalThis.__wateruiWasm` once the page has loaded.
document.title =
  location.origin + "|" + isSecureContext + "|" + crossOriginIsolated;

WebAssembly.instantiateStreaming(fetch("app.wasm"))
  .then(({ instance }) => {
    globalThis.__wateruiWasm = instance instanceof WebAssembly.Instance;
  })
  .catch((error) => {
    globalThis.__wateruiWasm = String(error);
  });
