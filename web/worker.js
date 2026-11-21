// A thread of the native frontend (native/src/threads.rs): its WebAssembly
// on the memory the page shares, doing the work the page was handed for it.
import init, { run_thread } from './native/parallax-native.js'

self.onmessage = async ({ data: [module, memory, work] }) => {
    // As much stack as std gives a thread natively.
    await init({ module_or_path: module, memory, thread_stack_size: 2 << 20 })
    run_thread(work)
}
