use crate::config::SyncSettings;
use crate::encryption;
use crate::error::{SnipError, SnipResult};
use snip_proto::snippet_sync_client::SnippetSyncClient;
use snip_proto::PremadeLibrary;
use snip_proto::{
    CreateLibraryRequest, GetPremadeLibraryRequest, HealthRequest, Library, ListLibrariesRequest,
    ListPremadeLibrariesRequest, RegisterRequest, SyncRequest,
};
use std::time::Duration;
use tonic::transport::{Channel, ClientTlsConfig, Endpoint, Uri};

#[derive(serde::Serialize, serde::Deserialize)]
struct EncryptedSnippetData {
    description: String,
    command: String,
    tags: Vec<String>,
}

pub struct SyncClient {
    client: SnippetSyncClient<Channel>,
    settings: SyncSettings,
}

impl SyncClient {
    pub async fn create(settings: SyncSettings) -> SnipResult<Self> {
        let server_url = settings.server_url.clone();

        let channel = create_tls_channel(&server_url)
            .await
            .map_err(|e| SnipError::runtime_error("Failed to connect", Some(&e.to_string())))?;

        Ok(Self {
            client: SnippetSyncClient::new(channel),
            settings,
        })
    }

    pub async fn sync_encrypted(
        &mut self,
        local_snippets: Vec<snip_proto::Snippet>,
        last_sync: i64,
        library_id: &str,
    ) -> SnipResult<snip_proto::SyncResponse> {
        let api_key = &self.settings.api_key;

        let mut encrypted_snippets = Vec::new();
        let mut encrypt_failed_ids = Vec::new();

        for s in local_snippets {
            match encrypt_snippet(api_key, &s) {
                Ok(es) => encrypted_snippets.push(es),
                Err(e) => {
                    encrypt_failed_ids.push(s.id.clone());
                    tracing::warn!("Failed to encrypt snippet {}: {}", s.id, e);
                }
            }
        }

        let request = tonic::Request::new(SyncRequest {
            api_key: api_key.clone(),
            local_snippets: encrypted_snippets,
            last_sync_timestamp: last_sync,
            library_id: library_id.to_string(),
            limit: 1000,
        });

        let mut response = self
            .client
            .sync(request)
            .await
            .map_err(|e| SnipError::runtime_error("Sync request failed", Some(&e.to_string())))?
            .into_inner();

        let mut decrypted_snippets = Vec::new();
        let mut decrypt_failed_ids = Vec::new();

        for s in response.snippets {
            match decrypt_snippet(api_key, &s) {
                Ok(ds) => decrypted_snippets.push(ds),
                Err(e) => {
                    decrypt_failed_ids.push(s.id.clone());
                    tracing::warn!("Failed to decrypt snippet {}: {}", s.id, e);
                }
            }
        }

        let total_skipped = encrypt_failed_ids.len() + decrypt_failed_ids.len();
        let mut all_skipped_ids = encrypt_failed_ids;
        all_skipped_ids.extend(decrypt_failed_ids);

        response.snippets = decrypted_snippets;
        response.skipped_count = total_skipped as i32;
        response.skipped_ids = all_skipped_ids;

        Ok(response)
    }

    pub async fn health_check(&mut self) -> SnipResult<bool> {
        let request = tonic::Request::new(HealthRequest {});

        match self.client.health(request).await {
            Ok(response) => Ok(response.into_inner().healthy),
            Err(_) => Ok(false),
        }
    }

    pub async fn register(server_url: String) -> SnipResult<(String, String)> {
        let channel = create_tls_channel(&server_url)
            .await
            .map_err(|e| SnipError::runtime_error("Failed to connect", Some(&e.to_string())))?;

        let mut client = SnippetSyncClient::new(channel);

        let request = tonic::Request::new(RegisterRequest {
            device_id: String::new(),
        });

        let response = client
            .register(request)
            .await
            .map_err(|e| SnipError::runtime_error("Register request failed", Some(&e.to_string())))?
            .into_inner();

        if response.success {
            Ok((response.api_key, response.device_id))
        } else {
            Err(SnipError::runtime_error(
                "Registration failed",
                Some(&response.message),
            ))
        }
    }

    pub async fn list_libraries(&mut self) -> SnipResult<Vec<Library>> {
        let request = tonic::Request::new(ListLibrariesRequest {
            api_key: self.settings.api_key.clone(),
            limit: 50,
            offset: 0,
        });

        let response = self
            .client
            .list_libraries(request)
            .await
            .map_err(|e| SnipError::runtime_error("List libraries failed", Some(&e.to_string())))?
            .into_inner();

        Ok(response.libraries)
    }

    pub async fn create_library(&mut self, name: &str) -> SnipResult<Library> {
        let request = tonic::Request::new(CreateLibraryRequest {
            api_key: self.settings.api_key.clone(),
            name: name.to_string(),
        });

        let response = self
            .client
            .create_library(request)
            .await
            .map_err(|e| SnipError::runtime_error("Create library failed", Some(&e.to_string())))?
            .into_inner();

        if response.success {
            Ok(Library {
                id: response.library_id,
                name: name.to_string(),
                created_at: chrono::Utc::now().timestamp(),
                snippet_count: 0,
            })
        } else {
            Err(SnipError::runtime_error(
                "Failed to create library",
                Some(&response.message),
            ))
        }
    }

    pub async fn list_premade_libraries(&mut self) -> SnipResult<Vec<PremadeLibrary>> {
        let request = tonic::Request::new(ListPremadeLibrariesRequest {
            api_key: self.settings.api_key.clone(),
        });

        let response = self
            .client
            .list_premade_libraries(request)
            .await
            .map_err(|e| {
                SnipError::runtime_error("List premade libraries failed", Some(&e.to_string()))
            })?
            .into_inner();

        Ok(response.libraries)
    }

    pub async fn get_premade_library(&mut self, filename: &str) -> SnipResult<String> {
        let request = tonic::Request::new(GetPremadeLibraryRequest {
            api_key: self.settings.api_key.clone(),
            filename: filename.to_string(),
        });

        let response = self
            .client
            .get_premade_library(request)
            .await
            .map_err(|e| {
                SnipError::runtime_error("Get premade library failed", Some(&e.to_string()))
            })?
            .into_inner();

        if response.success {
            Ok(response.content)
        } else {
            Err(SnipError::runtime_error(
                "Failed to get premade library",
                Some(&response.message),
            ))
        }
    }
}

async fn create_tls_channel(
    server_url: &str,
) -> Result<Channel, Box<dyn std::error::Error + Send + Sync>> {
    let uri: Uri = server_url.parse()?;

    let tls_config = ClientTlsConfig::new()
        .with_enabled_roots()
        .assume_http2(true);

    let channel = Endpoint::new(uri)?
        .tls_config(tls_config)?
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .connect()
        .await?;

    Ok(channel)
}

pub fn encrypt_snippet(
    api_key: &str,
    snippet: &snip_proto::Snippet,
) -> SnipResult<snip_proto::Snippet> {
    let data = EncryptedSnippetData {
        description: snippet.description.clone(),
        command: snippet.command.clone(),
        tags: snippet.tags.clone(),
    };

    let json = serde_json::to_string(&data)
        .map_err(|e| SnipError::runtime_error("Serialize snippet data", Some(&e.to_string())))?;

    let encrypted = encryption::encrypt(api_key, &json)
        .map_err(|e| SnipError::runtime_error("Encrypt snippet", Some(&e.to_string())))?;

    Ok(snip_proto::Snippet {
        id: snippet.id.clone(),
        description: String::new(),
        command: encrypted,
        tags: vec![],
        created_at: snippet.created_at,
        updated_at: snippet.updated_at,
        device_id: snippet.device_id.clone(),
        deleted: snippet.deleted,
        encrypted: true,
    })
}

pub fn decrypt_snippet(
    api_key: &str,
    snippet: &snip_proto::Snippet,
) -> SnipResult<snip_proto::Snippet> {
    if !snippet.encrypted {
        return Ok(snippet.clone());
    }

    let decrypted = encryption::decrypt(api_key, &snippet.command)
        .map_err(|e| SnipError::runtime_error("Decrypt snippet", Some(&e.to_string())))?;

    let data: EncryptedSnippetData = serde_json::from_str(&decrypted)
        .map_err(|e| SnipError::runtime_error("Deserialize snippet data", Some(&e.to_string())))?;

    Ok(snip_proto::Snippet {
        id: snippet.id.clone(),
        description: data.description,
        command: data.command,
        tags: data.tags,
        created_at: snippet.created_at,
        updated_at: snippet.updated_at,
        device_id: snippet.device_id.clone(),
        deleted: snippet.deleted,
        encrypted: false,
    })
}
