//! Request authorisation: bearer token, or loopback-only access protected
//! against DNS rebinding (Host and Origin must be loopback names).

use axum::http::{HeaderMap, StatusCode, header};

use super::Policy;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Access {
    /// Valid bearer token.
    Token,
    /// Unauthenticated local request.
    Local,
    Denied(StatusCode, &'static str),
}

fn strip_port(host: &str) -> &str {
    if let Some(rest) = host.strip_prefix('[') {
        return rest.split(']').next().unwrap_or("");
    }
    match host.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') && p.bytes().all(|b| b.is_ascii_digit()) => h,
        _ => host,
    }
}

pub(crate) fn is_loopback_host(host: &str) -> bool {
    let h = strip_port(host.trim()).to_ascii_lowercase();
    h == "localhost"
        || h.ends_with(".localhost")
        || h == "::1"
        || h.parse::<std::net::Ipv4Addr>()
            .is_ok_and(|ip| ip.is_loopback())
}

fn is_loopback_origin(origin: &str) -> bool {
    origin
        .split_once("://")
        .map(|(_, rest)| rest.split('/').next().unwrap_or(""))
        .is_some_and(is_loopback_host)
}

pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Decide whether a request may proceed. `lan` is true when the server
/// listens on a non-loopback address.
pub(crate) fn check(headers: &HeaderMap, policy: &Policy, lan: bool) -> Access {
    let get = |name| headers.get(name).and_then(|v| v.to_str().ok());
    if let Some(auth) = get(header::AUTHORIZATION) {
        let token = auth
            .strip_prefix("Bearer ")
            .or_else(|| auth.strip_prefix("bearer "))
            .map(str::trim);
        return match token {
            Some(t)
                if !policy.token.is_empty()
                    && constant_time_eq(t.as_bytes(), policy.token.as_bytes()) =>
            {
                Access::Token
            }
            _ => Access::Denied(StatusCode::UNAUTHORIZED, "invalid access token"),
        };
    }
    if lan {
        return Access::Denied(StatusCode::UNAUTHORIZED, "access token required");
    }
    if !get(header::HOST).is_some_and(is_loopback_host) {
        return Access::Denied(StatusCode::FORBIDDEN, "requests must address localhost");
    }
    if let Some(origin) = get(header::ORIGIN)
        && !is_loopback_origin(origin)
    {
        return Access::Denied(
            StatusCode::FORBIDDEN,
            "cross-origin requests are not allowed",
        );
    }
    Access::Local
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    fn headers(pairs: &[(header::HeaderName, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            if let Ok(v) = HeaderValue::from_str(v) {
                h.insert(k.clone(), v);
            }
        }
        h
    }

    fn policy() -> Policy {
        Policy {
            token: "secret".into(),
            allow_control: false,
            mcp: true,
        }
    }

    #[test]
    fn host_parsing() {
        assert!(is_loopback_host("localhost:8180"));
        assert!(is_loopback_host("127.0.0.1"));
        assert!(is_loopback_host("[::1]:8180"));
        assert!(!is_loopback_host("evil.example:8180"));
        assert!(!is_loopback_host("192.168.1.10:8180"));
    }

    #[test]
    fn local_access_rules() {
        let p = policy();
        assert_eq!(
            check(&headers(&[(header::HOST, "127.0.0.1:8180")]), &p, false),
            Access::Local
        );
        assert!(matches!(
            check(
                &headers(&[(header::HOST, "attacker.example:8180")]),
                &p,
                false
            ),
            Access::Denied(StatusCode::FORBIDDEN, _)
        ));
        assert!(matches!(
            check(
                &headers(&[
                    (header::HOST, "localhost:8180"),
                    (header::ORIGIN, "https://evil.example")
                ]),
                &p,
                false
            ),
            Access::Denied(StatusCode::FORBIDDEN, _)
        ));
        assert_eq!(
            check(
                &headers(&[
                    (header::HOST, "localhost:8180"),
                    (header::ORIGIN, "http://localhost:3000")
                ]),
                &p,
                false
            ),
            Access::Local
        );
    }

    #[test]
    fn token_rules() {
        let p = policy();
        let ok = headers(&[
            (header::HOST, "10.0.0.5:8180"),
            (header::AUTHORIZATION, "Bearer secret"),
        ]);
        assert_eq!(check(&ok, &p, true), Access::Token);
        let bad = headers(&[
            (header::HOST, "127.0.0.1"),
            (header::AUTHORIZATION, "Bearer nope"),
        ]);
        assert!(matches!(
            check(&bad, &p, false),
            Access::Denied(StatusCode::UNAUTHORIZED, _)
        ));
        let none = headers(&[(header::HOST, "10.0.0.5:8180")]);
        assert!(matches!(
            check(&none, &p, true),
            Access::Denied(StatusCode::UNAUTHORIZED, _)
        ));
        let empty = Policy {
            token: String::new(),
            ..policy()
        };
        assert!(matches!(
            check(
                &headers(&[(header::AUTHORIZATION, "Bearer ")]),
                &empty,
                false
            ),
            Access::Denied(StatusCode::UNAUTHORIZED, _)
        ));
    }
}
