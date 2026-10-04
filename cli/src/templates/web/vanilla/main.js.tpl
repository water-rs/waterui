import './style.css'
import javascriptLogo from '{{ ctx.logo }}'

const app = document.querySelector('#app')
app.innerHTML = `
  <main class="page">
    <div class="logos">
      <a href="https://waterui.dev" target="_blank">
        <img src="/waterui.svg" class="logo" alt="WaterUI logo" />
      </a>
      <img src="${javascriptLogo}" class="logo" alt="JavaScript logo" />
    </div>
    <h1>WaterUI + {{ ctx.framework }}</h1>
    <p class="tagline">
      Served by WaterUI from <code>web/</code> — edit
      <code>{{ ctx.entry }}</code> and save; <code>water run</code> gives you
      HMR.
    </p>
    <div class="card">
      <button id="counter" type="button"></button>
      <div class="bridge">
        <input id="name" type="text" value="WaterUI" />
        <button id="greet" type="button">Call Rust</button>
      </div>
      <p id="greeting" class="greeting"></p>
    </div>
    <p class="footer">
      <a href="https://waterui.dev" target="_blank">waterui.dev</a>
    </p>
  </main>
`

const counter = document.querySelector('#counter')
let count = 0
const renderCount = () => {
  counter.textContent = `count is ${count}`
}
counter.addEventListener('click', () => {
  count += 1
  renderCount()
})
renderCount()

const name = document.querySelector('#name')
const greeting = document.querySelector('#greeting')
document.querySelector('#greet').addEventListener('click', () => {
  const bridge = window.waterui
  if (!bridge) {
    greeting.textContent =
      'Not running inside WaterUI — the Rust bridge is only reachable in the app.'
    return
  }
  greeting.textContent = 'Calling Rust…'
  bridge
    .invoke('greet', { name: name.value })
    .then((reply) => {
      greeting.textContent = reply
    })
    .catch((error) => {
      greeting.textContent = `Bridge call failed: ${String(error)}`
    })
})
