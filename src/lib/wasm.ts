/**
 * The web's WebAssembly module, worldgen/ compiled with the baker of bake/
 * it depends on, inlined here (?inline) so it compiles synchronously, in a
 * worker, on the main thread or in Node, with no files to fetch.
 */
import wasm from '../../worldgen/target/wasm32-unknown-unknown/wasm/worldgen.wasm?inline'

// Compiled on first use, once per thread, synchronously
// (browsers allow that for a module this small even on the main thread).
let compiled: WebAssembly.Module | null = null
const wasmModule = () => (compiled ??= new WebAssembly.Module(Uint8Array.from(atob(wasm.slice(wasm.indexOf(',') + 1)), c => c.charCodeAt(0))))

/**
 * A new instance of the module, with its own memory: for baking a world
 * (chunk-baker.ts), for Coverage (occlusion.ts), or for generating a
 * forest (worldgen.ts).
 */
export function wasmInstance(): WebAssembly.Exports {
  return new WebAssembly.Instance(wasmModule()).exports
}
