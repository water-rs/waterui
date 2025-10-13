import init from "./pkg/app.js";

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

bootstrap();
