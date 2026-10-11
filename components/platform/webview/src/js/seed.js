// The mirrored-state seed.
//
// `StateRegistry::seed_script` applies this function to
// `{ rules, cursor, fields }`: the admission rules as `OriginRule` tokens, the
// cursor the values are current at, and `[key, { v, e, w }]` for every key.
//
// The first statement is the origin guard. Some engines inject document-start
// scripts into every document they load, whatever its origin, and the values
// below are the application's state; a document the admission policy refuses
// must not run them, even when it has defined the bridge's globals itself.
// The guard runs before any page script exists, so `location` and `top` are
// still the engine's own.
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
    globalThis.__wateruiState.define(field[0], field[1].v, field[1].e, field[1].w);
  }
  globalThis.__wateruiState.start(seed.cursor);
})
