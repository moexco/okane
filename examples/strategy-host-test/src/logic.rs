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
    let usize_len = usize::try_from(len).unwrap_or(0);
    let mut buf = Vec::with_capacity(usize_len);
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

/// # Safety
/// `ptr` must point to a valid, allocated buffer of at least `len` bytes.
#[unsafe(no_mangle)]
pub unsafe fn on_candle(ptr: *mut u8, len: i32) -> i32 {
    let usize_len = usize::try_from(len).unwrap_or(0);
    let _input_bytes = unsafe { Vec::from_raw_parts(ptr, usize_len, usize_len) };

    let now = unsafe { host_now() };

    let sym = b"AAPL";
    let tf = b"1m";
    let mut out_buf = vec![0u8; 1024];

    let bytes_written = unsafe {
        host_fetch_history(
            i32::try_from(sym.as_ptr() as usize).unwrap_or(0), i32::try_from(sym.len()).unwrap_or(0),
            i32::try_from(tf.as_ptr() as usize).unwrap_or(0), i32::try_from(tf.len()).unwrap_or(0),
            5,
            i32::try_from(out_buf.as_mut_ptr() as usize).unwrap_or(0),
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
        let len_u32 = match u32::try_from(json.len()) {
            Ok(l) => l,
            Err(_) => return 0,
        };
        let len_bytes = len_u32.to_le_bytes();
        let mut out = Vec::with_capacity(4 + json.len());
        out.extend_from_slice(&len_bytes);
        out.append(&mut json);

        let out_ptr = out.as_mut_ptr();
        std::mem::forget(out);
        return i32::try_from(out_ptr as usize).unwrap_or_default();
    }

    0
}
