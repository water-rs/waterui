<!DOCTYPE html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <meta name="color-scheme" content="light dark" />
    <title>{{ ctx.app_display_name }}</title>
    <style>
      /* The launch screen paints before anything is fetched, so everything it
         needs is inline: the page's own stylesheet arrives later. */
      :root {
        color-scheme: light dark;
        --waterui-launch-background: {{ launch.light.background }};
        --waterui-launch-foreground: {{ launch.light.foreground }};
      }
{% if let Some(dark) = launch.dark %}
      @media (prefers-color-scheme: dark) {
        :root {
          --waterui-launch-background: {{ dark.background }};
          --waterui-launch-foreground: {{ dark.foreground }};
        }
      }
{% endif %}
      html,
      body {
        margin: 0;
        width: 100%;
        height: 100%;
        overflow: hidden;
        background: var(--waterui-launch-background);
        /* The startup error, should one show, reads on the same background. */
        color: var(--waterui-launch-foreground);
      }
      #waterui-launch {
        position: fixed;
        inset: 0;
        z-index: 1;
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        gap: 28px;
        background: var(--waterui-launch-background);
        color: var(--waterui-launch-foreground);
        transition: opacity 220ms ease-out;
      }
      #waterui-launch.waterui-launch-leaving {
        opacity: 0;
        pointer-events: none;
      }
      #waterui-launch[hidden] {
        display: none;
      }
      .waterui-launch-artwork {
        width: 128px;
        height: 128px;
        display: block;
      }
      .waterui-launch-progress {
        width: 128px;
        height: 4px;
        border-radius: 2px;
        overflow: hidden;
        background: color-mix(in srgb, currentColor 14%, transparent);
      }
      .waterui-launch-progress-bar {
        height: 100%;
        width: 0;
        border-radius: inherit;
        background: color-mix(in srgb, currentColor 60%, transparent);
        transition: width 120ms linear;
      }
      .waterui-launch-progress.waterui-launch-indeterminate .waterui-launch-progress-bar {
        width: 40%;
        animation: waterui-launch-slide 1.2s ease-in-out infinite;
      }
      @keyframes waterui-launch-slide {
        from {
          transform: translateX(-100%);
        }
        to {
          transform: translateX(350%);
        }
      }
      @media (prefers-reduced-motion: reduce) {
        #waterui-launch {
          transition: none;
        }
        .waterui-launch-progress.waterui-launch-indeterminate .waterui-launch-progress-bar {
          animation: none;
          width: 100%;
        }
      }
    </style>
    <link rel="stylesheet" href="./style.css" />
  </head>
  <body>
    <div
      id="waterui-launch"
      role="status"
      aria-label="Loading {{ ctx.app_display_name }}"
      data-wasm-bytes="{{ launch.wasm_bytes }}"
    >
      <img class="waterui-launch-artwork" alt="" src="{{ launch.artwork_data_uri }}" />
      <div class="waterui-launch-progress waterui-launch-indeterminate">
        <div class="waterui-launch-progress-bar"></div>
      </div>
    </div>
    <canvas id="waterui-canvas"></canvas>
    <input id="waterui-ime" autocomplete="off" autocorrect="off" autocapitalize="off" spellcheck="false" />
    <script type="module" src="./bootstrap.js"></script>
  </body>
</html>
