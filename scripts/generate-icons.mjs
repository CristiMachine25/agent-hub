import { copyFile, mkdir } from 'node:fs/promises'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { execFileSync } from 'node:child_process'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const output = join(root, 'src-tauri/target/icon-build')
const cli = join(root, 'node_modules/@tauri-apps/cli/tauri.js')
const source = join(root, 'app-icon.png')
execFileSync(process.execPath, [cli, 'icon', source, '--output', output], { stdio: 'inherit' })

const desktop = [
  '32x32.png', '64x64.png', '128x128.png', '128x128@2x.png', 'icon.png', 'icon.icns', 'icon.ico',
  'StoreLogo.png', 'Square30x30Logo.png', 'Square44x44Logo.png', 'Square71x71Logo.png',
  'Square89x89Logo.png', 'Square107x107Logo.png', 'Square142x142Logo.png',
  'Square150x150Logo.png', 'Square284x284Logo.png', 'Square310x310Logo.png'
]
await Promise.all(desktop.map((name) => copyFile(join(output, name), join(root, 'src-tauri/icons', name))))
await mkdir(join(root, 'src/assets'), { recursive: true })
await copyFile(join(output, '128x128.png'), join(root, 'src/assets/app-icon.png'))
await mkdir(join(root, 'site/assets'), { recursive: true })
await copyFile(join(output, '64x64.png'), join(root, 'site/favicon.png'))
await copyFile(join(output, '128x128.png'), join(root, 'site/assets/app-icon.png'))
execFileSync(process.execPath, [cli, 'icon', source, '--output', output, '--png', '180'], { stdio: 'inherit' })
await copyFile(join(output, '180x180.png'), join(root, 'site/assets/apple-touch-icon.png'))
console.log('Relay desktop, interface, and website icons generated from app-icon.png.')
