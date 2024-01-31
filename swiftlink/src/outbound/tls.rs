use std::{
    fs::File,
    io::{self, BufReader},
    path::{Path, PathBuf},
    sync::Arc,
};

use tokio_rustls::{
    rustls::{
        pki_types::{CertificateDer, ServerName},
        ClientConfig, RootCertStore,
    },
    TlsConnector,
};

use swiftlink_infra::log::*;
use swiftlink_transport::socks5::Address;

use crate::context::{Metadata, Network};

use super::{AnyOutboundStream, OutboundStreamHandle};

pub struct Handle {
    sni: String,
    client_config: Arc<ClientConfig>,
}

impl Handle {
    pub fn new(
        sni: String,
        alpns: Vec<String>,
        skip_cert_verify: bool,
        ca_path: Option<PathBuf>,
        ca_file: Option<PathBuf>,
    ) -> anyhow::Result<Self> {
        let mut root_store = RootCertStore::empty();
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

        let certs = {
            let certs1 = rustls_native_certs::load_native_certs().unwrap_or_else(|err| {
                warn!("load native certs failed.{}", err);
                Default::default()
            });

            let certs2 = [ca_path, ca_file]
                .into_iter()
                .flatten()
                .filter_map(|path| match load_certs_from_path(path.as_path()) {
                    Ok(certs) => Some(certs),
                    Err(err) => {
                        warn!("load certs from path failed.{}", err);
                        None
                    }
                })
                .flatten();

            certs1.into_iter().chain(certs2)
        };

        for cert in certs {
            root_store.add(cert).unwrap_or_else(|err| {
                warn!("load certs from path failed.{}", err);
            })
        }

        let mut client_config = ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();

        // skip certificate verify
        client_config = if skip_cert_verify {
            let mut verify_off = client_config.clone();
            verify_off
                .dangerous()
                .set_certificate_verifier(Arc::new(NoCertificateVerification));

            #[derive(Debug)]
            struct NoCertificateVerification;

            #[allow(unused_variables)]
            impl tokio_rustls::rustls::client::danger::ServerCertVerifier for NoCertificateVerification {
                fn verify_server_cert(
                    &self,
                    end_entity: &CertificateDer<'_>,
                    intermediates: &[CertificateDer<'_>],
                    server_name: &tokio_rustls::rustls::pki_types::ServerName<'_>,
                    ocsp_response: &[u8],
                    now: tokio_rustls::rustls::pki_types::UnixTime,
                ) -> Result<tokio_rustls::rustls::client::danger::ServerCertVerified, tokio_rustls::rustls::Error>
                {
                    Ok(tokio_rustls::rustls::client::danger::ServerCertVerified::assertion())
                }

                fn verify_tls12_signature(
                    &self,
                    message: &[u8],
                    cert: &CertificateDer<'_>,
                    dss: &tokio_rustls::rustls::DigitallySignedStruct,
                ) -> Result<tokio_rustls::rustls::client::danger::HandshakeSignatureValid, tokio_rustls::rustls::Error>
                {
                    Ok(tokio_rustls::rustls::client::danger::HandshakeSignatureValid::assertion())
                }

                fn verify_tls13_signature(
                    &self,
                    message: &[u8],
                    cert: &CertificateDer<'_>,
                    dss: &tokio_rustls::rustls::DigitallySignedStruct,
                ) -> Result<tokio_rustls::rustls::client::danger::HandshakeSignatureValid, tokio_rustls::rustls::Error>
                {
                    Ok(tokio_rustls::rustls::client::danger::HandshakeSignatureValid::assertion())
                }

                fn supported_verify_schemes(&self) -> Vec<tokio_rustls::rustls::SignatureScheme> {
                    tokio_rustls::rustls::crypto::ring::default_provider()
                        .signature_verification_algorithms
                        .supported_schemes()
                }
            }

            verify_off
        } else {
            client_config
        };

        for alpn in alpns {
            client_config.alpn_protocols.push(alpn.into_bytes());
        }

        Ok(Self {
            sni,
            client_config: Arc::new(client_config),
        })
    }
}

#[async_trait::async_trait]
impl OutboundStreamHandle for Handle {
    fn remote_server_addr(&self, _metadata: &Metadata) -> Option<(Network, Address)> {
        None
    }

    async fn handle(&self, metadata: &Metadata, stream: Option<AnyOutboundStream>) -> io::Result<AnyOutboundStream> {
        let server_name = if !&self.sni.is_empty() {
            self.sni.to_owned()
        } else {
            metadata.target.host()
        };

        if let Some(stream) = stream {
            trace!("handle TLS {} with rustls", &server_name);
            let connector = TlsConnector::from(self.client_config.clone());
            let domain = ServerName::try_from(server_name.as_str().to_owned()).map_err(|e| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("invalid tls server name {}: {}", server_name, e),
                )
            })?;

            let tls_stream = connector
                .connect(domain, stream)
                .await
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, format!("connect tls error: {}", e)))?;

            return Ok(Box::new(tls_stream));
        }

        Err(io::Error::new(io::ErrorKind::Other, "no stream"))
    }
}

// TODO: move to swiftlink_infra crates

/// Load certificates from specific directory or file.
pub fn load_certs_from_path(path: &Path) -> Result<Vec<CertificateDer<'static>>, io::Error> {
    if path.is_dir() {
        let mut certs = vec![];
        for entry in path.read_dir()? {
            let path = entry?.path();
            if path.is_file() {
                certs.extend(load_pem_certs(path.as_path())?);
            }
        }
        Ok(certs)
    } else {
        load_pem_certs(path)
    }
}

fn load_pem_certs(path: &Path) -> Result<Vec<CertificateDer<'static>>, io::Error> {
    let f = File::open(path)?;
    let mut f = BufReader::new(f);

    match rustls_pemfile::certs(&mut f).into_iter().collect::<Result<Vec<_>, _>>() {
        Ok(contents) => Ok(contents),
        Err(_) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Couldn't load PEM file {:?}", path),
        )),
    }
}
