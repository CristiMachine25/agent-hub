import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { placeTip } from '@/lib/tip'

type Shown = { el: HTMLElement; text: string }

const TIP_ID = 'app-tooltip'

/**
 * The app's one tooltip. Any element with `data-tip="…"` gets it at once on hover and on keyboard focus, in the app's
 * colors, instead of the OS's delayed native title. `data-tip-side="right"` puts it beside the element instead of below.
 * Mounted once; it listens on the document, so it also works in dialogs, the sidebar and terminal panes.
 */
export function Tooltip() {
  const [shown, setShown] = useState<Shown | null>(null)
  const current = useRef<HTMLElement | null>(null)
  const box = useRef<HTMLDivElement>(null)
  current.current = shown?.el ?? null

  useEffect(() => {
    const find = (t: EventTarget | null) => (t instanceof Element ? t.closest<HTMLElement>('[data-tip]') : null)
    const show = (el: HTMLElement | null) =>
      setShown((s) => (!el?.dataset.tip ? null : s?.el === el ? s : { el, text: el.dataset.tip }))
    const hide = () => setShown(null)
    const onOver = (e: PointerEvent) => show(find(e.target))
    // Leaving the window: there is no element to move into.
    const onOut = (e: PointerEvent) => !e.relatedTarget && hide()
    // Keyboard focus only; a click focuses too, and that should not bring back the tip it just hid.
    const onFocusIn = (e: FocusEvent) => {
      const el = find(e.target)
      if (el && (e.target as Element).matches(':focus-visible')) show(el)
    }
    const onFocusOut = (e: FocusEvent) => find(e.target) === current.current && hide()
    // Only scrolling that moves the element; terminals scroll on their own as output arrives.
    const onScroll = (e: Event) => {
      const a = current.current
      if (a && (e.target === document || (e.target instanceof Node && e.target.contains(a)))) hide()
    }
    const on: [EventTarget, string, EventListener][] = [
      [document, 'pointerover', onOver as EventListener],
      [document, 'pointerout', onOut as EventListener],
      [document, 'focusin', onFocusIn as EventListener],
      [document, 'focusout', onFocusOut as EventListener],
      [document, 'scroll', onScroll],
      [document, 'pointerdown', hide],
      [document, 'keydown', hide],
      [window, 'blur', hide]
    ]
    for (const [t, type, fn] of on) t.addEventListener(type, fn, true)
    return () => {
      for (const [t, type, fn] of on) t.removeEventListener(type, fn, true)
    }
  }, [])

  // Follow the label while it shows (Maximize turns into Restore, and so on).
  const el = shown?.el
  useEffect(() => {
    if (!el) return
    const watch = new MutationObserver(() => setShown((s) => (s?.el !== el ? s : el.dataset.tip ? { el, text: el.dataset.tip } : null)))
    watch.observe(el, { attributes: true, attributeFilter: ['data-tip'] })
    return () => watch.disconnect()
  }, [el])

  // While it shows, the tip describes its element, so screen readers hear it too (unless it only repeats the name).
  useEffect(() => {
    if (!shown) return
    const { el, text } = shown
    if (el.getAttribute('aria-label') === text || el.textContent?.trim() === text) return
    const ids = el.getAttribute('aria-describedby')
    el.setAttribute('aria-describedby', ids ? `${ids} ${TIP_ID}` : TIP_ID)
    return () => {
      if (ids) el.setAttribute('aria-describedby', ids)
      else el.removeAttribute('aria-describedby')
    }
  }, [shown])

  useLayoutEffect(() => {
    const tip = box.current
    if (!shown || !tip) return
    if (!shown.el.isConnected) return setShown(null)
    const root = document.documentElement
    const at = placeTip(
      shown.el.getBoundingClientRect(),
      { width: tip.offsetWidth, height: tip.offsetHeight },
      { width: root.clientWidth, height: root.clientHeight },
      shown.el.dataset.tipSide === 'right' ? 'right' : 'bottom'
    )
    tip.style.transform = `translate(${Math.round(at.left)}px, ${Math.round(at.top)}px)`
  }, [shown])

  return (
    <div
      ref={box}
      id={TIP_ID}
      role="tooltip"
      hidden={!shown}
      className="tip-in pointer-events-none fixed left-0 top-0 z-[60] max-w-[min(320px,calc(100vw-16px))] whitespace-pre-line break-words rounded border border-rivet bg-raised px-2 py-1 text-xs text-ink shadow-lg shadow-black/25"
    >
      {shown?.text}
    </div>
  )
}
