/* Page geometry and background are set inline in index.html so the launch
   screen paints before this stylesheet arrives. */
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
