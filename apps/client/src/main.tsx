import React from 'react'
import { MotionConfig } from 'framer-motion'
import ReactDOM from 'react-dom/client'
import { App } from './App'
import { suppressNativeContextMenu } from './lib/nativeMenu'
import { startUpdateChecks } from './lib/updates'
import './styles.css'

// Before the first render, so no right click can ever reach the browser's
// own menu — not even during startup.
suppressNativeContextMenu()

// A newer Basalt is looked for on opening and twice a day after, so the offer
// is waiting in the sidebar, or on a phone in a banner and a notification,
// without anyone going to Settings to ask.
startUpdateChecks()

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    {/* Windows' "show animations" setting, honoured by everything that moves
        — not only the CSS, which the stylesheet already handles. */}
    <MotionConfig reducedMotion="user">
      <App />
    </MotionConfig>
  </React.StrictMode>,
)
