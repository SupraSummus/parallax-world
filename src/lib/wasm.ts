/**
 * The web's WebAssembly module, the worlds of worlds/ compiled with the
 * engine of worldgen/ and the baker of bake/ they depend on, inlined here
 * (?inline) so it compiles synchronously, in a worker, on the main thread
 * or in Node, with no files to fetch.
 */
import wasm from '../../worlds/target/wasm32-unknown-unknown/wasm/worlds.wasm?inline'

// Compiled on first use, once per thread, synchronously
// (browsers allow that for a module this small even on the main thread).
let compiled: WebAssembly.Module | null = null
const wasmModule = () => (compiled ??= new WebAssembly.Module(Uint8Array.from(atob(wasm.slice(wasm.indexOf(',') + 1)), c => c.charCodeAt(0))))

/**
 * A new instance of the module, with its own memory: for baking a world
 * (chunk-baker.ts), for Coverage (occlusion.ts), for generating a forest
 * (worldgen.ts), or for indexing grids (LayerIndex in voxel-grid.ts).
 */
export function wasmInstance(): WebAssembly.Exports {
  return new WebAssembly.Instance(wasmModule()).exports
}

// Memory may grow on any call into an instance, detaching earlier views of
// it: copyIn and copyOut view it only once called, after the call that
// gave the offset.

/** Copies `values` into an instance's memory at byte `offset`. */
export function copyIn(memory: WebAssembly.Memory, offset: number, values: Uint8Array | Int32Array | Uint32Array) {
  new Uint8Array(memory.buffer, offset, values.byteLength).set(new Uint8Array(values.buffer, values.byteOffset, values.byteLength))
}

/** A copy of `length` numbers of an instance's memory from byte `offset`, as `Type`. */
export function copyOut<T extends { slice(): T }>(memory: WebAssembly.Memory, Type: new (buffer: ArrayBuffer, offset: number, length: number) => T, offset: number, length: number): T {
  return new Type(memory.buffer, offset, length).slice()
}
