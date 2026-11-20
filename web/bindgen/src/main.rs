//! pnpm bindgen: the native frontend's WebAssembly (pnpm wasm) with the
//! JavaScript that binds it to a browser, wgpu's and winit's calls into
//! the page, in web/native/ for native.html to load.

use std::path::Path;

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut bindgen = wasm_bindgen_cli_support::Bindgen::new();
    bindgen.input_path(root.join("target/wasm32-unknown-unknown/wasm/parallax-native.wasm")).web(true).expect("a web target");
    bindgen.generate(root.join("web/native")).expect("bindings");
}
