import React from 'react'
import { MotionConfig } from 'framer-motion'
import ReactDOM from 'react-dom/client'
import { App } from './App'
import { suppressNativeContextMenu } from './lib/nativeMenu'
import './styles.css'

// Before the first render, so no right click can ever reach the browser's
// own menu — not even during startup.
suppressNativeContextMenu()

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    {/* Windows' "show animations" setting, honoured by everything that moves
        — not only the CSS, which the stylesheet already handles. */}
    <MotionConfig reducedMotion="user">
      <App />
    </MotionConfig>
  </React.StrictMode>,
)
