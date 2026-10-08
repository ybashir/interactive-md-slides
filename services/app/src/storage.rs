use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use axum::body::Bytes;
use chrono::Utc;
use hmac::{Hmac, Mac};
use reqwest::{Client, Method, Url};
use sha2::{Digest, Sha256};

use crate::config::Config;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub enum AssetStore {
    Local { root: PathBuf },
    S3(S3Store),
}

#[derive(Clone)]
pub struct S3Store {
    bucket: String,
    endpoint: String,
    region: String,
    access_key_id: String,
    secret_access_key: String,
    url_style: String,
    client: Client,
}

pub struct StoredAsset {
    pub bytes: Bytes,
}

impl AssetStore {
    pub fn from_config(config: &Config) -> Self {
        if let Some(bucket) = &config.asset_s3_bucket {
            return Self::S3(S3Store {
                bucket: bucket.clone(),
                endpoint: config
                    .asset_s3_endpoint
                    .clone()
                    .expect("validated S3 endpoint"),
                region: config
                    .asset_s3_region
                    .clone()
                    .unwrap_or_else(|| "auto".to_owned()),
                access_key_id: config
                    .asset_s3_access_key_id
                    .clone()
                    .expect("validated S3 access key"),
                secret_access_key: config
                    .asset_s3_secret_access_key
                    .clone()
                    .expect("validated S3 secret key"),
                url_style: config.asset_s3_url_style.clone(),
                client: Client::builder()
                    .user_agent("Interdeck/0.1 asset-store")
                    .timeout(std::time::Duration::from_secs(60))
                    .build()
                    .expect("asset HTTP client should build"),
            });
        }
        Self::Local {
            root: PathBuf::from(&config.asset_local_dir),
        }
    }

    pub async fn put(&self, key: &str, content_type: &str, bytes: Bytes) -> Result<()> {
        validate_key(key)?;
        match self {
            Self::Local { root } => {
                let path = root.join(key);
                let parent = path.parent().context("asset key has no parent")?;
                tokio::fs::create_dir_all(parent)
                    .await
                    .context("create local asset directory")?;
                tokio::fs::write(path, bytes)
                    .await
                    .context("write local asset")?;
                Ok(())
            }
            Self::S3(store) => store.put(key, content_type, bytes).await,
        }
    }

    pub async fn get(&self, key: &str) -> Result<StoredAsset> {
        validate_key(key)?;
        match self {
            Self::Local { root } => Ok(StoredAsset {
                bytes: Bytes::from(
                    tokio::fs::read(root.join(key))
                        .await
                        .context("read local asset")?,
                ),
            }),
            Self::S3(store) => store.get(key).await,
        }
    }

    pub async fn delete(&self, key: &str) -> Result<()> {
        validate_key(key)?;
        match self {
            Self::Local { root } => match tokio::fs::remove_file(root.join(key)).await {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error).context("delete local asset"),
            },
            Self::S3(store) => store.delete(key).await,
        }
    }
}

impl S3Store {
    async fn put(&self, key: &str, content_type: &str, bytes: Bytes) -> Result<()> {
        let url = self.object_url(key)?;
        let headers = self.authorization_headers(&Method::PUT, &url, &hex_sha256(&bytes))?;
        let response = self
            .client
            .put(url)
            .headers(headers)
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .body(bytes)
            .send()
            .await
            .context("upload asset to object storage")?;
        if !response.status().is_success() {
            bail!("object storage upload failed with {}", response.status());
        }
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<StoredAsset> {
        let url = self.object_url(key)?;
        let headers = self.authorization_headers(&Method::GET, &url, &hex_sha256(&[]))?;
        let response = self
            .client
            .get(url)
            .headers(headers)
            .send()
            .await
            .context("fetch asset from object storage")?;
        if !response.status().is_success() {
            bail!("object storage fetch failed with {}", response.status());
        }
        Ok(StoredAsset {
            bytes: response.bytes().await.context("read stored asset")?,
        })
    }

    async fn delete(&self, key: &str) -> Result<()> {
        let url = self.object_url(key)?;
        let headers = self.authorization_headers(&Method::DELETE, &url, &hex_sha256(&[]))?;
        let response = self
            .client
            .delete(url)
            .headers(headers)
            .send()
            .await
            .context("delete asset from object storage")?;
        if !response.status().is_success() {
            bail!("object storage delete failed with {}", response.status());
        }
        Ok(())
    }

    fn object_url(&self, key: &str) -> Result<Url> {
        let mut url = Url::parse(&self.endpoint).context("parse asset S3 endpoint")?;
        if self.url_style == "path" {
            url.set_path(&format!("/{}/{key}", self.bucket));
        } else {
            let host = url.host_str().context("asset S3 endpoint has no host")?;
            url.set_host(Some(&format!("{}.{}", self.bucket, host)))
                .map_err(|_| anyhow::anyhow!("invalid asset bucket host"))?;
            url.set_path(&format!("/{key}"));
        }
        Ok(url)
    }

    fn authorization_headers(
        &self,
        method: &Method,
        url: &Url,
        payload_hash: &str,
    ) -> Result<reqwest::header::HeaderMap> {
        let now = Utc::now();
        let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
        let date = now.format("%Y%m%d").to_string();
        let host = match url.port() {
            Some(port) => format!("{}:{port}", url.host_str().context("S3 URL has no host")?),
            None => url.host_str().context("S3 URL has no host")?.to_owned(),
        };
        let canonical_headers =
            format!("host:{host}\nx-amz-content-sha256:{payload_hash}\nx-amz-date:{amz_date}\n");
        let signed_headers = "host;x-amz-content-sha256;x-amz-date";
        let canonical_request = format!(
            "{}\n{}\n\n{canonical_headers}\n{signed_headers}\n{payload_hash}",
            method.as_str(),
            url.path()
        );
        let scope = format!("{date}/{}/s3/aws4_request", self.region);
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
            hex_sha256(canonical_request.as_bytes())
        );
        let date_key = hmac(
            format!("AWS4{}", self.secret_access_key).as_bytes(),
            date.as_bytes(),
        )?;
        let region_key = hmac(&date_key, self.region.as_bytes())?;
        let service_key = hmac(&region_key, b"s3")?;
        let signing_key = hmac(&service_key, b"aws4_request")?;
        let signature = hex_bytes(&hmac(&signing_key, string_to_sign.as_bytes())?);
        let authorization = format!(
            "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
            self.access_key_id
        );

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("x-amz-date", amz_date.parse()?);
        headers.insert("x-amz-content-sha256", payload_hash.parse()?);
        headers.insert(reqwest::header::AUTHORIZATION, authorization.parse()?);
        Ok(headers)
    }
}

fn validate_key(key: &str) -> Result<()> {
    if key.is_empty()
        || key.starts_with('/')
        || key.split('/').any(|part| {
            part.is_empty()
                || part == "."
                || part == ".."
                || !part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        })
    {
        bail!("invalid asset storage key");
    }
    Ok(())
}

fn hex_sha256(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

fn hex_bytes(value: &[u8]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn hmac(key: &[u8], value: &[u8]) -> Result<Vec<u8>> {
    let mut mac = HmacSha256::new_from_slice(key).context("create S3 signing key")?;
    mac.update(value);
    Ok(mac.finalize().into_bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_keys_cannot_escape_the_asset_root() {
        assert!(validate_key("decks/abc/assets/def/photo.png").is_ok());
        assert!(validate_key("../secret").is_err());
        assert!(validate_key("decks/abc/my photo.png").is_err());
        assert!(validate_key("/decks/abc/photo.png").is_err());
    }

    #[test]
    fn virtual_and_path_style_urls_are_supported() {
        let base = S3Store {
            bucket: "deck-storage-test".to_owned(),
            endpoint: "https://storage.example.com".to_owned(),
            region: "auto".to_owned(),
            access_key_id: "test".to_owned(),
            secret_access_key: "test".to_owned(),
            url_style: "virtual".to_owned(),
            client: Client::new(),
        };
        assert_eq!(
            base.object_url("decks/a/assets/b/photo.png")
                .unwrap()
                .as_str(),
            "https://deck-storage-test.storage.example.com/decks/a/assets/b/photo.png"
        );
        let path_style = S3Store {
            url_style: "path".to_owned(),
            ..base
        };
        assert_eq!(
            path_style
                .object_url("decks/a/assets/b/photo.png")
                .unwrap()
                .as_str(),
            "https://storage.example.com/deck-storage-test/decks/a/assets/b/photo.png"
        );
    }
}
