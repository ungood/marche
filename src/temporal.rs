//! Connecting to Temporal the way its own CLI does, and the search attributes
//! marche sets on every run.

use std::collections::HashMap;

use temporalio_client::envconfig::LoadClientConfigProfileOptions;
use temporalio_client::tonic::Request;
use temporalio_client::{Client, ClientOptions, Connection, Url};
use temporalio_common::protos::temporal::api::enums::v1::IndexedValueType;
use temporalio_common::protos::temporal::api::operatorservice::v1::{
    AddSearchAttributesRequest, ListSearchAttributesRequest,
};
use temporalio_common::search_attributes::SearchAttributeKey;

use crate::error::Error;

pub const DEFINITION: SearchAttributeKey<String> = SearchAttributeKey::keyword("MarcheDefinition");
pub const DEFINITION_HASH: SearchAttributeKey<String> =
    SearchAttributeKey::keyword("MarcheDefinitionHash");
pub const DIRECTORY: SearchAttributeKey<String> = SearchAttributeKey::keyword("MarcheDirectory");

const SEARCH_ATTRIBUTES: [&str; 3] = [
    "MarcheDefinition",
    "MarcheDefinitionHash",
    "MarcheDirectory",
];

/// The global flags that pick a server, over Temporal's environment and profile.
#[derive(Debug, Clone, Default, clap::Args)]
pub struct Target {
    /// The Temporal server, as host:port [env: TEMPORAL_ADDRESS, default: localhost:7233]
    #[arg(long, global = true)]
    pub address: Option<String>,
    /// The Temporal namespace [env: TEMPORAL_NAMESPACE, default: default]
    #[arg(long, global = true)]
    pub namespace: Option<String>,
}

pub struct Temporal {
    pub client: Client,
    pub namespace: String,
}

impl Target {
    pub async fn connect(&self) -> Result<Temporal, Error> {
        let (mut connection, mut options) =
            ClientOptions::load_from_config(LoadClientConfigProfileOptions::default())
                .map_err(|error| Error::Config(error.to_string()))?;
        if let Some(address) = &self.address {
            let url = if address.contains("://") {
                address.clone()
            } else {
                format!("http://{address}")
            };
            connection.target = Url::parse(&url).map_err(|error| Error::Connect {
                address: address.clone(),
                message: error.to_string(),
            })?;
        }
        if let Some(namespace) = &self.namespace {
            options = ClientOptions::new(namespace.clone()).build();
        }
        let namespace = options.namespace.clone();
        let address = connection.target.to_string();
        let connection = Connection::connect(connection)
            .await
            .map_err(|error| Error::Connect {
                address: address.clone(),
                message: error.to_string(),
            })?;
        let client = Client::new(connection, options).map_err(|error| Error::Connect {
            address,
            message: error.to_string(),
        })?;
        Ok(Temporal { client, namespace })
    }
}

impl Temporal {
    /// Registers marche's search attributes on the namespace if they're missing.
    pub async fn register_search_attributes(&self) -> Result<(), Error> {
        let mut operator = self.client.connection().operator_service();
        let existing = operator
            .list_search_attributes(Request::new(ListSearchAttributesRequest {
                namespace: self.namespace.clone(),
            }))
            .await
            .map_err(|status| Error::Worker(status.message().to_string()))?
            .into_inner()
            .custom_attributes;
        let missing: HashMap<String, i32> = SEARCH_ATTRIBUTES
            .iter()
            .filter(|name| !existing.contains_key(**name))
            .map(|name| (name.to_string(), IndexedValueType::Keyword as i32))
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        operator
            .add_search_attributes(Request::new(AddSearchAttributesRequest {
                search_attributes: missing,
                namespace: self.namespace.clone(),
            }))
            .await
            .map_err(|status| Error::Worker(status.message().to_string()))?;
        Ok(())
    }
}
