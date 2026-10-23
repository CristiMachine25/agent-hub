import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import '@fontsource-variable/hanken-grotesk'
// The optical-size build: display cuts for the large serif sizes, text cuts for small headings.
import '@fontsource-variable/source-serif-4/opsz.css'
import '@fontsource/ibm-plex-mono/400.css'
// A real bold for terminals (xterm's fontWeightBold), instead of a synthesized one.
import '@fontsource/ibm-plex-mono/600.css'
import './index.css'
import { App } from './App'
import { ErrorBoundary } from './components/ErrorBoundary'

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  </StrictMode>
)
