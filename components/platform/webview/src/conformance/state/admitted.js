// What an admitted document reports to the conformance case: every value the
// mirrored `count` takes here, and every `pageshow`, with whether the document
// came back from the back/forward cache. Both travel as ordinary handler
// calls, so the case waits on the document's own word rather than on time.
//
// A document holds no value until its first pull is answered, so `shown`
// reads the value behind `waterui.ready`: the value the document received,
// never one it made up.
(function () {
  function report(kind, fields) {
    var record = { kind: kind, href: globalThis.location.href };
    for (var name in fields) {
      record[name] = fields[name];
    }
    globalThis.waterui.invoke("report", record);
  }

  globalThis.waterui.watch("count", function (value) {
    report("observed", { value: value });
  });
  globalThis.addEventListener("pageshow", function (event) {
    globalThis.waterui.ready.then(function () {
      report("shown", { persisted: event.persisted, value: globalThis.waterui.state.count });
    });
  });
})();
