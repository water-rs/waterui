html,
body {
  margin: 0;
  width: 100%;
  height: 100%;
  overflow: hidden;
  background: #ffffff;
}

body {
  display: block;
}

#waterui-canvas {
  display: block;
  width: 100vw;
  height: 100vh;
}

#waterui-ime {
  position: fixed;
  opacity: 0;
  pointer-events: none;
  z-index: -1;
  left: 0;
  top: 0;
  width: 1px;
  height: 1px;
}

.waterui-startup-error {
  margin: 0;
  padding: 24px;
  font: 14px/1.5 ui-monospace, SFMono-Regular, Menlo, monospace;
  white-space: pre-wrap;
}
