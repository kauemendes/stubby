//! Turn `kubernetes.io/dockerconfigjson` secret payloads into a
//! `(username, password)` credential for a given registry host. Pure — the
//! caller fetches the secret bytes from the API.
use base64::Engine;
use serde_json::Value;

/// Find credentials for `host` across the provided dockerconfigjson blobs.
/// Returns the first match; `None` if none apply or all are malformed.
pub fn credentials_for(host: &str, dockerconfigs: &[Vec<u8>]) -> Option<(String, String)> {
    for raw in dockerconfigs {
        let Ok(cfg): Result<Value, _> = serde_json::from_slice(raw) else {
            continue;
        };
        let Some(auths) = cfg.get("auths").and_then(Value::as_object) else {
            continue;
        };
        for (entry_host, entry) in auths {
            if !host_matches(host, entry_host) {
                continue;
            }
            if let Some(creds) = decode_entry(entry) {
                return Some(creds);
            }
        }
    }
    None
}

/// A registry host matches a dockerconfig key if they're equal, or both are
/// Docker Hub under any of its aliases.
fn host_matches(host: &str, entry_host: &str) -> bool {
    if host == entry_host {
        return true;
    }
    let docker_aliases = [
        "docker.io",
        "index.docker.io",
        "https://index.docker.io/v1/",
        "registry-1.docker.io",
    ];
    docker_aliases.contains(&host) && docker_aliases.contains(&entry_host)
}

/// Decode either an `auth` (base64 `user:pass`) or explicit `username`/`password`.
fn decode_entry(entry: &Value) -> Option<(String, String)> {
    if let Some(auth) = entry.get("auth").and_then(Value::as_str) {
        let decoded = base64::engine::general_purpose::STANDARD.decode(auth).ok()?;
        let s = String::from_utf8(decoded).ok()?;
        let (u, p) = s.split_once(':')?;
        return Some((u.to_string(), p.to_string()));
    }
    let u = entry.get("username").and_then(Value::as_str)?;
    let p = entry.get("password").and_then(Value::as_str)?;
    Some((u.to_string(), p.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn dockercfg(host: &str, user: &str, pass: &str) -> Vec<u8> {
        let auth = base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pass}"));
        serde_json::to_vec(&serde_json::json!({
            "auths": { host: { "auth": auth } }
        })).unwrap()
    }

    #[test]
    fn extracts_credentials_for_host() {
        let creds = credentials_for("ghcr.io", &[dockercfg("ghcr.io", "u", "p")]);
        assert_eq!(creds, Some(("u".to_string(), "p".to_string())));
    }

    #[test]
    fn matches_docker_io_aliases() {
        let cfg = dockercfg("https://index.docker.io/v1/", "u", "p");
        let creds = credentials_for("docker.io", &[cfg]);
        assert_eq!(creds, Some(("u".to_string(), "p".to_string())));
    }

    #[test]
    fn none_when_no_match_or_garbage() {
        assert_eq!(credentials_for("ghcr.io", &[b"not json".to_vec()]), None);
        assert_eq!(credentials_for("ghcr.io", &[dockercfg("other.io", "u", "p")]), None);
    }
}
