import { Fragment, useMemo, type ReactNode } from 'react'
import { parseBlocks, parseInline, type Block, type Inline } from './parseMarkdown'
import { openUrl } from './shared'

/** SKILL.md set as a readable document: serif headings, a comfortable measure, real code blocks. */
export function Markdown({ source }: { source: string }) {
  const blocks = useMemo(() => parseBlocks(source), [source])
  return <div className="max-w-[68ch] text-[14px] leading-[22px]">{blocks.map(renderBlock)}</div>
}

function renderBlock(b: Block, k: number): ReactNode {
  switch (b.type) {
    case 'heading': {
      const cls =
        b.level === 1
          ? 'mb-3 mt-8 font-serif text-lg font-semibold first:mt-0'
          : b.level === 2
            ? 'mb-2 mt-7 font-serif text-[17px] font-semibold leading-6 first:mt-0'
            : 'mb-1.5 mt-5 text-[14px] font-semibold first:mt-0'
      const H = (['h3', 'h3', 'h4', 'h5', 'h6', 'h6'] as const)[b.level - 1]!
      return (
        <H key={k} className={cls}>
          {inline(b.text)}
        </H>
      )
    }
    case 'paragraph':
      return (
        <p key={k} className="my-3 first:mt-0 last:mb-0">
          {inline(b.text)}
        </p>
      )
    case 'code':
      return (
        <figure key={k} className="my-4 overflow-hidden rounded border border-rivet bg-well first:mt-0 last:mb-0">
          {b.lang && <figcaption className="border-b border-rivet px-3 py-1 text-xs text-faint">{b.lang}</figcaption>}
          <pre className="code overflow-x-auto px-3 py-2.5 text-ink">
            <code>{b.text}</code>
          </pre>
        </figure>
      )
    case 'list': {
      const L = b.ordered ? 'ol' : 'ul'
      return (
        <L key={k} start={b.ordered && b.start !== 1 ? b.start : undefined} className={b.ordered ? 'my-3 list-decimal pl-6 marker:text-muted' : 'my-3 list-disc pl-5 marker:text-faint'}>
          {b.items.map((item, j) => (
            // A tight item is one paragraph: render it bare so the list keeps its rhythm.
            <li key={j} className="my-1 pl-1 [&_ol]:my-1 [&_ul]:my-1">
              {item.length === 1 && item[0]!.type === 'paragraph' ? inline(item[0]!.text) : item.map(renderBlock)}
            </li>
          ))}
        </L>
      )
    }
    case 'quote':
      return (
        <blockquote key={k} className="my-4 border-l-2 border-rivet pl-4 text-muted">
          {b.blocks.map(renderBlock)}
        </blockquote>
      )
    case 'rule':
      return <hr key={k} className="my-6 border-rivet" />
    case 'table':
      return (
        <div key={k} className="my-4 overflow-x-auto rounded border border-rivet">
          <table className="w-full border-collapse text-sm">
            <thead className="bg-well">
              <tr>
                {b.head.map((h, j) => (
                  <th key={j} className="border-b border-rivet px-3 py-1.5 text-left font-semibold">
                    {inline(h)}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {b.rows.map((r, j) => (
                <tr key={j} className="border-b border-rivet last:border-b-0">
                  {r.map((c, n) => (
                    <td key={n} className="px-3 py-1.5 align-top">
                      {inline(c)}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )
  }
}

function inline(src: string): ReactNode {
  // A soft line break inside a paragraph reads as a space.
  return parseInline(src).map(renderInline)
}

function renderInline(n: Inline, k: number): ReactNode {
  if (typeof n === 'string') return <Fragment key={k}>{n.replace(/\n/g, ' ')}</Fragment>
  switch (n.t) {
    case 'code':
      return (
        <code key={k} className="rounded-[4px] border border-rivet bg-well px-1 py-px font-mono text-[12.5px]">
          {n.v}
        </code>
      )
    case 'strong':
      return (
        <strong key={k} className="font-semibold">
          {n.c.map(renderInline)}
        </strong>
      )
    case 'em':
      return <em key={k}>{n.c.map(renderInline)}</em>
    case 'image':
      return (
        <span key={k} className="text-faint">
          [{n.alt || 'image'}]
        </span>
      )
    case 'link':
      // Only web links open; a path inside the skill's repo has nowhere to go from here.
      return /^https?:\/\//.test(n.href) ? (
        <a
          key={k}
          href={n.href}
          data-tip={n.href}
          onClick={(e) => {
            e.preventDefault()
            openUrl(n.href)
          }}
          className="text-hop underline decoration-hop/40 underline-offset-2 hover:decoration-hop"
        >
          {n.c.map(renderInline)}
        </a>
      ) : (
        <span key={k} className="underline decoration-rivet underline-offset-2" data-tip={n.href}>
          {n.c.map(renderInline)}
        </span>
      )
  }
}
