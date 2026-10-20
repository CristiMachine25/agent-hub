import { useCallback, useState } from 'react'

/** A remembered open/closed flag for a side panel. Storage can be unavailable, so every access is guarded. */
export function useCollapsed(key: string, initial: () => boolean): [boolean, (v: boolean) => void] {
  const [collapsed, setState] = useState(() => {
    try {
      const v = localStorage.getItem(key)
      if (v === '1' || v === '0') return v === '1'
    } catch {
      // fall through to the default
    }
    return initial()
  })
  const set = useCallback(
    (v: boolean) => {
      setState(v)
      try {
        localStorage.setItem(key, v ? '1' : '0')
      } catch {
        // not remembered, still applied
      }
    },
    [key]
  )
  return [collapsed, set]
}
