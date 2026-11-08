//! A Rust-to-Wasm source policy action using only Gitr's explicit byte ABI.
//! No WASI, host filesystem, network, environment or subprocess imports.
#[link(wasm_import_module = "gitr_actions_v1")]
unsafe extern "C" {
    fn input_len() -> i32;
    fn input_read(offset: i32, pointer: *mut u8, length: i32) -> i32;
    fn output_write(pointer: *const u8, length: i32) -> i32;
}

#[unsafe(no_mangle)]
pub extern "C" fn run() -> i32 {
    // SAFETY: These imports are supplied only by the documented Gitr host. Buffers
    // below remain allocated for each call and lengths match their allocations.
    let length = unsafe { input_len() };
    if !(0..=1024 * 1024).contains(&length) {
        return 2;
    }
    let mut bytes = vec![0; length as usize];
    if unsafe { input_read(0, bytes.as_mut_ptr(), length) } != length {
        return 3;
    }
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return 4;
    };
    if text.lines().any(|line| line.ends_with([' ', '\t'])) {
        return 1;
    }
    let report = format!(
        "{{\"bytes\":{},\"lines\":{},\"trailing_whitespace\":false}}\n",
        bytes.len(),
        text.lines().count()
    );
    if unsafe { output_write(report.as_ptr(), report.len() as i32) } != report.len() as i32 {
        return 5;
    }
    0
}
