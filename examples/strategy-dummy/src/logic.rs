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
    let input_bytes = unsafe { Vec::from_raw_parts(ptr, len as usize, len as usize) };

    if let Ok(candle) = serde_json::from_slice::<Candle>(&input_bytes)
        && candle.close > 150.0
    {
        let sig = Signal {
            id: "sig_wasm_001".to_string(),
            symbol: "AAPL".to_string(),
            timestamp: "2026-02-02T10:00:00Z".to_string(),
            kind: "LongEntry".to_string(),
            strategy_id: "wasm-dummy".to_string(),
            metadata: std::collections::HashMap::new(),
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
    }

    0
}
