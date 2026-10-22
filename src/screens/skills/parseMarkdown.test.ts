import { describe, expect, it } from 'vitest'
import { parseBlocks, parseInline } from './parseMarkdown'

describe('parseBlocks', () => {
  it('reads headings, paragraphs and rules', () => {
    expect(parseBlocks('# Title\n\nOne line\nand the next\n\n---\n## Next ##')).toEqual([
      { type: 'heading', level: 1, text: 'Title' },
      { type: 'paragraph', text: 'One line\nand the next' },
      { type: 'rule' },
      { type: 'heading', level: 2, text: 'Next' }
    ])
  })

  it('keeps fenced code exactly, Markdown and blank lines included', () => {
    const src = 'Run:\n\n```bash\nnpm test\n\n  # **not bold**\n```\nAfter'
    expect(parseBlocks(src)).toEqual([
      { type: 'paragraph', text: 'Run:' },
      { type: 'code', lang: 'bash', text: 'npm test\n\n  # **not bold**' },
      { type: 'paragraph', text: 'After' }
    ])
  })

  it('runs an unclosed fence to the end', () => {
    expect(parseBlocks('~~~\na\nb')).toEqual([{ type: 'code', lang: '', text: 'a\nb' }])
  })

  it('reads ordered and nested lists with their start number', () => {
    const [list] = parseBlocks('3. Read\n4. Follow\n   - sub one\n   - sub two\n5. Report')
    expect(list).toEqual({
      type: 'list',
      ordered: true,
      start: 3,
      items: [
        [{ type: 'paragraph', text: 'Read' }],
        [
          { type: 'paragraph', text: 'Follow' },
          { type: 'list', ordered: false, start: 1, items: [[{ type: 'paragraph', text: 'sub one' }], [{ type: 'paragraph', text: 'sub two' }]] }
        ],
        [{ type: 'paragraph', text: 'Report' }]
      ]
    })
  })

  it('keeps a loose list together and ends it at a paragraph', () => {
    const blocks = parseBlocks('- one\n\n- two\n\nAfter the list')
    expect(blocks.map((b) => b.type)).toEqual(['list', 'paragraph'])
    expect(blocks[0]).toMatchObject({ items: [[{ text: 'one' }], [{ text: 'two' }]] })
  })

  it('puts code inside a list item', () => {
    const [list] = parseBlocks('1. Install:\n\n   ```sh\n   npm i\n   ```\n2. Done')
    expect(list).toMatchObject({ items: [[{ type: 'paragraph' }, { type: 'code', lang: 'sh', text: 'npm i' }], [{ text: 'Done' }]] })
  })

  it('reads tables, padding short rows and keeping escaped pipes', () => {
    expect(parseBlocks('| Tool | Use |\n|---|:-:|\n| `a\\|b` | pipes |\n| only |')).toEqual([
      { type: 'table', head: ['Tool', 'Use'], rows: [['`a|b`', 'pipes'], ['only', '']] }
    ])
  })

  it('reads quotes as blocks', () => {
    expect(parseBlocks('> **Note**\n> second')).toEqual([{ type: 'quote', blocks: [{ type: 'paragraph', text: '**Note**\nsecond' }] }])
  })
})

describe('parseInline', () => {
  it('reads code, emphasis and links', () => {
    expect(parseInline('Use `npm test`, **always** and *often*, see [docs](https://x.dev "t").')).toEqual([
      'Use ',
      { t: 'code', v: 'npm test' },
      ', ',
      { t: 'strong', c: ['always'] },
      ' and ',
      { t: 'em', c: ['often'] },
      ', see ',
      { t: 'link', href: 'https://x.dev', c: ['docs'] },
      '.'
    ])
  })

  it('leaves snake_case words and escaped marks alone', () => {
    expect(parseInline('run_tests and \\*literal\\* _em_')).toEqual(['run_tests and *literal* ', { t: 'em', c: ['em'] }])
  })

  it('does not read marks inside code', () => {
    expect(parseInline('`**x**` and `` a`b ``')).toEqual([{ t: 'code', v: '**x**' }, ' and ', { t: 'code', v: 'a`b' }])
  })

  it('turns images into their alt text and keeps autolinks', () => {
    expect(parseInline('![diagram](a.png) <https://skills.sh>')).toEqual([{ t: 'image', alt: 'diagram' }, ' ', { t: 'link', href: 'https://skills.sh', c: ['https://skills.sh'] }])
  })
})
