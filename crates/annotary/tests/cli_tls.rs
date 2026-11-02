//! CMD-18: the CLI speaks https. A one-shot local https server (an rcgen certificate for
//! `localhost`, rustls) answers `/api/v1/health`; a client trusting that certificate completes the
//! request, and the CLI's own client (the bundled webpki roots) refuses it as an untrusted
//! certificate (exit 6, kind `tls`), never as "transport is unsecured" (no TLS provider at all,
//! before the fix).

use std::io::{Read, Write};
use std::sync::Arc;

use annotary::client::Api;
use annotary::exit::Exit;

/// Serves one https request with a fixed health body; returns the port and the certificate.
fn https_once() -> (u16, Vec<u8>, std::thread::JoinHandle<()>) {
    let key = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let cert_der = key.cert.der().to_vec();
    let key_der = key.signing_key.serialize_der();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![rustls::pki_types::CertificateDer::from(cert_der.clone())],
            rustls::pki_types::PrivateKeyDer::Pkcs8(key_der.into()),
        )
        .unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        let (tcp, _) = listener.accept().unwrap();
        let conn = rustls::ServerConnection::new(Arc::new(config)).unwrap();
        let mut tls = rustls::StreamOwned::new(conn, tcp);
        let mut req = Vec::new();
        let mut buf = [0u8; 1024];
        while !req.windows(4).any(|w| w == b"\r\n\r\n") {
            match tls.read(&mut buf) {
                Ok(0) | Err(_) => return, // the refusing client hangs up mid-handshake
                Ok(n) => req.extend_from_slice(&buf[..n]),
            }
        }
        let body = r#"{"ok":true,"version":"9.9.9"}"#;
        let resp = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = tls.write_all(resp.as_bytes());
        let _ = tls.flush();
        tls.conn.send_close_notify();
        let _ = tls.flush();
    });
    (port, cert_der, handle)
}

// spec: S24-01
#[test]
fn the_client_completes_a_request_to_an_https_server() {
    let (port, cert, server) = https_once();
    let roots = ureq::tls::RootCerts::new_with_certs(&[ureq::tls::Certificate::from_der(&cert).to_owned()]);
    let tls = ureq::tls::TlsConfig::builder().root_certs(roots).build();
    let api = Api::with_tls(&format!("https://localhost:{port}"), "", tls);
    let health = api.health().unwrap_or_else(|e| panic!("https health failed: {}", e.message));
    assert_eq!(health["ok"], true);
    assert_eq!(health["version"], "9.9.9");
    server.join().unwrap();
}

// spec: S24-02
#[test]
fn the_cli_client_speaks_tls_and_refuses_an_untrusted_certificate() {
    let (port, _, server) = https_once();
    let err = Api::anonymous(&format!("https://localhost:{port}")).health().unwrap_err();
    assert!(!err.message.contains("unsecured"), "no TLS provider: {}", err.message);
    assert_eq!((err.exit, err.kind.as_str()), (Exit::Unreachable, "tls"), "{}", err.message);
    assert!(err.message.starts_with(&format!("TLS with https://localhost:{port} failed:")), "{}", err.message);
    assert!(err.message.to_lowercase().contains("certificate"), "{}", err.message);
    server.join().unwrap();
}
