//! HTTPS calls to the API server: the leader Lease and the StatefulSet's scale subresource.
//!
//! Authentication is the pod's projected service account (token + CA), so nothing but the two
//! objects the chart's Role grants is reachable.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::json;
use ureq::tls::{Certificate, PemItem, RootCerts, TlsConfig};

use super::time::{micro_time, now_secs, parse_rfc3339};
use super::*;

#[derive(Clone)]
pub(super) struct Api {
    agent: ureq::Agent,
    base: String,
    token: PathBuf,
    /// Lease holder identity: this matchmaker pod's name.
    pub(super) identity: String,
    namespace: String,
}

impl Api {
    pub(super) fn from_env(cfg: &ScaleConfig) -> Result<Self, String> {
        let host = std::env::var("KUBERNETES_SERVICE_HOST")
            .map_err(|_| "KUBERNETES_SERVICE_HOST is not set (not running in a cluster?)")?;
        let port = std::env::var("KUBERNETES_SERVICE_PORT").unwrap_or_else(|_| "443".into());
        let ca_path = sa_dir().join("ca.crt");
        let ca = std::fs::read(&ca_path).map_err(|e| format!("{}: {e}", ca_path.display()))?;
        let certs: Vec<Certificate<'static>> = ureq::tls::parse_pem(&ca)
            .filter_map(|item| match item {
                Ok(PemItem::Certificate(cert)) => Some(cert),
                _ => None,
            })
            .collect();
        if certs.is_empty() {
            return Err(format!("{} holds no certificate", ca_path.display()));
        }
        let tls = TlsConfig::builder()
            .root_certs(RootCerts::new_with_certs(&certs))
            .build();
        let agent = ureq::Agent::config_builder()
            .tls_config(tls)
            // 4xx/5xx are answers to read, not errors to throw.
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(5)))
            .build()
            .into();
        let identity = std::env::var("HOSTNAME")
            .ok()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| format!("matchmaker-{}", std::process::id()));
        Ok(Self {
            agent,
            base: format!("https://{host}:{port}"),
            token: sa_dir().join("token"),
            identity,
            namespace: cfg.namespace.clone(),
        })
    }

    fn lease_url(&self, cfg: &ScaleConfig) -> String {
        format!(
            "{}/apis/coordination.k8s.io/v1/namespaces/{}/leases/{}",
            self.base,
            self.namespace,
            cfg.lease_name()
        )
    }

    fn scale_url(&self, cfg: &ScaleConfig) -> String {
        format!(
            "{}/apis/apps/v1/namespaces/{}/statefulsets/{}/scale",
            self.base, self.namespace, cfg.statefulset
        )
    }

    /// Projected service-account tokens rotate, so read it per request (it is a tiny file).
    fn token(&self) -> Result<String, String> {
        std::fs::read_to_string(&self.token)
            .map(|token| token.trim().to_owned())
            .map_err(|e| format!("{}: {e}", self.token.display()))
    }

    fn get(&self, url: &str) -> Result<(u16, Option<serde_json::Value>), String> {
        let token = self.token()?;
        let mut response = self
            .agent
            .get(url)
            .header("Authorization", format!("Bearer {token}"))
            .call()
            .map_err(|e| format!("GET {url}: {e}"))?;
        let status = response.status().as_u16();
        Ok((status, response.body_mut().read_json().ok()))
    }

    fn send(&self, method: &str, url: &str, body: &serde_json::Value) -> Result<u16, String> {
        let token = self.token()?;
        let authorization = format!("Bearer {token}");
        let response = match method {
            "POST" => self
                .agent
                .post(url)
                .header("Authorization", authorization)
                .send_json(body),
            _ => self
                .agent
                .patch(url)
                .header("Authorization", authorization)
                .header("Content-Type", "application/merge-patch+json")
                .send_json(body),
        };
        let response = response.map_err(|e| format!("{method} {url}: {e}"))?;
        Ok(response.status().as_u16())
    }

    /// Renew the Lease, or take it over once the holder went quiet. `true` while we hold it.
    pub(super) fn renew_lease(&self, cfg: &ScaleConfig) -> Result<bool, String> {
        let url = self.lease_url(cfg);
        let now = now_secs();
        let renew = micro_time(now);
        let (status, body) = self.get(&url)?;
        match status {
            200 => {
                let spec = body
                    .as_ref()
                    .and_then(|body| body.get("spec"))
                    .cloned()
                    .unwrap_or_default();
                let holder = spec
                    .get("holderIdentity")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default();
                let mine = holder == self.identity;
                // A Lease with no renewTime was never renewed by its holder; an unreadable one is
                // not a free lease (wait, rather than split leadership).
                let expired = match spec.get("renewTime").and_then(|value| value.as_str()) {
                    Some(text) => parse_rfc3339(text).is_some_and(|at| at + LEASE_SECONDS <= now),
                    None => true,
                };
                if !mine && !expired {
                    return Ok(false);
                }
                let mut spec = json!({
                    "holderIdentity": self.identity,
                    "leaseDurationSeconds": LEASE_SECONDS,
                    "renewTime": renew,
                });
                if !mine {
                    let transitions = spec
                        .get("leaseTransitions")
                        .and_then(|value| value.as_u64())
                        .unwrap_or_default()
                        + 1;
                    spec["acquireTime"] = json!(renew);
                    spec["leaseTransitions"] = json!(transitions);
                }
                let status = self.send("PATCH", &url, &json!({ "spec": spec }))?;
                if status == 200 {
                    Ok(true)
                } else {
                    Err(format!("PATCH {url}: HTTP {status}"))
                }
            }
            404 => {
                let url = format!(
                    "{}/apis/coordination.k8s.io/v1/namespaces/{}/leases",
                    self.base, self.namespace
                );
                let body = json!({
                    "apiVersion": "coordination.k8s.io/v1",
                    "kind": "Lease",
                    "metadata": { "name": cfg.lease_name(), "namespace": self.namespace },
                    "spec": {
                        "holderIdentity": self.identity,
                        "leaseDurationSeconds": LEASE_SECONDS,
                        "acquireTime": renew,
                        "renewTime": renew,
                        "leaseTransitions": 0,
                    },
                });
                let status = self.send("POST", &url, &body)?;
                match status {
                    200 | 201 => Ok(true),
                    // Another replica created it first; stand by until the next tick.
                    409 => Ok(false),
                    other => Err(format!("POST {url}: HTTP {other}")),
                }
            }
            other => Err(format!("GET {url}: HTTP {other}")),
        }
    }

    pub(super) fn replicas(&self, cfg: &ScaleConfig) -> Result<u32, String> {
        let url = self.scale_url(cfg);
        let (status, body) = self.get(&url)?;
        if status != 200 {
            return Err(format!("GET {url}: HTTP {status}"));
        }
        body.as_ref()
            .and_then(|body| body.pointer("/spec/replicas"))
            .and_then(|value| value.as_u64())
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| format!("GET {url}: no spec.replicas"))
    }

    pub(super) fn set_replicas(&self, cfg: &ScaleConfig, replicas: u32) -> Result<(), String> {
        let url = self.scale_url(cfg);
        let status = self.send("PATCH", &url, &json!({ "spec": { "replicas": replicas } }))?;
        if (200..300).contains(&status) {
            Ok(())
        } else {
            Err(format!("PATCH {url} replicas={replicas}: HTTP {status}"))
        }
    }
}
