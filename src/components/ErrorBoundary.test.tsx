import { describe, expect, it } from 'vitest'
import { renderToStaticMarkup } from 'react-dom/server'
import { ErrorBoundary } from './ErrorBoundary'

describe('ErrorBoundary', () => {
  it('renders its children until something throws', () => {
    expect(renderToStaticMarkup(<ErrorBoundary>fine</ErrorBoundary>)).toBe('fine')
  })

  it('turns a render error into a recovery screen with the message and a reload button', () => {
    const b = new ErrorBoundary({ children: 'fine' })
    b.state = ErrorBoundary.getDerivedStateFromError(new TypeError("Cannot read properties of undefined (reading 'toLowerCase')"))
    const html = renderToStaticMarkup(<>{b.render()}</>)
    expect(html).toContain('Something went wrong')
    expect(html).toContain('reading &#x27;toLowerCase&#x27;')
    expect(html).toContain('Reload')
    expect(ErrorBoundary.getDerivedStateFromError('plain string').error.message).toBe('plain string')
  })
})
