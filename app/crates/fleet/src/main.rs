//! `homebody-fleet`: an organisation's own server for Homebody.
//!
//! It holds the organisation's rules, which joined computers take and put before their users'
//! own, and it receives those computers' logs, which arrive already signed and chained so that
//! this server can only store them, never change them unnoticed. People look at both through a
//! small web console, with roles deciding who may do what.
//!
//! It is one program with a folder of plain files next to it. Nothing here calls home.
//!
//!   homebody-fleet --data /var/lib/homebody-fleet --name "Example Org" \
//!       --listen 0.0.0.0:8470 --tls-cert fleet.pem --tls-key fleet.key --public-url https://fleet.example.org:8470

mod api;
mod auth;
mod http;
mod scim;
mod sso;
mod store;
mod tar;
#[cfg(test)]
mod tests;

use api::Fleet;
use std::path::PathBuf;
use std::sync::Arc;

const HELP: &str = "homebody-fleet: an organisation's own server for Homebody

  --data <folder>            where everything is kept (required)
  --name <text>              the organisation's name, shown on joined computers (first start)
  --listen <address:port>    default 127.0.0.1:8470
  --public-url <url>         the address computers and browsers use, default http://<listen>
  --tls-cert <file> --tls-key <file>   serve HTTPS with this certificate (PEM)

Sign-in through an identity provider (OpenID Connect), all optional:
  --oidc-issuer <url> --oidc-client-id <id> --oidc-client-secret-file <file>
  --oidc-admin-group <name> --oidc-auditor-group <name>
  --oidc-default-role viewer|none   for people in neither group (default none)
  --oidc-label <text>               the button's words (default \"your organisation\")

Provisioning people from the identity provider (SCIM 2.0), optional:
  --scim-token-file <file>          the secret the provider sends

Computers must reach this server over HTTPS unless it is on the same machine. Give it a
certificate here, or put it behind a web server that has one.";

pub struct Args {
    pub data: PathBuf,
    pub name: String,
    pub listen: String,
    pub public_url: Option<String>,
    pub tls: Option<(PathBuf, PathBuf)>,
    pub oidc: Option<sso::SsoConfig>,
    pub scim_token: Option<String>,
}

fn parse(args: &[String]) -> Result<Args, String> {
    let get = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let known = ["--data", "--name", "--listen", "--public-url", "--tls-cert", "--tls-key", "--oidc-issuer", "--oidc-client-id", "--oidc-client-secret-file", "--oidc-admin-group", "--oidc-auditor-group", "--oidc-default-role", "--oidc-label", "--scim-token-file"];
    let mut i = 0;
    while i < args.len() {
        if !known.contains(&args[i].as_str()) {
            return Err(format!("Unknown option: {}", args[i]));
        }
        i += 2;
    }
    let secret_from = |file: &str| std::fs::read_to_string(file).map(|s| s.trim().to_string()).map_err(|e| format!("{file} could not be read: {e}"));
    let tls = match (get("--tls-cert"), get("--tls-key")) {
        (Some(c), Some(k)) => Some((PathBuf::from(c), PathBuf::from(k))),
        (None, None) => None,
        _ => return Err("--tls-cert and --tls-key go together.".into()),
    };
    let oidc = match get("--oidc-issuer") {
        Some(issuer) => {
            let default_role = get("--oidc-default-role").unwrap_or_else(|| "none".into());
            if default_role != "viewer" && default_role != "none" {
                return Err("--oidc-default-role is viewer or none.".into());
            }
            Some(sso::SsoConfig {
                label: get("--oidc-label").unwrap_or_else(|| "your organisation".into()),
                issuer,
                client_id: get("--oidc-client-id").ok_or("--oidc-client-id is needed with --oidc-issuer.")?,
                client_secret: secret_from(&get("--oidc-client-secret-file").ok_or("--oidc-client-secret-file is needed with --oidc-issuer.")?)?,
                admin_group: get("--oidc-admin-group").unwrap_or_default(),
                auditor_group: get("--oidc-auditor-group").unwrap_or_default(),
                default_role,
            })
        }
        None => None,
    };
    let scim_token = match get("--scim-token-file") {
        Some(f) => {
            let t = secret_from(&f)?;
            if t.len() < 24 {
                return Err("The provisioning secret should be at least 24 characters.".into());
            }
            Some(t)
        }
        None => None,
    };
    Ok(Args { data: PathBuf::from(get("--data").ok_or("Say where to keep things: --data <folder>")?), name: get("--name").unwrap_or_else(|| "My organisation".into()), listen: get("--listen").unwrap_or_else(|| "127.0.0.1:8470".into()), public_url: get("--public-url"), tls, oidc, scim_token })
}

fn tls_acceptor(cert: &PathBuf, key: &PathBuf) -> Result<tokio_rustls::TlsAcceptor, String> {
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(cert).map_err(|e| format!("The certificate could not be read: {e}"))?.filter_map(Result::ok).collect();
    if certs.is_empty() {
        return Err("The certificate file holds no certificate.".into());
    }
    let key = PrivateKeyDer::from_pem_file(key).map_err(|e| format!("The key could not be read: {e}"))?;
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider())).with_safe_default_protocol_versions().map_err(|e| e.to_string())?.with_no_client_auth().with_single_cert(certs, key).map_err(|e| format!("The certificate and key do not go together: {e}"))?;
    Ok(tokio_rustls::TlsAcceptor::from(Arc::new(config)))
}

/// Open the folder and make the server's state. On the very first start, an administrator is
/// made and their password returned, to be shown once.
pub async fn build(args: Args) -> Result<(Arc<Fleet>, Option<String>), String> {
    let (store, first) = store::Store::open(&args.data, &args.name).map_err(|e| format!("{} could not be used: {e}", args.data.display()))?;
    let mut first_password = None;
    if first || store.people.lock().unwrap().is_empty() {
        let password = store::random_token(15);
        store.people.lock().unwrap().push(auth::new_person("admin", "admin", &password)?);
        store.save_people().map_err(|e| e.to_string())?;
        first_password = Some(password);
    }
    let sso = match args.oidc {
        Some(c) => Some(sso::Sso::discover(c).await?),
        None => None,
    };
    let public_url = args.public_url.unwrap_or_else(|| format!("{}://{}", if args.tls.is_some() { "https" } else { "http" }, args.listen)).trim_end_matches('/').to_string();
    let secure = public_url.starts_with("https://");
    Ok((Arc::new(Fleet { store, sessions: auth::Sessions::default(), public_url, secure, sso, scim_token_hash: args.scim_token.map(|t| store::sha256_hex(&t)) }), first_password))
}

pub async fn serve(fleet: Arc<Fleet>, listener: tokio::net::TcpListener, tls: Option<tokio_rustls::TlsAcceptor>) {
    loop {
        let Ok((stream, _)) = listener.accept().await else { continue };
        let (fleet, tls) = (fleet.clone(), tls.clone());
        tokio::spawn(async move {
            let work = async {
                match tls {
                    Some(acceptor) => {
                        if let Ok(mut s) = acceptor.accept(stream).await {
                            if let Some(req) = http::read_request(&mut s).await {
                                let _ = http::write_response(&mut s, api::handle(fleet, req).await).await;
                            }
                        }
                    }
                    None => {
                        let mut s = stream;
                        if let Some(req) = http::read_request(&mut s).await {
                            let _ = http::write_response(&mut s, api::handle(fleet, req).await).await;
                        }
                    }
                }
            };
            // Nobody gets to hold a connection open for ever.
            let _ = tokio::time::timeout(std::time::Duration::from_secs(120), work).await;
        });
    }
}

#[tokio::main]
async fn main() {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.is_empty() || raw.iter().any(|a| a == "--help" || a == "-h") {
        println!("{HELP}");
        return;
    }
    let args = match parse(&raw) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}\n\n{HELP}");
            std::process::exit(2);
        }
    };
    let listen = args.listen.clone();
    let tls = match &args.tls {
        Some((c, k)) => match tls_acceptor(c, k) {
            Ok(a) => Some(a),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(2);
            }
        },
        None => None,
    };
    let (fleet, first_password) = match build(args).await {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let listener = match tokio::net::TcpListener::bind(&listen).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Could not listen on {listen}: {e}");
            std::process::exit(1);
        }
    };
    println!("Homebody fleet for \"{}\"", fleet.store.settings.lock().unwrap().name);
    println!("Console: {}", fleet.public_url);
    println!("Fleet key: {}", fleet.store.public_key());
    if let Some(p) = first_password {
        println!("\nFirst start. Sign in as  admin  with the password  {p}\nIt is shown this once. Change it under People after signing in.\n");
    }
    if !fleet.secure && !listen.starts_with("127.0.0.1") && !listen.starts_with("localhost") {
        println!("NOTE: this address is not encrypted. Computers will only join over HTTPS: give --tls-cert and --tls-key, or put a web server with a certificate in front.");
    }
    // Old records go once a day, when a retention period is set.
    let keeper = fleet.clone();
    tokio::spawn(async move {
        loop {
            let f = keeper.clone();
            let _ = tokio::task::spawn_blocking(move || f.store.apply_retention()).await;
            tokio::time::sleep(std::time::Duration::from_secs(24 * 3600)).await;
        }
    });
    tokio::select! {
        _ = serve(fleet, listener, tls) => {}
        _ = tokio::signal::ctrl_c() => {}
    }
}
