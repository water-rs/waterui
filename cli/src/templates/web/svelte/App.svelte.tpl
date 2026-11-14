<script{% if ctx.typescript %} lang="ts"{% endif %}>
  import svelteLogo from '{{ ctx.logo }}'

  let count = 0
  let name = 'WaterUI'
  let greeting = ''

  async function callRust() {
    const bridge = window.waterui
    if (!bridge) {
      greeting =
        'Not running inside WaterUI — the Rust bridge is only reachable in the app.'
      return
    }
    greeting = 'Calling Rust…'
    try {
      greeting = await bridge.invoke{% if ctx.typescript %}<string>{% endif %}('greet', { name })
    } catch (error) {
      greeting = `Bridge call failed: ${String(error)}`
    }
  }
</script>

<main class="page">
  <div class="logos">
    <a href="https://waterui.dev" target="_blank">
      <img src="/waterui.svg" class="logo" alt="WaterUI logo" />
    </a>
    <img src={svelteLogo} class="logo" alt="Svelte logo" />
  </div>
  <h1>WaterUI + {{ ctx.framework }}</h1>
  <p class="tagline">
    Served by WaterUI from <code>web/</code> — edit
    <code>{{ ctx.entry }}</code> and save; <code>water run</code> gives you HMR.
  </p>
  <div class="card">
    <button type="button" onclick={() => (count += 1)}>
      count is {count}
    </button>
    <div class="bridge">
      <input bind:value={name} type="text" />
      <button type="button" onclick={callRust}>Call Rust</button>
    </div>
    <p class="greeting">{greeting}</p>
  </div>
  <p class="footer">
    <a href="https://waterui.dev" target="_blank">waterui.dev</a>
  </p>
</main>
