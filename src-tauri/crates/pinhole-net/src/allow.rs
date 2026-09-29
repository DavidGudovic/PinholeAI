//! Host allow-list (CLAUDE.md privacy rule 5).
//!
//! * **Primary hosts** (and their subdomains) may be requested directly.
//! * **CDN hosts** are redirect-only: a hop to a CDN host is accepted only when
//!   the redirect chain *started* on that CDN's owner (see [`CDN_OWNERS`]), e.g.
//!   an R2 presigned URL is only followed when the request began on civitai.com.
//! * Everything is `https` on the default port, without userinfo.
//!
//! Where the CDN hosts come from (checked 2026-09):
//! * Hugging Face `/resolve/` answers 302 to its storage/CDN hosts, all under
//!   `hf.co`: Xet `cas-bridge.xethub.hf.co` (EU: `*.xethub-eu.hf.co`), CloudFront
//!   `us.aws.cdn.hf.co` / `eu.aws.cdn.hf.co`, legacy LFS `cdn-lfs.hf.co`,
//!   `cdn-lfs-us-1.hf.co`, `cdn-lfs-eu-1.hf.co` (older `cdn-lfs*.huggingface.co`
//!   are covered by the primary `huggingface.co`). HF keeps adding hosts, so the
//!   whole `hf.co` suffix is accepted — but only by redirect from huggingface.co.
//! * GitHub release downloads answer 302 to `release-assets.githubusercontent.com`
//!   (current; observed on a stable-diffusion.cpp asset) and formerly
//!   `objects.githubusercontent.com`. `codeload.github.com` is a primary subdomain.
//!   Other `*.githubusercontent.com` hosts (raw, user content, gists) stay blocked.
//! * CivitAI `/api/download/models/<id>` answers 307 to a presigned Cloudflare R2
//!   URL (`civitai-delivery-worker-prod.<account>.r2.cloudflarestorage.com`) or to
//!   `b2.civitai.com`; preview images live on `image.civitai.com`. Both
//!   `*.civitai.com` hosts are primary subdomains.

use std::net::Ipv4Addr;

use url::{Host, Url};

use crate::NetError;

/// Hosts (and their subdomains) that Pinhole may call directly.
pub const PRIMARY_HOSTS: &[&str] = &["civitai.com", "huggingface.co", "github.com"];

/// Download CDNs (and their subdomains), accepted only as redirect targets.
pub const CDN_HOSTS: &[&str] = &[
    "hf.co",
    "release-assets.githubusercontent.com",
    "objects.githubusercontent.com",
    "r2.cloudflarestorage.com",
];

/// `(cdn, owner)`: a CDN host is only reachable by a redirect chain whose first
/// URL was on `owner` (or a subdomain of it).
pub const CDN_OWNERS: &[(&str, &str)] = &[
    ("hf.co", "huggingface.co"),
    ("release-assets.githubusercontent.com", "github.com"),
    ("objects.githubusercontent.com", "github.com"),
    ("r2.cloudflarestorage.com", "civitai.com"),
];

/// Maximum redirect hops followed for one request.
pub const MAX_REDIRECTS: usize = 10;

/// `true` if `host` is `domain` or a subdomain of it.
pub fn host_matches(host: &str, domain: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let domain = domain.trim_end_matches('.').to_ascii_lowercase();
    !domain.is_empty() && (host == domain || host.ends_with(&format!(".{domain}")))
}

pub fn is_primary(host: &str) -> bool {
    PRIMARY_HOSTS.iter().any(|d| host_matches(host, d))
}

pub fn is_cdn(host: &str) -> bool {
    CDN_HOSTS.iter().any(|d| host_matches(host, d))
}

/// `true` if the CDN host `cdn_host` may be reached by a redirect chain that
/// started at `origin_host`.
pub fn cdn_allowed_from(cdn_host: &str, origin_host: &str) -> bool {
    CDN_OWNERS
        .iter()
        .any(|(cdn, owner)| host_matches(cdn_host, cdn) && host_matches(origin_host, owner))
}

/// Per-client rules. Production clients never allow plain http.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Rules {
    /// Tests only: `http://127.0.0.1:<port>` acts as a primary host and
    /// `http://localhost:<port>` as a redirect-only CDN owned by 127.0.0.1.
    pub loopback_http: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HostKind {
    Primary,
    Cdn,
}

const TEST_PRIMARY: &str = "127.0.0.1";
const TEST_CDN: &str = "localhost";

/// Classify a URL against the allow-list. Errors never contain the path or
/// query (they may carry tokens), only the host.
pub(crate) fn classify(url: &Url, rules: Rules) -> Result<HostKind, NetError> {
    if !url.username().is_empty() || url.password().is_some() {
        return Err(NetError::BadUrl(
            "URLs with credentials are not allowed".into(),
        ));
    }
    match url.scheme() {
        "https" => {
            let host = match url.host() {
                Some(Host::Domain(d)) => d,
                Some(other) => return Err(NetError::HostNotAllowed(other.to_string())),
                None => return Err(NetError::BadUrl("URL has no host".into())),
            };
            if let Some(port) = url.port() {
                return Err(NetError::HostNotAllowed(format!("{host}:{port}")));
            }
            if is_primary(host) {
                Ok(HostKind::Primary)
            } else if is_cdn(host) {
                Ok(HostKind::Cdn)
            } else {
                Err(NetError::HostNotAllowed(host.to_string()))
            }
        }
        "http" if rules.loopback_http => match url.host() {
            Some(Host::Ipv4(ip)) if ip == Ipv4Addr::LOCALHOST => Ok(HostKind::Primary),
            Some(Host::Domain(TEST_CDN)) => Ok(HostKind::Cdn),
            _ => Err(NetError::BadUrl("only https URLs are allowed".into())),
        },
        _ => Err(NetError::BadUrl("only https URLs are allowed".into())),
    }
}

/// Decide whether a redirect hop to `next` may be followed. `previous` is the
/// chain so far (first element = the original request URL).
pub(crate) fn check_redirect(
    next: &Url,
    previous: &[Url],
    offline: bool,
    rules: Rules,
) -> Result<(), NetError> {
    if offline {
        return Err(NetError::Offline);
    }
    if previous.len() > MAX_REDIRECTS {
        return Err(NetError::Transport("too many redirects".into()));
    }
    match classify(next, rules)? {
        HostKind::Primary => Ok(()),
        HostKind::Cdn => {
            let host = next.host_str().unwrap_or_default();
            let origin = previous
                .first()
                .and_then(|u| u.host_str())
                .unwrap_or_default();
            let allowed = if rules.loopback_http && host == TEST_CDN {
                origin == TEST_PRIMARY
            } else {
                cdn_allowed_from(host, origin)
            };
            if allowed {
                Ok(())
            } else {
                Err(NetError::HostNotAllowed(host.to_string()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn primary_matrix() {
        for ok in [
            "civitai.com",
            "www.civitai.com",
            "image.civitai.com",
            "b2.civitai.com",
            "huggingface.co",
            "cdn-lfs.huggingface.co",
            "github.com",
            "codeload.github.com",
            "API.GitHub.com",
            "huggingface.co.",
        ] {
            assert!(is_primary(ok), "{ok} should be primary");
        }
        for bad in [
            "huggingface.co.evil.com",
            "evilcivitai.com",
            "civitai.com.evil",
            "civitai.co",
            "notgithub.com",
            "github.co",
            "hf.co",
            "cas-bridge.xethub.hf.co",
            "githubusercontent.com",
            "raw.githubusercontent.com",
            "release-assets.githubusercontent.com",
            "r2.cloudflarestorage.com",
            "example.com",
            "",
            "com",
        ] {
            assert!(!is_primary(bad), "{bad} must not be primary");
        }
    }

    #[test]
    fn cdn_matrix() {
        for ok in [
            "hf.co",
            "cdn-lfs.hf.co",
            "cdn-lfs-us-1.hf.co",
            "cas-bridge.xethub.hf.co",
            "us.aws.cdn.hf.co",
            "release-assets.githubusercontent.com",
            "objects.githubusercontent.com",
            "civitai-delivery-worker-prod.5ac0637cfd0766c97916cefa3764fbdf.r2.cloudflarestorage.com",
        ] {
            assert!(is_cdn(ok), "{ok} should be a CDN");
        }
        for bad in [
            "evilhf.co",
            "hf.co.evil.com",
            "raw.githubusercontent.com",
            "githubusercontent.com",
            "evil-release-assets.githubusercontent.com",
            "cloudflarestorage.com",
            "r2.cloudflarestorage.com.evil.com",
            "civitai.com",
        ] {
            assert!(!is_cdn(bad), "{bad} must not be a CDN");
        }
    }

    #[test]
    fn cdn_owner_pairing() {
        assert!(cdn_allowed_from(
            "cas-bridge.xethub.hf.co",
            "huggingface.co"
        ));
        assert!(cdn_allowed_from(
            "release-assets.githubusercontent.com",
            "github.com"
        ));
        assert!(cdn_allowed_from(
            "objects.githubusercontent.com",
            "api.github.com"
        ));
        assert!(cdn_allowed_from(
            "x.acct.r2.cloudflarestorage.com",
            "civitai.com"
        ));
        // A CDN is only reachable from its own owner.
        assert!(!cdn_allowed_from(
            "x.acct.r2.cloudflarestorage.com",
            "github.com"
        ));
        assert!(!cdn_allowed_from("cas-bridge.xethub.hf.co", "civitai.com"));
        assert!(!cdn_allowed_from(
            "release-assets.githubusercontent.com",
            "huggingface.co"
        ));
        assert!(!cdn_allowed_from("example.com", "civitai.com"));
    }

    #[test]
    fn classify_rules() {
        let prod = Rules::default();
        assert_eq!(
            classify(&u("https://civitai.com/api/v1/models"), prod).unwrap(),
            HostKind::Primary
        );
        assert_eq!(
            classify(&u("https://huggingface.co:443/x"), prod).unwrap(),
            HostKind::Primary
        );
        assert_eq!(
            classify(&u("https://cdn-lfs.hf.co/x"), prod).unwrap(),
            HostKind::Cdn
        );
        assert!(matches!(
            classify(&u("http://civitai.com/"), prod),
            Err(NetError::BadUrl(_))
        ));
        assert!(matches!(
            classify(&u("ftp://github.com/"), prod),
            Err(NetError::BadUrl(_))
        ));
        assert!(matches!(
            classify(&u("https://user:pw@github.com/"), prod),
            Err(NetError::BadUrl(_))
        ));
        assert!(matches!(
            classify(&u("https://github.com:8443/"), prod),
            Err(NetError::HostNotAllowed(_))
        ));
        assert!(matches!(
            classify(&u("https://140.82.112.3/"), prod),
            Err(NetError::HostNotAllowed(_))
        ));
        assert!(matches!(
            classify(&u("https://[::1]/"), prod),
            Err(NetError::HostNotAllowed(_))
        ));
        assert!(matches!(
            classify(&u("http://127.0.0.1:8080/"), prod),
            Err(NetError::BadUrl(_))
        ));
        // Unicode look-alike (Cyrillic 'а') is punycoded by the URL parser and rejected.
        assert!(matches!(
            classify(&u("https://huggingfаce.co/"), prod),
            Err(NetError::HostNotAllowed(_))
        ));
        // The error names only the host, never the path/query.
        match classify(&u("https://evil.com/p?token=secret"), prod) {
            Err(NetError::HostNotAllowed(h)) => assert_eq!(h, "evil.com"),
            other => panic!("unexpected {other:?}"),
        }

        let test = Rules {
            loopback_http: true,
        };
        assert_eq!(
            classify(&u("http://127.0.0.1:9/"), test).unwrap(),
            HostKind::Primary
        );
        assert_eq!(
            classify(&u("http://localhost:9/"), test).unwrap(),
            HostKind::Cdn
        );
        assert!(classify(&u("http://127.0.0.2:9/"), test).is_err());
        assert!(classify(&u("http://example.com/"), test).is_err());
    }

    #[test]
    fn redirect_rules() {
        let prod = Rules::default();
        let hf = [u(
            "https://huggingface.co/org/repo/resolve/main/model.safetensors",
        )];
        let civ = [u("https://civitai.com/api/download/models/1")];
        // CDN reached from its owner.
        check_redirect(
            &u("https://cas-bridge.xethub.hf.co/xet-bridge-us/abc?sig=1"),
            &hf,
            false,
            prod,
        )
        .unwrap();
        check_redirect(
            &u("https://b.acct.r2.cloudflarestorage.com/model/1?X-Amz=1"),
            &civ,
            false,
            prod,
        )
        .unwrap();
        // Primary → primary is always fine.
        check_redirect(
            &u("https://huggingface.co/api/resolve-cache/x"),
            &hf,
            false,
            prod,
        )
        .unwrap();
        // CDN from the wrong owner.
        assert!(matches!(
            check_redirect(
                &u("https://b.acct.r2.cloudflarestorage.com/x"),
                &hf,
                false,
                prod
            ),
            Err(NetError::HostNotAllowed(_))
        ));
        // Disallowed host, downgrade to http.
        assert!(matches!(
            check_redirect(&u("https://evil.com/"), &hf, false, prod),
            Err(NetError::HostNotAllowed(_))
        ));
        assert!(matches!(
            check_redirect(&u("http://cdn-lfs.hf.co/"), &hf, false, prod),
            Err(NetError::BadUrl(_))
        ));
        // Offline wins over everything.
        assert!(matches!(
            check_redirect(&u("https://huggingface.co/"), &hf, true, prod),
            Err(NetError::Offline)
        ));
        // Hop limit.
        let chain: Vec<Url> = (0..=MAX_REDIRECTS)
            .map(|i| u(&format!("https://github.com/{i}")))
            .collect();
        assert!(check_redirect(
            &u("https://github.com/x"),
            &chain[..MAX_REDIRECTS],
            false,
            prod
        )
        .is_ok());
        assert!(matches!(
            check_redirect(&u("https://github.com/x"), &chain, false, prod),
            Err(NetError::Transport(_))
        ));

        let test = Rules {
            loopback_http: true,
        };
        let local = [u("http://127.0.0.1:1/start")];
        check_redirect(&u("http://localhost:2/file"), &local, false, test).unwrap();
        assert!(check_redirect(&u("http://localhost:2/file"), &hf, false, test).is_err());
    }
}
