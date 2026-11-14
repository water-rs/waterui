<script setup{% if ctx.typescript %} lang="ts"{% endif %}>
import { ref } from 'vue'
import vueLogo from '{{ ctx.logo }}'

const count = ref(0)
const name = ref('WaterUI')
const greeting = ref('')

function callRust() {
  const bridge = window.waterui
  if (!bridge) {
    greeting.value =
      'Not running inside WaterUI — the Rust bridge is only reachable in the app.'
    return
  }
  greeting.value = 'Calling Rust…'
  bridge
    .invoke{% if ctx.typescript %}<string>{% endif %}('greet', { name: name.value })
    .then((reply) => {
      greeting.value = reply
    })
    .catch((error{% if ctx.typescript %}: unknown{% endif %}) => {
      greeting.value = `Bridge call failed: ${String(error)}`
    })
}
</script>

<template>
  <main class="page">
    <div class="logos">
      <a href="https://waterui.dev" target="_blank">
        <img src="/waterui.svg" class="logo" alt="WaterUI logo" />
      </a>
      <img :src="vueLogo" class="logo" alt="Vue logo" />
    </div>
    <h1>WaterUI + {{ ctx.framework }}</h1>
    <p class="tagline">
      Served by WaterUI from <code>web/</code> — edit
      <code>{{ ctx.entry }}</code> and save; <code>water run</code> gives you
      HMR.
    </p>
    <div class="card">
      <button type="button" @click="count++">
        {% raw %}count is {{ count }}{% endraw %}
      </button>
      <div class="bridge">
        <input v-model="name" type="text" />
        <button type="button" @click="callRust">Call Rust</button>
      </div>
      <p v-if="greeting" class="greeting">{% raw %}{{ greeting }}{% endraw %}</p>
    </div>
    <p class="footer">
      <a href="https://waterui.dev" target="_blank">waterui.dev</a>
    </p>
  </main>
</template>
