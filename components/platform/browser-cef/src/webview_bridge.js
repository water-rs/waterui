(() => {
    const pending = new Map();
    let nextId = 0;

    // Reachable from ordinary page script: an unknown id is ignored rather than
    // thrown, so a page cannot turn a stray call into an unhandled exception.
    globalThis.__wateruiResolve = (id, ok, payload) => {
        const entry = pending.get(id);
        if (entry === undefined) {
            return;
        }
        pending.delete(id);
        if (ok) {
            entry.resolve(Uint8Array.from(payload));
        } else {
            entry.reject(new Error(String(payload)));
        }
    };

    globalThis.waterui = Object.freeze({
        invoke(name, data = []) {
            const id = ++nextId;
            const bytes = Array.from(data);
            return new Promise((resolve, reject) => {
                pending.set(id, { resolve, reject });
                globalThis.__wateruiInvoke(JSON.stringify({ id, name, data: bytes }));
            });
        },
    });
})();
