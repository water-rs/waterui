import init from "./pkg/app.js";

const launch = document.getElementById("waterui-launch");
const progress = launch.querySelector(".waterui-launch-progress");
const progressBar = launch.querySelector(".waterui-launch-progress-bar");
const canvas = document.getElementById("waterui-canvas");

// The wasm size is stamped into the page at build time, so the bar stays
// determinate whatever the server does to Content-Length (a compressed
// transfer reports the compressed size, the stream yields decoded bytes).
const wasmBytes = Number(launch.dataset.wasmBytes) || 0;

function showProgress(loaded, total) {
  if (!total) {
    progress.classList.add("waterui-launch-indeterminate");
    return;
  }
  progress.classList.remove("waterui-launch-indeterminate");
  progressBar.style.width = `${Math.min(100, (loaded / total) * 100).toFixed(1)}%`;
}

async function fetchWasm() {
  const url = new URL("./pkg/app_bg.wasm", import.meta.url);
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`Failed to fetch ${url.pathname}: HTTP ${response.status}`);
  }
  const total = wasmBytes || Number(response.headers.get("Content-Length")) || 0;
  showProgress(0, total);
  if (!response.body) {
    return response;
  }
  let loaded = 0;
  const reader = response.body.getReader();
  const counted = new ReadableStream({
    async pull(controller) {
      const { done, value } = await reader.read();
      if (done) {
        controller.close();
        return;
      }
      loaded += value.byteLength;
      showProgress(loaded, total);
      controller.enqueue(value);
    },
    cancel(reason) {
      return reader.cancel(reason);
    },
  });
  // The original headers keep `Content-Type: application/wasm`, which is what
  // lets wasm-bindgen use `WebAssembly.instantiateStreaming`.
  return new Response(counted, {
    status: response.status,
    statusText: response.statusText,
    headers: response.headers,
  });
}

function dismissLaunchScreen() {
  launch.classList.add("waterui-launch-leaving");
  const remove = () => launch.remove();
  if (matchMedia("(prefers-reduced-motion: reduce)").matches) {
    remove();
  } else {
    launch.addEventListener("transitionend", remove, { once: true });
  }
}

function showStartupError(error) {
  console.error("Failed to start WaterUI Hydrolysis web app", error);
  document.body.innerHTML = "";
  const message = document.createElement("pre");
  message.className = "waterui-startup-error";
  message.textContent = String(error);
  document.body.appendChild(message);
}

async function bootstrap() {
  canvas.addEventListener("waterui:first-frame", dismissLaunchScreen, { once: true });
  try {
    await init({ module_or_path: fetchWasm() });
  } catch (error) {
    showStartupError(error);
  }
}

bootstrap();
