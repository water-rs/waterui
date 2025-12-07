import init from "./pkg/app.js";

const HOT_RELOAD_PORT = "__HOT_RELOAD_PORT__";

async function bootstrap() {
  try {
    const wasm = await init();
    if (wasm && typeof wasm.waterui_init === "function") {
      wasm.waterui_init();
    }

    if (wasm && typeof wasm.waterui_main === "function") {
      // The current web backend is experimental. Rendering is handled by
      // packages in the WaterUI workspace and will evolve as the project matures.
      wasm.waterui_main();
    }

    console.info("WaterUI WebAssembly bundle initialised.");
  } catch (error) {
    console.error("Failed to start WaterUI web app", error);
    const root = document.getElementById("water-app");
    if (root) {
      root.innerHTML = "";
      const message = document.createElement("pre");
      message.className = "waterui-error";
      message.textContent = String(error);
      root.appendChild(message);
    }
  }
}

function connect_hot_reload() {
    const ws = new WebSocket(`ws://127.0.0.1:${HOT_RELOAD_PORT}/hot-reload-web`);
    ws.onmessage = (event) => {
        if (event.data === "reload") {
            location.reload();
        }
    };
    ws.onclose = () => {
        setTimeout(connect_hot_reload, 1000);
    };
}

if (HOT_RELOAD_PORT !== "__HOT_RELOAD_PORT__") {
    connect_hot_reload();
}

bootstrap();
