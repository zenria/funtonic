use crate::file_utils::{parse_yaml_from_file, path_concat2, read};
use anyhow::Error;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashMap;
use std::fs::File;
use std::path::Path;
use std::path::PathBuf;
use thiserror::Error;
use tonic::transport::{Certificate, ClientTlsConfig, Identity, ServerTlsConfig};

#[derive(Serialize, Deserialize, Debug)]
pub struct TlsConfig {
    /// CA PEM encoded certificate file path
    pub ca_cert: String,
    /// PEM encoded private key file path
    pub key: String,
    /// PEM encoded certificate file path
    pub cert: String,
    /// Expected domain name of the server certificate.
    ///
    /// When configuring a client, the domain name of the server certificate is validated against
    /// the server url.
    ///
    /// Specifying the server_domain overrides the server url domain.
    pub server_domain: Option<String>,
}

impl TlsConfig {
    pub fn get_client_config(&self) -> Result<ClientTlsConfig, anyhow::Error> {
        let mut client_tls_config = ClientTlsConfig::with_rustls();
        client_tls_config
            .identity(self.get_identify()?)
            .ca_certificate(self.get_ca_certificate()?);
        if let Some(domain) = &self.server_domain {
            client_tls_config.domain_name(domain);
        }
        Ok(client_tls_config)
    }

    pub fn get_server_config(&self) -> Result<ServerTlsConfig, anyhow::Error> {
        let mut server_tls_config = ServerTlsConfig::with_rustls();
        server_tls_config
            .identity(self.get_identify()?)
            .client_ca_root(self.get_ca_certificate()?);
        Ok(server_tls_config)
    }

    fn get_identify(&self) -> Result<Identity, anyhow::Error> {
        let cert = read(&self.cert)?;
        let key = read(&self.key)?;
        Ok(Identity::from_pem(cert, key))
    }

    fn get_ca_certificate(&self) -> Result<Certificate, anyhow::Error> {
        Ok(Certificate::from_pem(read(&self.ca_cert)?))
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ServerConfig {
    /// bind address
    pub bind_address: String,
}
#[derive(Serialize, Deserialize, Debug)]
pub struct CommanderConfig {
    pub server_url: String,
}
#[derive(Serialize, Deserialize, Debug)]
pub struct ExecutorConfig {
    pub tags: HashMap<String, String>,
    pub client_id: String,
    pub server_url: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
pub enum Role {
    #[serde(rename = "server")]
    Server(ServerConfig),
    #[serde(rename = "executor")]
    Executor(ExecutorConfig),
    #[serde(rename = "commander")]
    Commander(CommanderConfig),
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Config {
    /// TLS configuration. If not present, plain unencrypted socket communication will be used
    pub tls: Option<TlsConfig>,
    #[serde(flatten)]
    pub role: Role,
}

const DEFAULT_CONFIG_LOCATION: &[&str] = &["~/.funtonic/", "/etc/funtonic/"];

#[derive(Error, Debug)]
#[error("Config file not found: {0}")]
struct NoConfigFileError(String);

fn resolve_config<P: AsRef<Path>, T: AsRef<Path>, D: DeserializeOwned>(
    provided_config: &Option<P>,
    name: T,
) -> Result<D, Error> {
    provided_config
        .as_ref()
        .map(|path| PathBuf::from(path.as_ref()))
        .into_iter()
        .chain(
            DEFAULT_CONFIG_LOCATION
                .iter()
                .map(|loc| shellexpand::tilde(*loc))
                .map(|loc| path_concat2(loc.into_owned(), &name)),
        )
        .filter(|loc| loc.exists())
        .nth(0)
        .map(|loc| parse_yaml_from_file(loc))
        .unwrap_or(Err(NoConfigFileError(
            name.as_ref().to_string_lossy().into(),
        )
        .into()))
}

impl Config {
    pub fn parse<P: AsRef<Path>, T: AsRef<Path>>(
        provided_config: &Option<P>,
        name: T,
    ) -> Result<Config, Error> {
        resolve_config(provided_config, name)
    }
}
