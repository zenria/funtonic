use crate::executor_meta::{ExecutorMeta, Tag};
use crate::file_utils::{parse_yaml_from_file, path_concat2, read};
use crate::tonic;
use anyhow::Error;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};
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
        let mut client_tls_config = ClientTlsConfig::new();
        client_tls_config = client_tls_config
            .identity(self.get_identity()?)
            .ca_certificate(self.get_ca_certificate()?);
        if let Some(domain) = &self.server_domain {
            client_tls_config = client_tls_config.domain_name(domain);
        }
        Ok(client_tls_config)
    }

    pub fn get_server_config(&self) -> Result<ServerTlsConfig, anyhow::Error> {
        Ok(ServerTlsConfig::new()
            .identity(self.get_identity()?)
            .client_ca_root(self.get_ca_certificate()?))
    }

    fn get_identity(&self) -> Result<Identity, anyhow::Error> {
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
    /// TLS configuration. If not present, plain unencrypted socket communication will be used
    pub tls: Option<TlsConfig>,
    /// bind address
    pub bind_address: String,
    /// Where the server stores its data
    pub data_directory: String,
    /// List of "authorized" public keys
    pub authorized_keys: BTreeMap<String, String>,
    /// List of admin related keys
    pub admin_authorized_keys: BTreeMap<String, String>,
}
#[derive(Serialize, Deserialize, Debug)]
pub struct CommanderConfig {
    /// TLS configuration. If not present, plain unencrypted socket communication will be used
    pub tls: Option<TlsConfig>,
    pub server_url: String,
    pub ed25519_key: ED25519Key,
}
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ED25519Key {
    pub id: String,
    pub pkcs8: String,
    // useful for retrieving the public key from the config ;)
    pub public_key: Option<String>,
}
#[derive(Serialize, Deserialize, Debug)]
pub struct ExecutorConfig {
    /// TLS configuration. If not present, plain unencrypted socket communication will be used
    pub tls: Option<TlsConfig>,
    pub client_id: String,
    pub tags: HashMap<String, Tag>,
    pub server_url: String,
    pub authorized_keys: BTreeMap<String, String>,
}

const DEFAULT_CONFIG_LOCATION: &[&str] = &["~/.funtonic/", "/etc/funtonic/"];

#[derive(Error, Debug)]
#[error("Config file not found: {0}")]
struct NoConfigFileError(String);

fn resolve_config<P: AsRef<Path>, T: AsRef<Path>, D: DeserializeOwned>(
    provided_config: &Option<P>,
    name: T,
) -> Result<(D, PathBuf), Error> {
    let config_path = resolve_config_path(provided_config, name)?;
    Ok((parse_yaml_from_file(&config_path)?, config_path))
}

fn resolve_config_path<P: AsRef<Path>, T: AsRef<Path>>(
    provided_config: &Option<P>,
    name: T,
) -> Result<PathBuf, NoConfigFileError> {
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
        .ok_or(NoConfigFileError(name.as_ref().to_string_lossy().into()))
}

pub fn parse<P: AsRef<Path>, T: AsRef<Path>, C: DeserializeOwned>(
    provided_config: &Option<P>,
    name: T,
) -> Result<(C, PathBuf), Error> {
    resolve_config(provided_config, name)
}

pub fn get_config_directory<P: AsRef<Path>, T: AsRef<Path>>(
    provided_config: &Option<P>,
    name: T,
) -> Result<PathBuf, Error> {
    let mut config_file = resolve_config_path(provided_config, name)?;
    config_file.pop();
    Ok(config_file)
}

impl From<(&str, &[u8])> for ED25519Key {
    fn from((id, bytes): (&str, &[u8])) -> Self {
        Self {
            id: id.to_string(),
            pkcs8: data_encoding::BASE64.encode(bytes),
            public_key: None,
        }
    }
}

impl ED25519Key {
    pub fn to_bytes(&self) -> Result<Vec<u8>, data_encoding::DecodeError> {
        data_encoding::BASE64.decode(self.pkcs8.as_bytes())
    }

    pub fn id(&self) -> &str {
        &self.id
    }
}
