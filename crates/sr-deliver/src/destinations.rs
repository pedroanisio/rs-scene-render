//! Post-render delivery. Credentials never live in the document: a
//! destination's `@credentials` names a profile, read from environment
//! variables `SR_CREDENTIALS_<PROFILE>_<NAME>`:
//!
//! | kind | variables |
//! |---|---|
//! | `s3` | `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_REGION` (default us-east-1), `AWS_SESSION_TOKEN`, `S3_ENDPOINT` (path-style, e.g. MinIO) |
//! | `gcs` | `GCS_TOKEN` (OAuth 2 access token), `GCS_ENDPOINT` |
//! | `azure-blob` | `AZURE_SAS` (shared access signature query) |
//! | `http-put` | `TOKEN` (bearer) |
//! | `sftp` | `PASSWORD` or `SSH_KEY` (private key file) |
//! | `webhook` | `WEBHOOK_SECRET` (HMAC-SHA256 signature header), `TOKEN` |
//!
//! Transfers run through `curl` (override with `SR_CURL`), which brings
//! TLS, HTTP/2 and SFTP. S3 requests are signed here with AWS Signature
//! Version 4.

use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};
use sr_model::model as m;

use crate::DeliverError;

/// HMAC-SHA256 (RFC 2104).
pub fn hmac(key: &[u8], msg: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(k.map(|b| b ^ 0x36));
    inner.update(msg);
    let mut outer = Sha256::new();
    outer.update(k.map(|b| b ^ 0x5c));
    outer.update(inner.finalize());
    outer.finalize().into()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// SigV4 signing key for a date (YYYYMMDD), region and service.
pub fn sigv4_key(secret: &str, date: &str, region: &str, service: &str) -> [u8; 32] {
    let k = hmac(format!("AWS4{secret}").as_bytes(), date.as_bytes());
    let k = hmac(&k, region.as_bytes());
    let k = hmac(&k, service.as_bytes());
    hmac(&k, b"aws4_request")
}

fn uri_encode(s: &str, slash: bool) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => o.push(b as char),
            b'/' if !slash => o.push('/'),
            _ => o.push_str(&format!("%{b:02X}")),
        }
    }
    o
}

/// Headers of a SigV4-signed S3 `PUT` (host, x-amz-date, x-amz-content-sha256, optional token, authorization).
#[allow(clippy::too_many_arguments)]
pub fn s3_put_headers(
    host: &str,
    path: &str,
    payload_sha256: &str,
    amz_date: &str,
    region: &str,
    key_id: &str,
    secret: &str,
    token: Option<&str>,
) -> Vec<(String, String)> {
    let date = &amz_date[..8];
    let mut headers = vec![
        ("host".to_string(), host.to_string()),
        ("x-amz-content-sha256".to_string(), payload_sha256.to_string()),
        ("x-amz-date".to_string(), amz_date.to_string()),
    ];
    if let Some(t) = token {
        headers.push(("x-amz-security-token".into(), t.into()));
    }
    headers.sort();
    let canonical_headers: String = headers.iter().map(|(k, v)| format!("{k}:{}\n", v.trim())).collect();
    let signed: String = headers.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(";");
    let canonical = format!("PUT\n{}\n\n{canonical_headers}\n{signed}\n{payload_sha256}", uri_encode(path, false));
    let scope = format!("{date}/{region}/s3/aws4_request");
    let to_sign = format!("AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}", hex(&Sha256::digest(canonical.as_bytes())));
    let sig = hex(&hmac(&sigv4_key(secret, date, region, "s3"), to_sign.as_bytes()));
    headers.push((
        "authorization".into(),
        format!("AWS4-HMAC-SHA256 Credential={key_id}/{scope}, SignedHeaders={signed}, Signature={sig}"),
    ));
    headers
}

fn amz_now() -> String {
    // UTC timestamp without a date library: civil-from-days (Howard Hinnant)
    let secs =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
    let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mth = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if mth <= 2 { 1 } else { 0 };
    format!("{y:04}{mth:02}{d:02}T{:02}{:02}{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

fn cred(profile: Option<&str>, name: &str) -> Option<String> {
    let p = profile?
        .to_ascii_uppercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect::<String>();
    std::env::var(format!("SR_CREDENTIALS_{p}_{name}")).ok().filter(|v| !v.is_empty())
}

fn curl(args: &[String], uri: &str) -> Result<String, DeliverError> {
    let tool = std::env::var("SR_CURL").unwrap_or_else(|_| "curl".into());
    let out = Command::new(&tool)
        .args(["-sS", "--fail-with-body", "--retry", "2"])
        .args(args)
        .output()
        .map_err(|e| DeliverError::Destination { uri: uri.into(), message: format!("cannot run {tool}: {e}") })?;
    if !out.status.success() {
        let msg =
            format!("{} {}", String::from_utf8_lossy(&out.stderr).trim(), String::from_utf8_lossy(&out.stdout).trim());
        return Err(DeliverError::Destination { uri: uri.into(), message: msg.trim().to_string() });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn join(base: &str, name: &str) -> String {
    if base.ends_with('/') {
        format!("{base}{name}")
    } else {
        format!("{base}/{name}")
    }
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Uploads `files` to one destination; returns their remote locations.
pub fn upload(d: &m::Destination, files: &[PathBuf], base: &Path) -> Result<Vec<String>, DeliverError> {
    let uri = d.uri.as_str();
    let profile = d.credentials.as_deref();
    let mut out = Vec::new();
    match d.kind.as_str() {
        "file" => {
            let dir = super::pipeline_resolve(base, uri);
            std::fs::create_dir_all(&dir)?;
            for f in files {
                let to = dir.join(file_name(f));
                if to != *f {
                    std::fs::copy(f, &to)?;
                }
                out.push(to.display().to_string());
            }
        }
        "http-put" => {
            for f in files {
                let url =
                    if files.len() == 1 && !uri.ends_with('/') { uri.to_string() } else { join(uri, &file_name(f)) };
                let mut a = vec!["-T".to_string(), f.display().to_string()];
                if let Some(t) = cred(profile, "TOKEN") {
                    a.extend(["-H".into(), format!("Authorization: Bearer {t}")]);
                }
                a.push(url.clone());
                curl(&a, &url)?;
                out.push(url);
            }
        }
        "s3" => {
            let rest = uri.strip_prefix("s3://").ok_or_else(|| DeliverError::Destination {
                uri: uri.into(),
                message: "expected s3://bucket/prefix".into(),
            })?;
            let (bucket, prefix) = rest.split_once('/').unwrap_or((rest, ""));
            let key_id = cred(profile, "AWS_ACCESS_KEY_ID").ok_or_else(|| DeliverError::Destination {
                uri: uri.into(),
                message: "missing AWS_ACCESS_KEY_ID in the credentials profile".into(),
            })?;
            let secret = cred(profile, "AWS_SECRET_ACCESS_KEY").ok_or_else(|| DeliverError::Destination {
                uri: uri.into(),
                message: "missing AWS_SECRET_ACCESS_KEY in the credentials profile".into(),
            })?;
            let region = cred(profile, "AWS_REGION").unwrap_or_else(|| "us-east-1".into());
            let token = cred(profile, "AWS_SESSION_TOKEN");
            for f in files {
                let key = format!(
                    "{}{}",
                    if prefix.is_empty() || prefix.ends_with('/') { prefix.to_string() } else { format!("{prefix}/") },
                    file_name(f)
                );
                let (scheme_host, host, path) = match cred(profile, "S3_ENDPOINT") {
                    Some(ep) => {
                        let host = ep.split("://").nth(1).unwrap_or(&ep).trim_end_matches('/').to_string();
                        (ep.trim_end_matches('/').to_string(), host, format!("/{bucket}/{key}"))
                    }
                    None => {
                        let host = format!("{bucket}.s3.{region}.amazonaws.com");
                        (format!("https://{host}"), host, format!("/{key}"))
                    }
                };
                let body = std::fs::read(f)?;
                let hash = hex(&Sha256::digest(&body));
                let headers =
                    s3_put_headers(&host, &path, &hash, &amz_now(), &region, &key_id, &secret, token.as_deref());
                let url = format!("{scheme_host}{}", uri_encode(&path, false));
                let mut a = vec!["-T".to_string(), f.display().to_string()];
                for (k, v) in headers.iter().filter(|(k, _)| k != "host") {
                    a.extend(["-H".into(), format!("{k}: {v}")]);
                }
                a.push(url);
                curl(&a, uri)?;
                out.push(format!("s3://{bucket}/{key}"));
            }
        }
        "gcs" => {
            let rest = uri.strip_prefix("gs://").or_else(|| uri.strip_prefix("gcs://")).ok_or_else(|| {
                DeliverError::Destination { uri: uri.into(), message: "expected gs://bucket/prefix".into() }
            })?;
            let (bucket, prefix) = rest.split_once('/').unwrap_or((rest, ""));
            let token = cred(profile, "GCS_TOKEN").ok_or_else(|| DeliverError::Destination {
                uri: uri.into(),
                message: "missing GCS_TOKEN in the credentials profile".into(),
            })?;
            let endpoint = cred(profile, "GCS_ENDPOINT").unwrap_or_else(|| "https://storage.googleapis.com".into());
            for f in files {
                let name = format!(
                    "{}{}",
                    if prefix.is_empty() || prefix.ends_with('/') { prefix.to_string() } else { format!("{prefix}/") },
                    file_name(f)
                );
                let url = format!(
                    "{}/upload/storage/v1/b/{bucket}/o?uploadType=media&name={}",
                    endpoint.trim_end_matches('/'),
                    uri_encode(&name, true)
                );
                curl(
                    &[
                        "--data-binary".into(),
                        format!("@{}", f.display()),
                        "-H".into(),
                        format!("Authorization: Bearer {token}"),
                        "-H".into(),
                        "Content-Type: application/octet-stream".into(),
                        url,
                    ],
                    uri,
                )?;
                out.push(format!("gs://{bucket}/{name}"));
            }
        }
        "azure-blob" => {
            let sas = cred(profile, "AZURE_SAS").ok_or_else(|| DeliverError::Destination {
                uri: uri.into(),
                message: "missing AZURE_SAS in the credentials profile".into(),
            })?;
            for f in files {
                let blob = join(uri, &file_name(f));
                let url = format!("{blob}?{}", sas.trim_start_matches('?'));
                curl(
                    &["-T".into(), f.display().to_string(), "-H".into(), "x-ms-blob-type: BlockBlob".into(), url],
                    uri,
                )?;
                out.push(blob);
            }
        }
        "sftp" => {
            let mut auth = Vec::new();
            if let Some(k) = cred(profile, "SSH_KEY") {
                auth.extend(["--key".to_string(), k]);
            }
            if let Some(pw) = cred(profile, "PASSWORD") {
                let user = uri
                    .split("://")
                    .nth(1)
                    .and_then(|r| r.split_once('@'))
                    .map(|(u, _)| u.to_string())
                    .unwrap_or_default();
                auth.extend(["-u".to_string(), format!("{user}:{pw}")]);
            }
            for f in files {
                let url = join(uri, &file_name(f));
                let mut a = auth.clone();
                a.extend(["--ftp-create-dirs".into(), "-T".into(), f.display().to_string(), url.clone()]);
                curl(&a, uri)?;
                out.push(url);
            }
        }
        "webhook" => {}
        k => {
            return Err(DeliverError::Destination { uri: uri.into(), message: format!("unknown destination kind {k}") })
        }
    }
    Ok(out)
}

/// Posts the completion notification.
pub fn notify(
    d: &m::Destination,
    output: &m::Output,
    files: &[PathBuf],
    uploaded: &[String],
) -> Result<(), DeliverError> {
    let entries: Vec<serde_json::Value> = files
        .iter()
        .map(|f| {
            let bytes = std::fs::read(f).unwrap_or_default();
            serde_json::json!({ "name": file_name(f), "bytes": bytes.len(), "sha256": hex(&Sha256::digest(&bytes)) })
        })
        .collect();
    let body = serde_json::json!({
        "event": "render.completed",
        "output": output.id,
        "codec": output.codec.as_str(),
        "files": entries,
        "locations": uploaded,
    })
    .to_string();
    let profile = d.credentials.as_deref();
    let mut a = vec![
        "-X".to_string(),
        "POST".into(),
        "-H".into(),
        "Content-Type: application/json".into(),
        "--data-binary".into(),
        body.clone(),
    ];
    if let Some(secret) = cred(profile, "WEBHOOK_SECRET") {
        a.extend([
            "-H".into(),
            format!("X-Scene-Render-Signature: sha256={}", hex(&hmac(secret.as_bytes(), body.as_bytes()))),
        ]);
    }
    if let Some(t) = cred(profile, "TOKEN") {
        a.extend(["-H".into(), format!("Authorization: Bearer {t}")]);
    }
    a.push(d.uri.clone());
    curl(&a, &d.uri).map(|_| ())
}

/// Uploads to every destination, then posts webhooks with the resulting locations.
pub fn deliver_all(
    dests: &[&m::Destination],
    files: &[PathBuf],
    output: &m::Output,
    base: &Path,
) -> Result<Vec<String>, DeliverError> {
    let mut locations = Vec::new();
    for d in dests.iter().filter(|d| d.kind.as_str() != "webhook") {
        locations.extend(upload(d, files, base)?);
    }
    for d in dests.iter().filter(|d| d.kind.as_str() == "webhook") {
        notify(d, output, files, &locations)?;
    }
    Ok(locations)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_and_sigv4_match_published_vectors() {
        // RFC 4231 test case 2
        assert_eq!(
            hex(&hmac(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        // AWS SigV4 documentation example signing key
        assert_eq!(
            hex(&sigv4_key("wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY", "20150830", "us-east-1", "iam")),
            "c4afb1cc5771d871763a393e44b703571b55cc28424d1a5e86da6ed3c154a4b9"
        );
        let h = s3_put_headers(
            "b.s3.us-east-1.amazonaws.com",
            "/a b.mp4",
            "UNSIGNED",
            "20240101T000000Z",
            "us-east-1",
            "AKID",
            "secret",
            None,
        );
        let auth = &h.iter().find(|(k, _)| k == "authorization").unwrap().1;
        assert!(auth.starts_with("AWS4-HMAC-SHA256 Credential=AKID/20240101/us-east-1/s3/aws4_request, SignedHeaders=host;x-amz-content-sha256;x-amz-date, Signature="));
        assert_eq!(amz_now().len(), 16);
    }
}
