/**
 * A small Markdown reader for SKILL.md: the blocks and inline marks skills actually use (headings, lists,
 * fenced code, tables, quotes, emphasis, links). It builds plain data, never HTML, so a skill's text can't
 * inject markup into the app.
 */

export type Inline =
  | string
  | { t: 'code'; v: string }
  | { t: 'strong' | 'em'; c: Inline[] }
  | { t: 'link'; href: string; c: Inline[] }
  | { t: 'image'; alt: string }

export type Block =
  | { type: 'heading'; level: number; text: string }
  | { type: 'paragraph'; text: string }
  | { type: 'code'; lang: string; text: string }
  | { type: 'list'; ordered: boolean; start: number; items: Block[][] }
  | { type: 'quote'; blocks: Block[] }
  | { type: 'rule' }
  | { type: 'table'; head: string[]; rows: string[][] }

const FENCE = /^(\s{0,3})(`{3,}|~{3,})\s*([^`\s]*)/
const HEADING = /^\s{0,3}(#{1,6})\s+(.*?)(?:\s+#+)?\s*$/
const RULE = /^\s{0,3}([-*_])(?:\s*\1){2,}\s*$/
const QUOTE = /^\s{0,3}>\s?/
const ITEM = /^(\s*)([-*+]|\d{1,9}[.)])(\s+)(.*)$/
const DELIM = /^\s*\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?\s*$/

const indentOf = (line: string) => line.length - line.trimStart().length
const blank = (line: string | undefined) => line === undefined || line.trim() === ''
/** A table row's cells; `\|` stays a pipe. (No lookbehind: older macOS WebViews can't parse it.) */
function cells(line: string) {
  const out: string[] = []
  let cell = ''
  for (let k = 0, s = line.trim(); k < s.length; k++) {
    const c = s.charAt(k)
    if (c === '\\' && s.charAt(k + 1) === '|') cell += s.charAt(++k)
    else if (c === '|') {
      out.push(cell)
      cell = ''
    } else cell += c
  }
  out.push(cell)
  if (line.trim().startsWith('|')) out.shift()
  if (line.trim().endsWith('|') && !line.trim().endsWith('\\|')) out.pop()
  return out.map((c) => c.trim())
}

function startsBlock(line: string, next?: string) {
  return FENCE.test(line) || HEADING.test(line) || QUOTE.test(line) || RULE.test(line) || ITEM.test(line) || (line.includes('|') && next !== undefined && DELIM.test(next))
}

export function parseBlocks(src: string): Block[] {
  const lines = src.replace(/\r\n?/g, '\n').split('\n')
  const out: Block[] = []
  let i = 0
  while (i < lines.length) {
    const line = lines[i]!
    if (blank(line)) {
      i++
      continue
    }

    const fence = FENCE.exec(line)
    if (fence) {
      const [, pad, marks, lang] = fence as unknown as [string, string, string, string]
      const body: string[] = []
      i++
      while (i < lines.length && !new RegExp(`^\\s{0,3}${marks[0]}{${marks.length},}\\s*$`).test(lines[i]!)) {
        const l = lines[i]!
        body.push(l.slice(Math.min(pad.length, indentOf(l))))
        i++
      }
      i++ // the closing fence, or the end
      out.push({ type: 'code', lang, text: body.join('\n') })
      continue
    }

    const heading = HEADING.exec(line)
    if (heading) {
      out.push({ type: 'heading', level: heading[1]!.length, text: heading[2]! })
      i++
      continue
    }

    if (RULE.test(line)) {
      out.push({ type: 'rule' })
      i++
      continue
    }

    if (QUOTE.test(line)) {
      const body: string[] = []
      while (i < lines.length && QUOTE.test(lines[i]!)) body.push(lines[i++]!.replace(QUOTE, ''))
      out.push({ type: 'quote', blocks: parseBlocks(body.join('\n')) })
      continue
    }

    if (line.includes('|') && DELIM.test(lines[i + 1] ?? '')) {
      const head = cells(line)
      const rows: string[][] = []
      i += 2
      while (i < lines.length && !blank(lines[i]) && lines[i]!.includes('|')) rows.push(cells(lines[i++]!))
      out.push({ type: 'table', head, rows: rows.map((r) => head.map((_, k) => r[k] ?? '')) })
      continue
    }

    const item = ITEM.exec(line)
    if (item) {
      const base = item[1]!.length
      const ordered = /\d/.test(item[2]!)
      const items: string[][] = []
      let content = 0
      while (i < lines.length) {
        const l = lines[i]!
        const m = ITEM.exec(l)
        if (m && m[1]!.length <= base + 1 && /\d/.test(m[2]!) === ordered) {
          content = m[1]!.length + m[2]!.length + Math.min(m[3]!.length, 4)
          items.push([m[4]!])
          i++
        } else if (blank(l)) {
          // A blank line ends the list unless more of it follows.
          let j = i + 1
          while (blank(lines[j]) && j < lines.length) j++
          const after = lines[j]
          const same = after !== undefined && ITEM.exec(after)
          if (after !== undefined && (indentOf(after) >= content || (same && same[1]!.length <= base + 1 && /\d/.test(same[2]!) === ordered))) {
            items[items.length - 1]!.push('')
            i++
          } else break
        } else if (indentOf(l) >= content) {
          items[items.length - 1]!.push(l.slice(content))
          i++
        } else if (!startsBlock(l, lines[i + 1])) {
          items[items.length - 1]!.push(l.trim()) // a lazy continuation of the item's paragraph
          i++
        } else break
      }
      out.push({ type: 'list', ordered, start: ordered ? parseInt(item[2]!, 10) : 1, items: items.map((it) => parseBlocks(it.join('\n'))) })
      continue
    }

    const para: string[] = [line.trim()]
    i++
    while (i < lines.length && !blank(lines[i]) && !startsBlock(lines[i]!, lines[i + 1])) para.push(lines[i++]!.trim())
    out.push({ type: 'paragraph', text: para.join('\n') })
  }
  return out
}

const INLINE =
  /\\([\\`*_{}[\]()#+\-.!|>~])|(`+)([\s\S]*?[^`])\2(?!`)|!\[([^\]]*)\]\([^)]*\)|\[([^\]]+)\]\(\s*<?([^)\s>]+)>?(?:\s+"[^"]*")?\s*\)|<(https?:\/\/[^>\s]+)>|\*\*([\s\S]+?)\*\*|__([\s\S]+?)__|\*([^*\s](?:[\s\S]*?[^*\s])?)\*|(^|[^\w])_([^_\s](?:[\s\S]*?[^_\s])?)_(?!\w)/g

export function parseInline(src: string): Inline[] {
  const out: Inline[] = []
  const text = (s: string) => {
    if (!s) return
    const last = out[out.length - 1]
    if (typeof last === 'string') out[out.length - 1] = last + s
    else out.push(s)
  }
  let at = 0
  for (const m of src.matchAll(new RegExp(INLINE))) {
    text(src.slice(at, m.index))
    at = m.index + m[0].length
    const [, escaped, , code, alt, label, href, auto, strong, strong2, em, pre, em2] = m
    if (escaped !== undefined) text(escaped)
    else if (code !== undefined) out.push({ t: 'code', v: code.replace(/^ (.*) $/s, '$1').replace(/\n/g, ' ') })
    else if (alt !== undefined) out.push({ t: 'image', alt })
    else if (label !== undefined) out.push({ t: 'link', href: href!, c: parseInline(label) })
    else if (auto !== undefined) out.push({ t: 'link', href: auto, c: [auto] })
    else if (strong !== undefined || strong2 !== undefined) out.push({ t: 'strong', c: parseInline((strong ?? strong2)!) })
    else if (em !== undefined) out.push({ t: 'em', c: parseInline(em) })
    else if (em2 !== undefined) {
      text(pre!)
      out.push({ t: 'em', c: parseInline(em2) })
    }
  }
  text(src.slice(at))
  return out
}
