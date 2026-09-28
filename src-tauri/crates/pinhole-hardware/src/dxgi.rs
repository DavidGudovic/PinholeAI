//! Windows: list display adapters with DXGI (`CreateDXGIFactory1` →
//! `EnumAdapters1` → `GetDesc1`). No COM initialisation is needed for DXGI.

use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE};

use crate::DxgiAdapter;

/// Upper bound on adapters we look at (guards against a misbehaving driver).
const MAX_ADAPTERS: u32 = 64;

pub(crate) fn adapters() -> Vec<DxgiAdapter> {
    let mut out = Vec::new();
    // SAFETY: plain DXGI calls on interfaces owned by this function; every
    // returned object is released when its wrapper drops.
    let factory: IDXGIFactory1 = match unsafe { CreateDXGIFactory1() } {
        Ok(f) => f,
        Err(_) => return out,
    };
    for i in 0..MAX_ADAPTERS {
        // Fails with DXGI_ERROR_NOT_FOUND after the last adapter.
        let Ok(adapter) = (unsafe { factory.EnumAdapters1(i) }) else { break };
        let Ok(desc) = (unsafe { adapter.GetDesc1() }) else { continue };
        let len = desc.Description.iter().position(|&c| c == 0).unwrap_or(desc.Description.len());
        out.push(DxgiAdapter {
            vendor_id: desc.VendorId,
            description: String::from_utf16_lossy(&desc.Description[..len]),
            dedicated_video_memory: desc.DedicatedVideoMemory as u64,
            software: desc.Flags & (DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) != 0,
        });
    }
    out
}
