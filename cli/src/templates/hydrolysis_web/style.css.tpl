/* Page geometry and background are set inline in index.html so the launch
   screen paints before this stylesheet arrives. */
/* The layout viewport exactly: `100vh` is taller than the visible area on
   mobile browsers, which makes the page itself pannable under the app. The
   engine stacks its canvases and hosted elements inside this element. */
#waterui-root {
  display: block;
  position: fixed;
  inset: 0;
  width: 100%;
  height: 100%;
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
