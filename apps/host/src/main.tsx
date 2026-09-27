import React from 'react'
import ReactDOM from 'react-dom/client'
import { App } from './App'
import { suppressNativeContextMenu } from './lib/nativeMenu'
import { startUpdateChecks } from './lib/updates'
import './styles.css'

// Before the first render, so no right click can ever reach the browser's
// own menu — not even during startup.
suppressNativeContextMenu()

// A newer host is looked for on opening and twice a day after: a host runs
// for weeks, and the offer should be waiting at the top of the window.
startUpdateChecks()

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
)
