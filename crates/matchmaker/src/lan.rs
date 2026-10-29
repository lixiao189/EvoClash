//! LAN play for `make dev LAN=1` / `--lan`.
//!
//! Browsers only expose WebTransport in a secure context: `https://`, or `http://` on
//! localhost / 127.0.0.1. A page opened as `http://192.168.x.x` cannot connect to a match,
//! so other machines get a self-signed HTTPS listener. This machine keeps plain HTTP on
//! loopback, which is already a secure context, and match servers heartbeat there too
//! (their HTTP client would reject the self-signed certificate).

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::connect_info::Connected;
use axum::serve::{IncomingStream, Listener};
use rcgen::{CertificateParams, DnType, ExtendedKeyUsagePurpose, KeyPair, KeyUsagePurpose};
use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;
use tokio_rustls::server::TlsStream;

/// HTTPS bind when `--lan` does not pass `--tls-listen`.
pub const DEFAULT_TLS_ADDR: &str = "0.0.0.0:8443";

#[derive(Debug)]
pub struct Plan {
    /// HTTP socket. `--lan` rewrites a loopback listen to `0.0.0.0` so both this machine
    /// (`127.0.0.1`) and the HTTPS listener's host can be reached; the port stays put.
    pub http: SocketAddr,
    pub tls: Option<SocketAddr>,
    /// Address put in connect tokens. Loopback unless `--lan` (or an explicit `--public-host`).
    pub public_host: IpAddr,
}

/// `detect` runs only when `--lan` needs an address and `--public-host` was omitted.
pub fn plan(
    listen: SocketAddr,
    lan: bool,
    tls_listen: Option<SocketAddr>,
    public_host: Option<IpAddr>,
    detect: impl FnOnce() -> Option<IpAddr>,
) -> Result<Plan, String> {
    let mut http = listen;
    let tls = if lan {
        Some(tls_listen.unwrap_or_else(|| {
            DEFAULT_TLS_ADDR
                .parse()
                .expect("DEFAULT_TLS_ADDR is a socket address")
        }))
    } else {
        tls_listen
    };
    if lan {
        // 0.0.0.0 still accepts 127.0.0.1, so the developer's existing URL keeps working.
        http.set_ip(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    }
    if let Some(tls) = tls
        && tls.port() == http.port()
    {
        return Err(format!(
            "HTTPS port {} collides with HTTP {}; pass --tls-listen with a different port",
            tls.port(),
            http.port()
        ));
    }
    let public_host = match public_host {
        Some(ip) => ip,
        // Advertising 127.0.0.1 is what Docker and Helm rely on when `--public-host` is omitted.
        // Only `--lan` should guess, and a wrong guess would hand every client a dead address.
        None if lan => detect().ok_or_else(|| {
            "could not detect a LAN address; pass --public-host (the address other machines use to reach this one)".to_string()
        })?,
        None => IpAddr::V4(Ipv4Addr::LOCALHOST),
    };
    Ok(Plan {
        http,
        tls,
        public_host,
    })
}

/// URL match servers heartbeat to. `0.0.0.0` is a bind address, not something to connect to
/// (macOS rejects it). The process is local, so loopback always reaches this matchmaker.
pub fn loopback_base(listen: SocketAddr) -> String {
    let ip = if listen.ip().is_unspecified() {
        IpAddr::V4(Ipv4Addr::LOCALHOST)
    } else {
        listen.ip()
    };
    format!("http://{}", SocketAddr::new(ip, listen.port()))
}

/// Outbound interface of the default route. UDP `connect` does not send a packet.
pub fn detect_lan_ip() -> Option<IpAddr> {
    let socket = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_unspecified() && !ip.is_loopback()).then_some(ip)
}

fn bracket_host(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(ip) => ip.to_string(),
        IpAddr::V6(ip) => format!("[{ip}]"),
    }
}

fn cert_names(public_host: IpAddr) -> Vec<String> {
    let mut names = vec![
        "localhost".to_string(),
        Ipv4Addr::LOCALHOST.to_string(),
        "::1".to_string(),
        public_host.to_string(),
    ];
    names.sort();
    names.dedup();
    names
}

fn dev_acceptor(public_host: IpAddr) -> TlsAcceptor {
    let mut params = CertificateParams::new(cert_names(public_host))
        .unwrap_or_else(|e| panic!("dev certificate names: {e}"));
    params
        .distinguished_name
        .push(DnType::CommonName, "evoclash-dev");
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let key = KeyPair::generate().expect("dev certificate key");
    let cert = params.self_signed(&key).expect("dev certificate signature");
    // Pin `ring`. `ServerConfig::builder()` panics when both rustls crypto features are on,
    // and the rest of the workspace already enables `ring` for WebTransport.
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("ring supports the default TLS versions")
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(cert.der().to_vec())],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der())),
        )
        .expect("dev tls config");
    // `axum::serve` picks HTTP/1 or HTTP/2 by peeking at the bytes, and that path resets
    // the connection when ALPN already selected h2. The page only needs fetch, so offer HTTP/1.1.
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    TlsAcceptor::from(Arc::new(config))
}

/// Remote address of an HTTPS connection. A separate type from [`SocketAddr`] because axum's
/// `Connected<IncomingStream<TcpListener>>` impl does not cover this listener, and a foreign
/// trait cannot be implemented for the foreign `SocketAddr` from this crate.
#[derive(Clone, Copy, Debug)]
pub struct PeerAddr(pub SocketAddr);

struct TlsListener {
    tcp: TcpListener,
    acceptor: TlsAcceptor,
}

impl Listener for TlsListener {
    type Io = TlsStream<TcpStream>;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let (stream, addr) = match self.tcp.accept().await {
                Ok(conn) => conn,
                Err(err) => {
                    eprintln!("accept on tls listener failed: {err}");
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                }
            };
            match self.acceptor.accept(stream).await {
                Ok(tls) => return (tls, addr),
                Err(err) => eprintln!("tls handshake from {addr} failed: {err}"),
            }
        }
    }

    fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.tcp.local_addr()
    }
}

impl Connected<IncomingStream<'_, TlsListener>> for PeerAddr {
    fn connect_info(stream: IncomingStream<'_, TlsListener>) -> Self {
        Self(*stream.remote_addr())
    }
}

/// Ctrl-C locally, SIGTERM from a container runtime or process manager.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}

pub async fn serve(app: Router, http: SocketAddr, tls: Option<SocketAddr>, public_host: IpAddr) {
    let listener = TcpListener::bind(http)
        .await
        .unwrap_or_else(|e| panic!("bind {http}: {e}"));
    println!("matchmaker listening on http://{http}");

    let Some(tls_addr) = tls else {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async {
            shutdown_signal().await;
            println!("shutting down; running matches keep going and will be re-adopted");
        })
        .await
        .expect("server error");
        return;
    };

    let tls_tcp = TcpListener::bind(tls_addr)
        .await
        .unwrap_or_else(|e| panic!("bind {tls_addr}: {e}"));
    println!("this machine: {}", loopback_base(http));
    println!(
        "other machines: https://{}:{} — the certificate is self-signed; in the browser choose Advanced and continue once",
        bracket_host(public_host),
        tls_addr.port()
    );
    println!("matches are advertised at {public_host}");

    let (tx, rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        shutdown_signal().await;
        println!("shutting down; running matches keep going and will be re-adopted");
        let _ = tx.send(true);
    });
    let stop = |mut rx: tokio::sync::watch::Receiver<bool>| async move {
        let _ = rx.wait_for(|stop| *stop).await;
    };
    let http_serve = axum::serve(
        listener,
        app.clone()
            .into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(stop(rx.clone()));
    let https_serve = axum::serve(
        TlsListener {
            tcp: tls_tcp,
            acceptor: dev_acceptor(public_host),
        },
        app.into_make_service_with_connect_info::<PeerAddr>(),
    )
    .with_graceful_shutdown(stop(rx));
    let (http_result, https_result) = tokio::join!(http_serve, https_serve);
    http_result.expect("server error");
    https_result.expect("server error");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(text: &str) -> SocketAddr {
        text.parse().unwrap()
    }

    #[test]
    fn plain_listen_stays_loopback() {
        let plan = plan(addr("127.0.0.1:8080"), false, None, None, || unreachable!()).unwrap();
        assert_eq!(plan.http, addr("127.0.0.1:8080"));
        assert!(plan.tls.is_none());
        assert_eq!(plan.public_host, IpAddr::V4(Ipv4Addr::LOCALHOST));
    }

    #[test]
    fn unspecified_without_lan_keeps_loopback_advertisement() {
        let plan = plan(addr("0.0.0.0:8080"), false, None, None, || unreachable!()).unwrap();
        assert_eq!(plan.http, addr("0.0.0.0:8080"));
        assert!(plan.tls.is_none());
        assert_eq!(plan.public_host, IpAddr::V4(Ipv4Addr::LOCALHOST));
        assert_eq!(loopback_base(plan.http), "http://127.0.0.1:8080");
    }

    #[test]
    fn lan_binds_every_interface_and_a_separate_https_port() {
        let plan = plan(addr("127.0.0.1:9000"), true, None, None, || {
            Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)))
        })
        .unwrap();
        assert_eq!(plan.http, addr("0.0.0.0:9000"));
        assert_eq!(plan.tls, Some(addr("0.0.0.0:8443")));
        assert_eq!(plan.public_host, IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)));
        assert_eq!(loopback_base(plan.http), "http://127.0.0.1:9000");
    }

    #[test]
    fn explicit_public_host_skips_detection() {
        let plan = plan(
            addr("127.0.0.1:8080"),
            true,
            Some(addr("0.0.0.0:9443")),
            Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 8))),
            || unreachable!(),
        )
        .unwrap();
        assert_eq!(plan.tls, Some(addr("0.0.0.0:9443")));
        assert_eq!(plan.public_host, IpAddr::V4(Ipv4Addr::new(10, 0, 0, 8)));
    }

    #[test]
    fn https_cannot_share_the_http_port() {
        let err = plan(addr("127.0.0.1:8443"), true, None, None, || {
            Some(IpAddr::V4(Ipv4Addr::LOCALHOST))
        })
        .unwrap_err();
        assert!(err.contains("8443"), "{err}");
    }

    #[test]
    fn missing_route_names_the_flag() {
        let err = plan(addr("127.0.0.1:8080"), true, None, None, || None).unwrap_err();
        assert!(err.contains("--public-host"), "{err}");
    }
}
