/**
 * pnpm shot [out.png] [view query] [--ui] [--size WxH] [--timeout S]
 *
 * The app in headless Chromium at the view the query gives
 * (parseViewQuery in web/src/lib/renderer.ts, e.g. 'seed=1&look=30,-10'),
 * saved once nothing is left to bake, by default without its panels:
 * to check a change without a GPU.
 * Exits with 1 if the view did not complete in time (it saves it anyway).
 *
 * It draws with Mesa's software Vulkan (lavapipe) where installed,
 * several times faster than Chromium's own SwiftShader.
 * Even so a frame takes seconds, so it draws none until the view is complete:
 * those frames would only take cores from the bake workers.
 * The native frontend's --shot is faster still (see AGENTS.md).
 * Playwright is not a dependency: it comes with the machine (NODE_PATH).
 */
import { existsSync } from 'node:fs'
import { createRequire } from 'node:module'
import { createServer } from 'vite'

const args = process.argv.slice(2)
const option = (name, fallback) => {
  const i = args.indexOf(name)
  return i < 0 ? fallback : args.splice(i, 2)[1]
}
const [width, height] = option('--size', '1280x800').split('x').map(Number)
const timeout = Number(option('--timeout', 180)) * 1000
const ui = args.includes('--ui')
const [out = 'shot.png', query = ''] = args.filter(a => a !== '--ui')

const lavapipe = ['/usr/share/vulkan/icd.d/lvp_icd.json', '/etc/vulkan/icd.d/lvp_icd.json'].some(existsSync)
const { chromium } = createRequire(import.meta.url)('playwright')

const start = performance.now()
const log = message => { console.log(`${((performance.now() - start) / 1000).toFixed(1)} s: ${message}`) }

const server = await createServer({ root: 'web', server: { port: 0 }, logLevel: 'warn' })
await server.listen()
const browser = await chromium.launch({
  args: lavapipe ? ['--use-angle=vulkan', '--enable-features=Vulkan,VulkanFromANGLE,DefaultANGLEVulkan', '--ignore-gpu-blocklist'] : [],
})
try {
  const page = await browser.newPage({ viewport: { width, height } })
  page.on('console', m => { if (['error', 'warning'].includes(m.type())) console.log(`page ${m.type()}: ${m.text()}`) })
  page.on('pageerror', e => { console.log(`page error: ${e.message}`) })
  // WebGL 2's draws do nothing until window.shotDraws is set.
  await page.addInitScript(() => {
    const gl = WebGL2RenderingContext.prototype
    for (const name of ['drawArrays', 'drawElements', 'drawArraysInstanced', 'drawElementsInstanced', 'drawRangeElements']) {
      const draw = gl[name]
      gl[name] = function (...a) { if (window.shotDraws) draw.apply(this, a) }
    }
  })
  const url = new URL(server.resolvedUrls.local[0])
  url.search = query
  await page.goto(url.href)
  const gl = await page.evaluate(() => {
    const context = document.createElement('canvas').getContext('webgl2')
    return context ? String(context.getParameter(context.getExtension('WEBGL_debug_renderer_info')?.UNMASKED_RENDERER_WEBGL ?? context.RENDERER)) : 'no WebGL 2'
  })
  log(`loaded, drawing with ${gl}`)
  if (!gl.includes('llvmpipe')) console.warn('Not on lavapipe (apt-get install mesa-vulkan-drivers): much slower.')

  // The app sets data-pending after each render: chunks baking and levels generating.
  const complete = await page.locator('canvas[data-pending="0"]').waitFor({ timeout }).then(() => true, () => false)
  log(complete ? 'complete' : `gave up, ${await page.locator('canvas').getAttribute('data-pending')} still pending`)
  if (!ui) await page.addStyleTag({ content: 'body * { visibility: hidden } canvas { visibility: visible }' })
  // A resize renders again (App.tsx), drawing this time; the screenshot waits for that frame.
  await page.evaluate(() => {
    window.shotDraws = true
    window.dispatchEvent(new Event('resize'))
  })
  await page.screenshot({ path: out, timeout: 120_000 })
  log(`saved ${out}`)
  process.exitCode = complete ? 0 : 1
} finally {
  await browser.close()
  await server.close()
}
