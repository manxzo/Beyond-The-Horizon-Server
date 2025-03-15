use base64::{engine::general_purpose::STANDARD, Engine};
use log::{debug, error, info};
use reqwest::{header, Client};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::error::Error;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// Backblaze B2 API response structures
#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct AuthorizeAccountResponse {
    #[serde(rename = "absoluteMinimumPartSize")]
    pub absolute_minimum_part_size: u64,
    #[serde(rename = "accountId")]
    pub account_id: String,
    pub allowed: AllowedCapabilities,
    #[serde(rename = "apiUrl")]
    pub api_url: String,
    #[serde(rename = "authorizationToken")]
    pub authorization_token: String,
    #[serde(rename = "downloadUrl")]
    pub download_url: String,
    #[serde(rename = "recommendedPartSize")]
    pub recommended_part_size: u64,
    #[serde(rename = "s3ApiUrl")]
    pub s3_api_url: String,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct AllowedCapabilities {
    pub capabilities: Vec<String>,
    pub bucket_id: Option<String>,
    pub bucket_name: Option<String>,
    pub name_prefix: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct GetUploadUrlResponse {
    #[serde(rename = "authorizationToken")]
    pub authorization_token: String,
    #[serde(rename = "bucketId")]
    pub bucket_id: String,
    #[serde(rename = "uploadUrl")]
    pub upload_url: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct UploadFileResponse {
    #[serde(rename = "accountId")]
    pub account_id: String,
    pub action: Option<String>,
    #[serde(rename = "bucketId")]
    pub bucket_id: String,
    #[serde(rename = "contentLength")]
    pub content_length: u64,
    #[serde(rename = "contentMd5")]
    pub content_md5: Option<String>,
    #[serde(rename = "contentSha1")]
    pub content_sha1: String,
    #[serde(rename = "contentType")]
    pub content_type: String,
    #[serde(rename = "fileId")]
    pub file_id: String,
    #[serde(rename = "fileInfo")]
    pub file_info: Option<serde_json::Value>,
    #[serde(rename = "fileName")]
    pub file_name: String,
    #[serde(rename = "uploadTimestamp")]
    pub upload_timestamp: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeleteFileRequest {
    pub file_name: String,
    pub file_id: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct DeleteFileResponse {
    pub file_id: String,
    pub file_name: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ListFileNamesResponse {
    pub files: Vec<FileInfo>,
    pub next_file_name: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct FileInfo {
    pub file_id: String,
    pub file_name: String,
    pub content_type: String,
    pub content_length: u64,
    pub upload_timestamp: u64,
}

// B2 client with caching for auth tokens
#[derive(Clone)]
pub struct B2Client {
    client: Client,
    auth_data: Arc<Mutex<Option<AuthorizeAccountResponse>>>,
    auth_time: Arc<Mutex<Option<Instant>>>,
    application_key_id: String,
    application_key: String,
    bucket_id: String,
}

impl B2Client {
    pub fn new(
        application_key_id: String,
        application_key: String,
        bucket_id: String,
    ) -> Result<Self, Box<dyn Error>> {
        let client = Client::builder().timeout(Duration::from_secs(60)).build()?;

        Ok(B2Client {
            client,
            auth_data: Arc::new(Mutex::new(None)),
            auth_time: Arc::new(Mutex::new(None)),
            application_key_id,
            application_key,
            bucket_id,
        })
    }

    // Create a new B2Client from a SecretStore
    pub fn from_secrets(secrets: &shuttle_runtime::SecretStore) -> Result<Self, Box<dyn Error>> {
        let application_key_id = secrets
            .get("B2_APPLICATION_KEY_ID")
            .ok_or("B2_APPLICATION_KEY_ID not found in secrets")?
            .to_string();

        let application_key = secrets
            .get("B2_APPLICATION_KEY")
            .ok_or("B2_APPLICATION_KEY not found in secrets")?
            .to_string();

        let bucket_id = secrets
            .get("B2_BUCKET_ID")
            .ok_or("B2_BUCKET_ID not found in secrets")?
            .to_string();

        Self::new(application_key_id, application_key, bucket_id)
    }

    // Authorize account and get auth token
    async fn authorize_account(&self) -> Result<AuthorizeAccountResponse, Box<dyn Error>> {
        // Check if we have a valid auth token (less than 23 hours old)
        let auth_time_guard = self.auth_time.lock().unwrap();
        let auth_data_guard = self.auth_data.lock().unwrap();

        if let (Some(auth_time), Some(auth_data)) = (&*auth_time_guard, &*auth_data_guard) {
            if auth_time.elapsed() < Duration::from_secs(23 * 60 * 60) {
                debug!("Using cached B2 authorization token");
                return Ok(auth_data.clone());
            }
        }
        drop(auth_time_guard);
        drop(auth_data_guard);

        info!(
            "Authorizing B2 account with key ID: {}",
            self.application_key_id
        );

        // Create basic auth header
        let auth = format!("{}:{}", self.application_key_id, self.application_key);
        let encoded_auth = STANDARD.encode(auth);

        // Make the authorization request
        let response = self
            .client
            .get("https://api.backblazeb2.com/b2api/v2/b2_authorize_account")
            .header(header::AUTHORIZATION, format!("Basic {}", encoded_auth))
            .send()
            .await?;

        // Log the status code
        info!("B2 authorization response status: {}", response.status());

        if !response.status().is_success() {
            let error_text = response.text().await?;
            error!("B2 authorization failed: {}", error_text);
            return Err(format!("B2 authorization failed: {}", error_text).into());
        }

        // Get the response body as text first for logging
        let response_text = response.text().await?;
        debug!("B2 authorization response: {}", response_text);

        // Parse the response
        let auth_data: AuthorizeAccountResponse = match serde_json::from_str(&response_text) {
            Ok(data) => data,
            Err(e) => {
                error!("Failed to parse B2 authorization response: {}", e);
                error!("Response was: {}", response_text);
                return Err(format!("Failed to parse B2 authorization response: {}", e).into());
            }
        };

        // Log successful authorization
        info!(
            "B2 authorization successful. API URL: {}",
            auth_data.api_url
        );

        // Cache the auth data
        let mut auth_data_guard = self.auth_data.lock().unwrap();
        *auth_data_guard = Some(auth_data.clone());
        drop(auth_data_guard);

        let mut auth_time_guard = self.auth_time.lock().unwrap();
        *auth_time_guard = Some(Instant::now());

        Ok(auth_data)
    }

    // Get upload URL
    async fn get_upload_url(&self) -> Result<GetUploadUrlResponse, Box<dyn Error>> {
        let auth = self.authorize_account().await?;

        info!("Getting upload URL for bucket: {}", self.bucket_id);

        let response = self
            .client
            .post(format!("{}/b2api/v2/b2_get_upload_url", auth.api_url))
            .header(header::AUTHORIZATION, &auth.authorization_token)
            .json(&serde_json::json!({
                "bucketId": self.bucket_id
            }))
            .send()
            .await?;

        // Log the status code
        info!("B2 get_upload_url response status: {}", response.status());

        if !response.status().is_success() {
            let error_text = response.text().await?;
            error!("Failed to get upload URL: {}", error_text);
            return Err(format!("Failed to get upload URL: {}", error_text).into());
        }

        // Get the response body as text first for logging
        let response_text = response.text().await?;
        info!("B2 get_upload_url response: {}", response_text);

        // Parse the response
        let upload_url: GetUploadUrlResponse = match serde_json::from_str(&response_text) {
            Ok(url) => url,
            Err(e) => {
                error!("Failed to parse upload URL response: {}", e);
                error!("Response was: {}", response_text);
                return Err(format!("Failed to parse upload URL response: {}", e).into());
            }
        };

        info!("Got B2 upload URL: {}", upload_url.upload_url);
        Ok(upload_url)
    }

    // Upload file to B2
    pub async fn upload_file(
        &self,
        file_data: &[u8],
        file_name: &str,
        content_type: &str,
    ) -> Result<String, Box<dyn Error>> {
        let upload_url = self.get_upload_url().await?;

        // Calculate SHA1 hash
        let mut hasher = Sha1::new();
        hasher.update(file_data);
        let sha1_hash = hasher.finalize();
        let sha1_hex = format!("{:x}", sha1_hash);

        info!(
            "Uploading file {} ({} bytes) to B2 with URL: {}",
            file_name,
            file_data.len(),
            upload_url.upload_url
        );

        // Upload the file
        let response = self
            .client
            .post(&upload_url.upload_url)
            .header(header::AUTHORIZATION, &upload_url.authorization_token)
            .header("X-Bz-File-Name", file_name)
            .header("Content-Type", content_type)
            .header("Content-Length", file_data.len().to_string())
            .header("X-Bz-Content-Sha1", sha1_hex)
            .body(file_data.to_vec())
            .send()
            .await?;

        // Log the status code
        info!("B2 upload_file response status: {}", response.status());

        if !response.status().is_success() {
            let error_text = response.text().await?;
            error!("Failed to upload file: {}", error_text);
            return Err(format!("Failed to upload file: {}", error_text).into());
        }

        // Get the response body as text first for logging
        let response_text = response.text().await?;
        info!("B2 upload_file response: {}", response_text);

        // Parse the response
        let upload_response: UploadFileResponse = match serde_json::from_str(&response_text) {
            Ok(resp) => resp,
            Err(e) => {
                error!("Failed to parse upload file response: {}", e);
                error!("Response was: {}", response_text);
                return Err(format!("Failed to parse upload file response: {}", e).into());
            }
        };

        // Construct the download URL
        let auth = self.authorize_account().await?;
        let download_url = format!(
            "{}/file/{}/{}",
            auth.download_url, upload_response.bucket_id, upload_response.file_name
        );

        info!("File uploaded successfully: {}", download_url);
        Ok(download_url)
    }

    // Find file ID by name
    async fn find_file_id(&self, file_name: &str) -> Result<Option<String>, Box<dyn Error>> {
        let auth = self.authorize_account().await?;

        let response = self
            .client
            .post(format!("{}/b2api/v2/b2_list_file_names", auth.api_url))
            .header(header::AUTHORIZATION, &auth.authorization_token)
            .json(&serde_json::json!({
                "bucketId": self.bucket_id,
                "prefix": file_name,
                "maxFileCount": 1
            }))
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            error!("Failed to list files: {}", error_text);
            return Err(format!("Failed to list files: {}", error_text).into());
        }

        let list_response: ListFileNamesResponse = response.json().await?;

        // Find the exact file
        for file in list_response.files {
            if file.file_name == file_name {
                return Ok(Some(file.file_id));
            }
        }

        Ok(None)
    }

    // Delete file from B2
    pub async fn delete_file(&self, file_name: &str) -> Result<(), Box<dyn Error>> {
        // First, find the file ID
        let file_id = match self.find_file_id(file_name).await? {
            Some(id) => id,
            None => {
                info!("File not found for deletion: {}", file_name);
                return Ok(());
            }
        };

        let auth = self.authorize_account().await?;

        let response = self
            .client
            .post(format!("{}/b2api/v2/b2_delete_file_version", auth.api_url))
            .header(header::AUTHORIZATION, &auth.authorization_token)
            .json(&DeleteFileRequest {
                file_name: file_name.to_string(),
                file_id,
            })
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            error!("Failed to delete file: {}", error_text);
            return Err(format!("Failed to delete file: {}", error_text).into());
        }

        info!("File deleted successfully: {}", file_name);
        Ok(())
    }
}
