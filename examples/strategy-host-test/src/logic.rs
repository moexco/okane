use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct Candle {
    pub close: f64,
}

#[derive(Serialize)]
pub struct Signal {
    pub id: String,
    pub symbol: String,
    pub timestamp: String,
    pub kind: String,
    pub strategy_id: String,
    pub metadata: std::collections::HashMap<String, String>,
}

// Host functions (pure wasm interface from env namespace)
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn host_now() -> i64;
    #[allow(dead_code)]
    fn host_log(level: i32, ptr: i32, len: i32);
    fn host_fetch_history(sym_ptr: i32, sym_len: i32, tf_ptr: i32, tf_len: i32, limit: i32, out_ptr: i32) -> i32;
}

#[unsafe(no_mangle)]
pub fn alloc(len: i32) -> *mut u8 {
    let mut buf = Vec::with_capacity(len as usize);
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

/// # Safety
/// `ptr` must point to a valid, allocated buffer of at least `len` bytes.
#[unsafe(no_mangle)]
pub unsafe fn on_candle(ptr: *mut u8, len: i32) -> i32 {
    let _input_bytes = unsafe { Vec::from_raw_parts(ptr, len as usize, len as usize) };

    let now = unsafe { host_now() };

    let sym = b"AAPL";
    let tf = b"1m";
    let mut out_buf = vec![0u8; 1024];

    let bytes_written = unsafe {
        host_fetch_history(
            sym.as_ptr() as i32, sym.len() as i32,
            tf.as_ptr() as i32, tf.len() as i32,
            5,
            out_buf.as_mut_ptr() as i32,
        )
    };

    let history_len = if bytes_written > 4 { 5 } else { 0 };

    let mut metadata = std::collections::HashMap::new();
    metadata.insert("logical_now".to_string(), now.to_string());
    metadata.insert("history_count".to_string(), history_len.to_string());

    let sig = Signal {
        id: "host_test_wasm_001".to_string(),
        symbol: "AAPL".to_string(),
        timestamp: "2026-02-02T10:00:00Z".to_string(),
        kind: "Info".to_string(),
        strategy_id: "wasm-host-test".to_string(),
        metadata,
    };

    if let Ok(mut json) = serde_json::to_vec(&sig) {
        let len_bytes = (json.len() as u32).to_le_bytes();
        let mut out = Vec::with_capacity(4 + json.len());
        out.extend_from_slice(&len_bytes);
        out.append(&mut json);

        let out_ptr = out.as_mut_ptr();
        std::mem::forget(out);
        return out_ptr as i32;
    }

    0
}
