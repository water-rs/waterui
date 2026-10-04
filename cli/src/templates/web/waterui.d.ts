// The WaterUI webview bridge installs `window.waterui` when the app serves
// this page; it is absent in a plain browser tab.
declare global {
  interface Window {
    waterui?: {
      invoke<T = unknown>(name: string, payload?: unknown): Promise<T>
    }
  }
}

export {}
