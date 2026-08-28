(() => {
    document.body.replaceChildren(document.createElement("input"));
    const input = document.body.firstElementChild;
    input.id = "waterui-input-probe";
    input.value = "waterui";
    input.autocomplete = "off";
    input.spellcheck = false;
    window.wateruiInputProbeEvents = [];
    for (const type of ["keydown", "keypress", "keyup", "beforeinput", "input"]) {
        input.addEventListener(type, (event) => {
            window.wateruiInputProbeEvents.push({
                type: event.type,
                key: event.key ?? null,
                code: event.code ?? null,
                metaKey: event.metaKey ?? false,
                controlKey: event.ctrlKey ?? false,
                inputType: event.inputType ?? null,
                data: event.data ?? null,
                defaultPrevented: event.defaultPrevented,
            });
        });
    }
    Object.assign(input.style, {
        boxSizing: "border-box",
        width: "100vw",
        height: "100vh",
        padding: "32px",
        border: "0",
        outline: "0",
        font: "32px system-ui",
    });
    input.focus();
    input.setSelectionRange(input.value.length, input.value.length);
    return "ready";
})();
