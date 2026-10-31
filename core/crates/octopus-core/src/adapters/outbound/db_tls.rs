//! TLS settings in Postgres and MySQL DSNs, read the way each database's own
//! client reads them, so a DSN means the same here as in libpq (psycopg) or
//! the `mysql` command-line client.
//!
//! Postgres (`sslmode`, libpq):
//! - `disable`: no TLS.
//! - `allow`, `prefer` (the default): TLS when the server offers it, the
//!   certificate not checked.
//! - `require`: TLS, the certificate not checked; with `sslrootcert` given,
//!   checked as `verify-ca` (as libpq does).
//! - `verify-ca`: the certificate must chain to a trusted root.
//! - `verify-full`: that, and it must name the host.
//!
//! The trusted roots are `sslrootcert` (a PEM file, or `system`), else
//! `~/.postgresql/root.crt` when it exists, else the Mozilla roots
//! (`webpki-roots`); libpq has no such last fallback and refuses instead.
//! `sslcert` and `sslkey` give a client certificate.
//!
//! MySQL (`ssl-mode`, also spelled `ssl_mode` or `sslmode`):
//! - `DISABLED`, `PREFERRED` (the default): no TLS. The driver cannot fall
//!   back from TLS, so `PREFERRED` stays plain.
//! - `REQUIRED`: TLS, the certificate not checked; with `ssl-ca` given,
//!   checked as `VERIFY_CA` (as the mysql client does).
//! - `VERIFY_CA`: the certificate must chain to `ssl-ca` or a Mozilla root.
//! - `VERIFY_IDENTITY`: that, and it must name the host.
//!
//! `require_ssl=true` means `REQUIRED`; the Go driver's `tls=true`,
//! `tls=skip-verify` and `tls=false` mean `VERIFY_IDENTITY`, `REQUIRED` and
//! `DISABLED`. `ssl-cert` and `ssl-key` give a client certificate.

/// Splits `query` ("k=v&k2=v2") into the pairs whose key `take` accepts and
/// the rest, still encoded.
fn split_query(query: &str, take: impl Fn(&str) -> bool) -> (Vec<(String, String)>, Vec<String>) {
    let mut taken = Vec::new();
    let mut kept = Vec::new();
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let key = percent_decode(k);
        if take(&key) {
            taken.push((key, percent_decode(v)));
        } else {
            kept.push(pair.to_string());
        }
    }
    (taken, kept)
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => match (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                (Some(hi), Some(lo)) => {
                    out.push(hi << 4 | lo);
                    i += 3;
                    continue;
                }
                _ => out.push(b'%'),
            },
            b'+' => out.push(b' '),
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Rebuilds a URL from its part before '?' and the query pairs left.
fn with_query(base: &str, kept: &[String]) -> String {
    if kept.is_empty() {
        base.to_string()
    } else {
        format!("{}?{}", base, kept.join("&"))
    }
}

// ============================================================================
// Postgres
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PgSslMode {
    Disable,
    Prefer,
    Require,
    VerifyCa,
    VerifyFull,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PgTls {
    pub mode: PgSslMode,
    pub root_cert: Option<String>,
    pub cert: Option<String>,
    pub key: Option<String>,
}

const PG_TLS_KEYS: [&str; 4] = ["sslmode", "sslrootcert", "sslcert", "sslkey"];

/// The DSN without its TLS settings (which the `postgres` crate either
/// refuses or reads differently), and those settings.
pub fn split_pg_tls(dsn: &str) -> Result<(String, PgTls), String> {
    let dsn = dsn.trim();
    let mut found: Vec<(String, String)> = Vec::new();
    let rest = if dsn.starts_with("postgres://") || dsn.starts_with("postgresql://") {
        match dsn.split_once('?') {
            Some((base, query)) => {
                let (taken, kept) = split_query(query, |k| PG_TLS_KEYS.contains(&k));
                found = taken;
                with_query(base, &kept)
            }
            None => dsn.to_string(),
        }
    } else {
        let mut kept = Vec::new();
        for (key, value, text) in kv_pairs(dsn)? {
            if PG_TLS_KEYS.contains(&key.as_str()) {
                found.push((key, value));
            } else {
                kept.push(text);
            }
        }
        kept.join(" ")
    };
    let get = |k: &str| {
        found
            .iter()
            .rev()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.clone())
            .filter(|v| !v.is_empty())
    };
    let root_cert = get("sslrootcert");
    let mode = match get("sslmode")
        .as_deref()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        None | Some("prefer") | Some("allow") => PgSslMode::Prefer,
        Some("disable") => PgSslMode::Disable,
        Some("require") if root_cert.is_some() => PgSslMode::VerifyCa,
        Some("require") => PgSslMode::Require,
        Some("verify-ca") => PgSslMode::VerifyCa,
        Some("verify-full") => PgSslMode::VerifyFull,
        Some(other) => return Err(format!("invalid sslmode '{}'", other)),
    };
    Ok((
        rest,
        PgTls {
            mode,
            root_cert,
            cert: get("sslcert"),
            key: get("sslkey"),
        },
    ))
}

/// libpq's `key=value` form: (key, unquoted value, the pair as written).
fn kv_pairs(dsn: &str) -> Result<Vec<(String, String, String)>, String> {
    let chars: Vec<char> = dsn.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        if i >= chars.len() {
            break;
        }
        let start = i;
        while i < chars.len() && chars[i] != '=' && !chars[i].is_whitespace() {
            i += 1;
        }
        let key: String = chars[start..i].iter().collect();
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        if i >= chars.len() || chars[i] != '=' {
            return Err(format!("invalid DSN: '{}' has no value", key));
        }
        i += 1;
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if i < chars.len() && chars[i] == '\'' {
            i += 1;
            loop {
                match chars.get(i) {
                    None => return Err(format!("invalid DSN: unterminated quote in '{}'", key)),
                    Some('\'') => {
                        i += 1;
                        break;
                    }
                    Some('\\') if i + 1 < chars.len() => {
                        value.push(chars[i + 1]);
                        i += 2;
                    }
                    Some(c) => {
                        value.push(*c);
                        i += 1;
                    }
                }
            }
        } else {
            while i < chars.len() && !chars[i].is_whitespace() {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    i += 1;
                }
                value.push(chars[i]);
                i += 1;
            }
        }
        out.push((key, value, chars[start..i].iter().collect()));
    }
    Ok(out)
}

#[cfg(feature = "postgres")]
pub use pg_rustls::client_config as pg_client_config;

#[cfg(feature = "postgres")]
mod pg_rustls {
    use super::{PgSslMode, PgTls};
    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use rustls::client::WebPkiServerVerifier;
    use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider};
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
    use rustls::{
        CertificateError, ClientConfig, DigitallySignedStruct, Error, RootCertStore,
        SignatureScheme,
    };
    use std::sync::Arc;

    /// The rustls client settings for `tls` (not used for `disable`).
    pub fn client_config(tls: &PgTls) -> Result<ClientConfig, String> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let check_chain = matches!(tls.mode, PgSslMode::VerifyCa | PgSslMode::VerifyFull);
        let roots = if check_chain {
            roots(tls.root_cert.as_deref())?
        } else {
            RootCertStore::empty()
        };
        let inner = if check_chain {
            Some(
                WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider.clone())
                    .build()
                    .map_err(|e| format!("TLS roots: {}", e))?,
            )
        } else {
            None
        };
        let verifier = Verifier {
            inner,
            check_name: tls.mode == PgSslMode::VerifyFull,
            provider: provider.clone(),
        };
        let builder = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| e.to_string())?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(verifier));
        match (&tls.cert, &tls.key) {
            (Some(cert), Some(key)) => {
                let chain = CertificateDer::pem_file_iter(cert)
                    .and_then(|it| it.collect::<Result<Vec<_>, _>>())
                    .map_err(|e| format!("sslcert '{}': {}", cert, e))?;
                let key = PrivateKeyDer::from_pem_file(key)
                    .map_err(|e| format!("sslkey '{}': {}", key, e))?;
                builder
                    .with_client_auth_cert(chain, key)
                    .map_err(|e| e.to_string())
            }
            (None, None) => Ok(builder.with_no_client_auth()),
            _ => Err("sslcert and sslkey go together".to_string()),
        }
    }

    fn roots(root_cert: Option<&str>) -> Result<RootCertStore, String> {
        let mut store = RootCertStore::empty();
        let default_file = std::env::var_os("HOME")
            .map(|h| {
                std::path::Path::new(&h)
                    .join(".postgresql")
                    .join("root.crt")
            })
            .filter(|p| p.is_file());
        let file = match root_cert {
            Some("system") => None,
            Some(path) => Some(std::path::PathBuf::from(path)),
            None => default_file,
        };
        match file {
            Some(path) => {
                let certs = CertificateDer::pem_file_iter(&path)
                    .and_then(|it| it.collect::<Result<Vec<_>, _>>())
                    .map_err(|e| format!("sslrootcert '{}': {}", path.display(), e))?;
                let (added, _) = store.add_parsable_certificates(certs);
                if added == 0 {
                    return Err(format!(
                        "sslrootcert '{}' has no certificate",
                        path.display()
                    ));
                }
            }
            None => store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned()),
        }
        Ok(store)
    }

    /// Checks the server's certificate as far as the sslmode asks. The
    /// handshake's signatures are always checked.
    #[derive(Debug)]
    struct Verifier {
        inner: Option<Arc<WebPkiServerVerifier>>,
        check_name: bool,
        provider: Arc<CryptoProvider>,
    }

    impl ServerCertVerifier for Verifier {
        fn verify_server_cert(
            &self,
            end_entity: &CertificateDer<'_>,
            intermediates: &[CertificateDer<'_>],
            server_name: &ServerName<'_>,
            ocsp_response: &[u8],
            now: UnixTime,
        ) -> Result<ServerCertVerified, Error> {
            let Some(inner) = &self.inner else {
                return Ok(ServerCertVerified::assertion());
            };
            match inner.verify_server_cert(
                end_entity,
                intermediates,
                server_name,
                ocsp_response,
                now,
            ) {
                Err(Error::InvalidCertificate(
                    CertificateError::NotValidForName
                    | CertificateError::NotValidForNameContext { .. },
                )) if !self.check_name => Ok(ServerCertVerified::assertion()),
                other => other,
            }
        }

        fn verify_tls12_signature(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, Error> {
            verify_tls12_signature(
                message,
                cert,
                dss,
                &self.provider.signature_verification_algorithms,
            )
        }

        fn verify_tls13_signature(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, Error> {
            verify_tls13_signature(
                message,
                cert,
                dss,
                &self.provider.signature_verification_algorithms,
            )
        }

        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            self.provider
                .signature_verification_algorithms
                .supported_schemes()
        }
    }
}

// ============================================================================
// MySQL
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MySqlSslMode {
    Disabled,
    Required,
    VerifyCa,
    VerifyIdentity,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MySqlTls {
    pub mode: MySqlSslMode,
    pub ca: Option<String>,
    pub cert: Option<String>,
    pub key: Option<String>,
}

fn mysql_tls_key(key: &str) -> Option<&'static str> {
    Some(match key.to_ascii_lowercase().replace('-', "_").as_str() {
        "ssl_mode" | "sslmode" => "mode",
        "require_ssl" => "require",
        "tls" => "tls",
        "ssl_ca" | "sslca" | "sslrootcert" => "ca",
        "ssl_cert" | "sslcert" => "cert",
        "ssl_key" | "sslkey" => "key",
        _ => return None,
    })
}

/// The `mysql://` URL without its TLS settings, and those settings.
pub fn split_mysql_tls(url: &str) -> Result<(String, MySqlTls), String> {
    let (rest, found) = match url.split_once('?') {
        Some((base, query)) => {
            let (taken, kept) = split_query(query, |k| mysql_tls_key(k).is_some());
            (with_query(base, &kept), taken)
        }
        None => (url.to_string(), Vec::new()),
    };
    let get = |name: &str| {
        found
            .iter()
            .rev()
            .find(|(k, _)| mysql_tls_key(k) == Some(name))
            .map(|(_, v)| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let ca = get("ca");
    let mut mode = MySqlSslMode::Disabled;
    if let Some(tls) = get("tls") {
        mode = match tls.to_ascii_lowercase().as_str() {
            "true" | "1" => MySqlSslMode::VerifyIdentity,
            "skip-verify" => MySqlSslMode::Required,
            "false" | "0" | "preferred" => MySqlSslMode::Disabled,
            other => return Err(format!("invalid tls '{}'", other)),
        };
    }
    if let Some(req) = get("require") {
        if matches!(req.to_ascii_lowercase().as_str(), "true" | "1") {
            mode = MySqlSslMode::Required;
        }
    }
    if let Some(m) = get("mode") {
        mode = match m.to_ascii_uppercase().replace('-', "_").as_str() {
            "DISABLED" | "PREFERRED" => MySqlSslMode::Disabled,
            "REQUIRED" => MySqlSslMode::Required,
            "VERIFY_CA" => MySqlSslMode::VerifyCa,
            "VERIFY_IDENTITY" => MySqlSslMode::VerifyIdentity,
            other => return Err(format!("invalid ssl-mode '{}'", other)),
        };
    }
    if mode == MySqlSslMode::Required && ca.is_some() {
        mode = MySqlSslMode::VerifyCa;
    }
    Ok((
        rest,
        MySqlTls {
            mode,
            ca,
            cert: get("cert"),
            key: get("key"),
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postgres_tls_settings_come_out_of_a_url() {
        let (rest, tls) = split_pg_tls(
            "postgres://u:p@db.example.com:5432/app?application_name=x&sslmode=verify-full&sslrootcert=%2Fetc%2Fca.pem",
        )
        .unwrap();
        assert_eq!(
            rest,
            "postgres://u:p@db.example.com:5432/app?application_name=x"
        );
        assert_eq!(tls.mode, PgSslMode::VerifyFull);
        assert_eq!(tls.root_cert.as_deref(), Some("/etc/ca.pem"));
    }

    #[test]
    fn postgres_tls_settings_come_out_of_key_value_pairs() {
        let (rest, tls) =
            split_pg_tls("host=db user=u password='a b\\'c' sslmode=require dbname=app").unwrap();
        assert_eq!(rest, "host=db user=u password='a b\\'c' dbname=app");
        assert_eq!(tls.mode, PgSslMode::Require);
        // libpq: require with a root certificate checks the chain.
        let (_, tls) = split_pg_tls("host=db sslmode=require sslrootcert=/ca.pem").unwrap();
        assert_eq!(tls.mode, PgSslMode::VerifyCa);
    }

    #[test]
    fn postgres_without_sslmode_prefers_tls() {
        let (rest, tls) = split_pg_tls("postgres://u@localhost/app").unwrap();
        assert_eq!(rest, "postgres://u@localhost/app");
        assert_eq!(tls.mode, PgSslMode::Prefer);
        assert_eq!(
            split_pg_tls("host=x sslmode=disable").unwrap().1.mode,
            PgSslMode::Disable
        );
        assert!(split_pg_tls("host=x sslmode=sometimes").is_err());
    }

    #[test]
    fn mysql_tls_settings_in_their_spellings() {
        let (rest, tls) =
            split_mysql_tls("mysql://u:p@h:3306/db?ssl-mode=REQUIRED&pool_max=4").unwrap();
        assert_eq!(rest, "mysql://u:p@h:3306/db?pool_max=4");
        assert_eq!(tls.mode, MySqlSslMode::Required);
        let (rest, tls) = split_mysql_tls("mysql://h/db?ssl_mode=verify_identity").unwrap();
        assert_eq!(rest, "mysql://h/db");
        assert_eq!(tls.mode, MySqlSslMode::VerifyIdentity);
        assert_eq!(
            split_mysql_tls("mysql://h/db?require_ssl=true")
                .unwrap()
                .1
                .mode,
            MySqlSslMode::Required
        );
        assert_eq!(
            split_mysql_tls("mysql://h/db?tls=skip-verify")
                .unwrap()
                .1
                .mode,
            MySqlSslMode::Required
        );
        assert_eq!(
            split_mysql_tls("mysql://h/db?tls=true").unwrap().1.mode,
            MySqlSslMode::VerifyIdentity
        );
        assert_eq!(
            split_mysql_tls("mysql://h/db?ssl-mode=PREFERRED")
                .unwrap()
                .1
                .mode,
            MySqlSslMode::Disabled
        );
        // As the mysql client: REQUIRED with a CA checks the chain.
        let tls = split_mysql_tls("mysql://h/db?ssl-mode=REQUIRED&ssl-ca=/ca.pem")
            .unwrap()
            .1;
        assert_eq!(
            (tls.mode, tls.ca.as_deref()),
            (MySqlSslMode::VerifyCa, Some("/ca.pem"))
        );
        assert!(split_mysql_tls("mysql://h/db?ssl-mode=SOMETIMES").is_err());
    }
}

/// The Postgres driver against a server that speaks just enough of the
/// protocol: it answers the TLS request (or not), then refuses the login
/// with a message saying how the client came in.
#[cfg(all(test, feature = "postgres"))]
mod postgres_handshake_tests {
    use crate::adapters::outbound::sql_backend;
    use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair};
    use rustls::pki_types::PrivateKeyDer;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;

    struct Pki {
        server: Arc<rustls::ServerConfig>,
        ca_file: std::path::PathBuf,
    }

    /// A CA, and a server certificate it signed for "localhost" only.
    fn pki() -> Pki {
        let ca_key = KeyPair::generate().unwrap();
        let mut ca = CertificateParams::new(Vec::<String>::new()).unwrap();
        ca.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca.distinguished_name
            .push(DnType::CommonName, "octopus test CA");
        let ca_cert = ca.self_signed(&ca_key).unwrap();
        let key = KeyPair::generate().unwrap();
        let cert = CertificateParams::new(vec!["localhost".to_string()])
            .unwrap()
            .signed_by(&key, &ca_cert, &ca_key)
            .unwrap();
        let server = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.der().clone()],
            PrivateKeyDer::Pkcs8(key.serialize_der().into()),
        )
        .unwrap();
        let ca_file =
            std::env::temp_dir().join(format!("octopus-test-ca-{}.pem", std::process::id()));
        std::fs::write(&ca_file, ca_cert.pem()).unwrap();
        Pki {
            server: Arc::new(server),
            ca_file,
        }
    }

    fn error_response(message: &str) -> Vec<u8> {
        let mut body = Vec::new();
        for (field, value) in [
            (b'S', "FATAL"),
            (b'V', "FATAL"),
            (b'C', "28000"),
            (b'M', message),
        ] {
            body.push(field);
            body.extend_from_slice(value.as_bytes());
            body.push(0);
        }
        body.push(0);
        let mut out = vec![b'E'];
        out.extend_from_slice(&((body.len() + 4) as u32).to_be_bytes());
        out.extend(body);
        out
    }

    /// Serves one connection; the client's error then says "came in over TLS"
    /// or "came in plain". `offer_tls`: whether the server accepts TLS.
    fn fake_server(pki: &Pki, offer_tls: bool) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let config = pki.server.clone();
        std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            sock.set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut head = [0u8; 8];
            if sock.read_exact(&mut head).is_err() {
                return;
            }
            const SSL_REQUEST: [u8; 8] = [0, 0, 0, 8, 0x04, 0xd2, 0x16, 0x2f];
            if head == SSL_REQUEST && offer_tls {
                sock.write_all(b"S").unwrap();
                let conn = rustls::ServerConnection::new(config).unwrap();
                let mut tls = rustls::StreamOwned::new(conn, sock);
                let mut len = [0u8; 4];
                if tls.read_exact(&mut len).is_err() {
                    return; // the client gave up on the handshake
                }
                let mut rest = vec![0u8; u32::from_be_bytes(len) as usize - 4];
                let _ = tls.read_exact(&mut rest);
                let _ = tls.write_all(&error_response("came in over TLS"));
                let _ = tls.flush();
            } else {
                if head == SSL_REQUEST {
                    sock.write_all(b"N").unwrap();
                    if sock.read_exact(&mut head).is_err() {
                        return;
                    }
                }
                let len = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
                let mut rest = vec![0u8; len.saturating_sub(8)];
                let _ = sock.read_exact(&mut rest);
                let _ = sock.write_all(&error_response("came in plain"));
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        });
        port
    }

    fn connect(pki: &Pki, host: &str, extra: &str, offer_tls: bool) -> String {
        let port = fake_server(pki, offer_tls);
        let dsn = format!(
            "postgres://u:p@{}:{}/d?{}",
            host,
            port,
            extra.replace("{ca}", &pki.ca_file.display().to_string())
        );
        match sql_backend::connect("postgres", &dsn, 1) {
            Ok(_) => "connected".to_string(),
            Err(e) => e,
        }
    }

    #[test]
    fn each_sslmode_checks_what_libpq_checks() {
        let pki = pki();
        let ok = |out: String| assert!(out.contains("came in over TLS"), "{}", out);
        let refused = |out: String| assert!(out.to_lowercase().contains("certificate"), "{}", out);

        // No sslmode (prefer) and require: TLS, the certificate unchecked.
        ok(connect(&pki, "127.0.0.1", "", true));
        ok(connect(&pki, "127.0.0.1", "sslmode=require", true));
        // verify-ca: the chain, not the name.
        ok(connect(
            &pki,
            "127.0.0.1",
            "sslmode=verify-ca&sslrootcert={ca}",
            true,
        ));
        // require with a root certificate is verify-ca.
        ok(connect(
            &pki,
            "127.0.0.1",
            "sslmode=require&sslrootcert={ca}",
            true,
        ));
        // verify-full: the chain and the name.
        ok(connect(
            &pki,
            "localhost",
            "sslmode=verify-full&sslrootcert={ca}",
            true,
        ));
        refused(connect(
            &pki,
            "127.0.0.1",
            "sslmode=verify-full&sslrootcert={ca}",
            true,
        ));
        // Not signed by a Mozilla root.
        refused(connect(&pki, "localhost", "sslmode=verify-full", true));

        // A server without TLS: prefer and disable come in plain, require fails.
        let plain = |out: String| assert!(out.contains("came in plain"), "{}", out);
        plain(connect(&pki, "127.0.0.1", "", false));
        plain(connect(&pki, "127.0.0.1", "sslmode=disable", true));
        let out = connect(&pki, "127.0.0.1", "sslmode=require", false);
        assert!(!out.contains("came in"), "{}", out);
        let _ = std::fs::remove_file(&pki.ca_file);
    }
}

/// The MySQL driver against a server that sends MySQL's greeting (offering
/// TLS or not), then refuses the login saying how the client came in.
#[cfg(all(test, feature = "mysql"))]
mod mysql_handshake_tests {
    use crate::adapters::outbound::sql_backend;
    use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair};
    use rustls::pki_types::PrivateKeyDer;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::Arc;

    fn pki() -> (Arc<rustls::ServerConfig>, std::path::PathBuf) {
        let ca_key = KeyPair::generate().unwrap();
        let mut ca = CertificateParams::new(Vec::<String>::new()).unwrap();
        ca.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca.distinguished_name
            .push(DnType::CommonName, "octopus test CA");
        let ca_cert = ca.self_signed(&ca_key).unwrap();
        let key = KeyPair::generate().unwrap();
        let cert = CertificateParams::new(vec!["localhost".to_string()])
            .unwrap()
            .signed_by(&key, &ca_cert, &ca_key)
            .unwrap();
        let server = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.der().clone()],
            PrivateKeyDer::Pkcs8(key.serialize_der().into()),
        )
        .unwrap();
        let ca_file =
            std::env::temp_dir().join(format!("octopus-test-mysql-ca-{}.pem", std::process::id()));
        std::fs::write(&ca_file, ca_cert.pem()).unwrap();
        (Arc::new(server), ca_file)
    }

    fn packet(seq: u8, payload: &[u8]) -> Vec<u8> {
        let len = payload.len() as u32;
        let mut out = vec![len as u8, (len >> 8) as u8, (len >> 16) as u8, seq];
        out.extend_from_slice(payload);
        out
    }

    fn read_packet(r: &mut impl Read) -> Option<(u8, Vec<u8>)> {
        let mut head = [0u8; 4];
        r.read_exact(&mut head).ok()?;
        let len = head[0] as usize | (head[1] as usize) << 8 | (head[2] as usize) << 16;
        let mut payload = vec![0u8; len];
        r.read_exact(&mut payload).ok()?;
        Some((head[3], payload))
    }

    /// Protocol 10 greeting with mysql_native_password.
    fn greeting(offer_tls: bool) -> Vec<u8> {
        const BASE: u32 = 0x1 | 0x4 | 0x8 | 0x200 | 0x2000 | 0x8000 | 0x80000;
        let caps = if offer_tls { BASE | 0x800 } else { BASE };
        let mut p = vec![10u8];
        p.extend_from_slice(b"8.0.36\0");
        p.extend_from_slice(&1u32.to_le_bytes());
        p.extend_from_slice(b"abcdefgh\0");
        p.extend_from_slice(&(caps as u16).to_le_bytes());
        p.push(0x21);
        p.extend_from_slice(&2u16.to_le_bytes());
        p.extend_from_slice(&((caps >> 16) as u16).to_le_bytes());
        p.push(21);
        p.extend_from_slice(&[0u8; 10]);
        p.extend_from_slice(b"ijklmnopqrst\0");
        p.extend_from_slice(b"mysql_native_password\0");
        packet(0, &p)
    }

    fn refusal(seq: u8, message: &str) -> Vec<u8> {
        let mut p = vec![0xff];
        p.extend_from_slice(&1045u16.to_le_bytes());
        p.extend_from_slice(b"#28000");
        p.extend_from_slice(message.as_bytes());
        packet(seq, &p)
    }

    fn serve(mut sock: TcpStream, config: Arc<rustls::ServerConfig>, offer_tls: bool) {
        sock.set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        sock.write_all(&greeting(offer_tls)).unwrap();
        let Some((seq, payload)) = read_packet(&mut sock) else {
            return;
        };
        if payload.len() == 32 && offer_tls {
            // An SSL request: the rest comes over TLS.
            let conn = rustls::ServerConnection::new(config).unwrap();
            let mut tls = rustls::StreamOwned::new(conn, sock);
            let Some((seq, _)) = read_packet(&mut tls) else {
                return;
            };
            let _ = tls.write_all(&refusal(seq + 1, "came in over TLS"));
            let _ = tls.flush();
        } else {
            let _ = sock.write_all(&refusal(seq + 1, "came in plain"));
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }

    fn connect(
        server: &(Arc<rustls::ServerConfig>, std::path::PathBuf),
        host: &str,
        query: &str,
        offer_tls: bool,
    ) -> String {
        // The driver tries the first address only ("localhost" may be ::1).
        let addr = std::net::ToSocketAddrs::to_socket_addrs(&(host, 0))
            .unwrap()
            .next()
            .unwrap();
        let listener = TcpListener::bind(addr).unwrap();
        let port = listener.local_addr().unwrap().port();
        let config = server.0.clone();
        std::thread::spawn(move || {
            for sock in listener.incoming().flatten() {
                let config = config.clone();
                std::thread::spawn(move || serve(sock, config, offer_tls));
            }
        });
        let dsn = format!(
            "mysql://u:p@{}:{}/d?{}",
            host,
            port,
            query.replace("{ca}", &server.1.display().to_string())
        );
        match sql_backend::connect("mysql", &dsn, 1) {
            Ok(_) => "connected".to_string(),
            Err(e) => e,
        }
    }

    #[test]
    fn each_ssl_mode_checks_what_the_mysql_client_checks() {
        let server = pki();
        let ok = |out: String| assert!(out.contains("came in over TLS"), "{}", out);
        let refused = |out: String| assert!(out.to_lowercase().contains("certificate"), "{}", out);
        let plain = |out: String| assert!(out.contains("came in plain"), "{}", out);

        ok(connect(&server, "127.0.0.1", "ssl-mode=REQUIRED", true));
        ok(connect(&server, "127.0.0.1", "require_ssl=true", true));
        ok(connect(
            &server,
            "127.0.0.1",
            "ssl-mode=VERIFY_CA&ssl-ca={ca}",
            true,
        ));
        ok(connect(
            &server,
            "localhost",
            "ssl-mode=VERIFY_IDENTITY&ssl-ca={ca}",
            true,
        ));
        refused(connect(
            &server,
            "127.0.0.1",
            "ssl-mode=VERIFY_IDENTITY&ssl-ca={ca}",
            true,
        ));
        refused(connect(&server, "127.0.0.1", "ssl-mode=VERIFY_CA", true));

        // No ssl-mode (PREFERRED) stays plain; REQUIRED needs a server with TLS.
        plain(connect(&server, "127.0.0.1", "", true));
        let out = connect(&server, "127.0.0.1", "ssl-mode=REQUIRED", false);
        assert!(!out.contains("came in"), "{}", out);
        let _ = std::fs::remove_file(&server.1);
    }
}
