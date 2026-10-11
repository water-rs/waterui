import { useState } from 'react'
import reactLogo from '{{ ctx.logo }}'
import './App.css'

function App() {
  const [count, setCount] = useState(0)
  const [name, setName] = useState('WaterUI')
  const [greeting, setGreeting] = useState('')

  const callRust = () => {
    const bridge = window.waterui
    if (!bridge) {
      setGreeting(
        'Not running inside WaterUI — the Rust bridge is only reachable in the app.',
      )
      return
    }
    setGreeting('Calling Rust…')
    bridge
      .invoke<string>('greet', { name })
      .then(setGreeting)
      .catch((error: unknown) => {
        setGreeting(`Bridge call failed: ${String(error)}`)
      })
  }

  return (
    <main className="page">
      <div className="logos">
        <a href="https://waterui.dev" target="_blank">
          <img src="/waterui.svg" className="logo" alt="WaterUI logo" />
        </a>
        <img src={reactLogo} className="logo" alt="React logo" />
      </div>
      <h1>WaterUI + {{ ctx.framework }}</h1>
      <p className="tagline">
        Served by WaterUI from <code>web/</code> — edit{' '}
        <code>{{ ctx.entry }}</code> and save; <code>water run</code> gives you
        HMR.
      </p>
      <div className="card">
        <button onClick={() => setCount((current) => current + 1)}>
          count is {count}
        </button>
        <div className="bridge">
          <input
            type="text"
            value={name}
            onChange={(event) => setName(event.target.value)}
          />
          <button onClick={callRust}>Call Rust</button>
        </div>
        <p className="greeting">{greeting}</p>
      </div>
      <p className="footer">
        <a href="https://waterui.dev" target="_blank">
          waterui.dev
        </a>
      </p>
    </main>
  )
}

export default App
