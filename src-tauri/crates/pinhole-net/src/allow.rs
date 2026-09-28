//! Host allow-list. Primary hosts may be requested directly; CDN hosts are only
//! accepted as redirect targets from a primary host.

/// Hosts (and their subdomains) that Pinhole may call directly.
pub const PRIMARY_HOSTS: &[&str] = &["civitai.com", "huggingface.co", "github.com"];

/// Download CDNs, accepted only as redirect targets. Net agent: verify the real
/// redirect targets (HF xet/LFS CDNs, GitHub release assets, CivitAI delivery).
pub const CDN_HOSTS: &[&str] = &[
    "hf.co",
    "huggingface.co",
    "githubusercontent.com",
    "civitai.com",
    "r2.cloudflarestorage.com",
];

/// `true` if `host` is `domain` or a subdomain of it.
pub fn host_matches(host: &str, domain: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host == domain || host.ends_with(&format!(".{domain}"))
}

pub fn is_primary(host: &str) -> bool {
    PRIMARY_HOSTS.iter().any(|d| host_matches(host, d))
}

pub fn is_cdn(host: &str) -> bool {
    CDN_HOSTS.iter().any(|d| host_matches(host, d))
}
