import init from "./pkg/app.js";

async function bootstrap() {
  try {
    await init();
  } catch (error) {
    console.error("Failed to start WaterUI Hydrolysis web app", error);
    document.body.innerHTML = "";
    const message = document.createElement("pre");
    message.className = "waterui-startup-error";
    message.textContent = String(error);
    document.body.appendChild(message);
  }
}

bootstrap();
