// The mirrored-state declarations.
//
// `StateDeclarations::seed_script` applies this function to `{ rules, fields }`:
// the admission rules as `OriginRule` tokens, and `[key, writable]` for every
// key. It carries no value. Injected text can reach a renderer process the
// admission policy refuses, so the values a document mirrors arrive only as the
// reply to its own first pull; until that reply lands, `waterui.ready` is
// pending and reading a key throws.
//
// The first statement is the origin guard. Some engines inject document-start
// scripts into every document they load, whatever its origin; a document the
// admission policy refuses must not start a pull, even when it has defined the
// bridge's globals itself. The guard runs before any page script exists, so
// `location` and `top` are still the engine's own.
(function (seed) {
  if (
    !(function (rules) {
      if (globalThis.top !== globalThis) {
        return false;
      }
      for (var index = 0; index < rules.length; index += 1) {
        var rule = rules[index];
        if (rule === "*") {
          return true;
        }
        if (rule === "file:" ? globalThis.location.protocol === "file:" : rule === globalThis.location.origin) {
          return true;
        }
      }
      return false;
    })(seed.rules)
  ) {
    return;
  }
  for (var index = 0; index < seed.fields.length; index += 1) {
    var field = seed.fields[index];
    globalThis.__wateruiState.declare(field[0], field[1]);
  }
  globalThis.__wateruiState.start(seed.fields.length > 0);
})
