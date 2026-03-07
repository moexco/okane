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
    let usize_len = usize::try_from(len).unwrap_or(0);
    let mut buf = Vec::with_capacity(usize_len);
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

/// # Safety
/// `ptr` must point to a valid, allocated buffer of at least `len` bytes, created by `alloc`.
#[unsafe(no_mangle)]
pub unsafe fn dealloc(ptr: *mut u8, len: i32) {
    let usize_len = usize::try_from(len).unwrap_or(0);
    drop(unsafe { Vec::from_raw_parts(ptr, usize_len, usize_len) });
}

unsafe extern "C" {
    fn host_buy(
        sym_ptr: *const u8, sym_len: i32,
        price_ptr: *const u8, price_len: i32,
        vol_ptr: *const u8, vol_len: i32,
        out_ptr: *mut u8
    ) -> i32;
}

/// # Safety
/// `ptr` must point to a valid, allocated buffer of at least `len` bytes.
#[unsafe(no_mangle)]
pub unsafe fn on_candle(ptr: *mut u8, len: i32) -> i32 {
    let usize_len = usize::try_from(len).unwrap_or(0);
    let input_bytes = unsafe { std::slice::from_raw_parts(ptr, usize_len) };

    if let Ok(candle) = serde_json::from_slice::<Candle>(input_bytes)
        && candle.close > 150.0
    {
        let symbol = "AAPL";
        let price = "155.0";
        let vol = "100.0";
        
        // 分配 1024 字节用于接收响应（下单结果 JSON）
        let out_buf = alloc(1024);
        
        let res_len = unsafe {
            host_buy(
                symbol.as_ptr(), i32::try_from(symbol.len()).unwrap_or(0),
                price.as_ptr(), i32::try_from(price.len()).unwrap_or(0),
                vol.as_ptr(), i32::try_from(vol.len()).unwrap_or(0),
                out_buf
            )
        };

        if res_len > 0 {
            // 返回包含长度头部的指针包，供宿主读取和后续释放
            let usize_res_len = usize::try_from(res_len).unwrap_or(0);
            let mut out = Vec::with_capacity(4 + usize_res_len);
            out.extend_from_slice(&res_len.to_le_bytes());
            let res_bytes = unsafe { std::slice::from_raw_parts(out_buf, usize_res_len) };
            out.extend_from_slice(res_bytes);
            
            // 立即释放原始响应缓冲，因为它已被拷贝到 out 中
            unsafe { dealloc(out_buf, 1024) };

            let final_ptr = out.as_mut_ptr();
            std::mem::forget(out);
            return i32::try_from(final_ptr as usize).unwrap_or(0);
        } else {
            unsafe { dealloc(out_buf, 1024) };
        }
    }

    0
}
