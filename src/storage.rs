//! Tarball object storage.
//!
//! The Rails app stores uploads through paperclip, on the filesystem by default
//! and in S3 when `S3_BUCKET`/`S3_REGION` are set (`app/lib/supermarket/s3_config.rb`,
//! `config/initializers/paperclip.rb`). The port keeps the same switch, key
//! layout (`:class/:attachment/:compatible_id/:style/:basename.:extension`),
//! URL styles and write/delete semantics.
//!
//! All S3 calls are lazy: the client is built on first use, so a deployment
//! without S3 never pays for the credential chain.

use std::time::Duration;

use aws_sdk_s3::config::{Credentials, Region};
use aws_sdk_s3::error::ProvideErrorMetadata;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{BucketLocationConstraint, ObjectCannedAcl, ServerSideEncryption};
use tokio::sync::OnceCell;

use crate::config::S3Config;

/// URL shape selected by `S3_DOMAIN_STYLE`/`CDN_URL`, mirroring paperclip's
/// `:s3_domain_url`, `:s3_path_url` and `:s3_alias_url` interpolations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlStyle {
    /// `proto//bucket.host/key`
    Domain,
    /// `proto//host/bucket/key`
    Path,
    /// `proto//cdn_host/key`
    Alias,
}

impl UrlStyle {
    fn parse(value: Option<&str>) -> Self {
        match value {
            Some(":s3_path_url") => UrlStyle::Path,
            Some(":s3_alias_url") => UrlStyle::Alias,
            _ => UrlStyle::Domain,
        }
    }
}

#[derive(Debug, Clone)]
pub struct S3 {
    bucket: String,
    region: String,
    access_key_id: Option<String>,
    secret_access_key: Option<String>,
    /// `S3_PATH`, already normalised without leading/trailing slashes.
    prefix: Option<String>,
    private_objects: bool,
    encryption: Option<String>,
    /// Host name that signs the URLs; the endpoint's authority when one is set.
    host_name: String,
    endpoint: Option<String>,
    style: UrlStyle,
    cdn_url: Option<String>,
    protocol: String,
    client: OnceCell<aws_sdk_s3::Client>,
}

const DEFAULT_S3_HOST: &str = "s3.amazonaws.com";
const MAX_SLOWDOWN_RETRIES: u32 = 5;

impl S3 {
    /// Parses the same variables `Supermarket::S3Config#to_paperclip_options`
    /// consumes. `PROTOCOL` is the URL scheme, exactly as upstream.
    pub fn from_config(config: &S3Config, protocol: &str) -> Self {
        let endpoint = config
            .endpoint
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.trim_end_matches('/').to_owned());
        let host_name = endpoint
            .as_deref()
            .map(|value| {
                value
                    .trim_start_matches("http://")
                    .trim_start_matches("https://")
            })
            .unwrap_or(DEFAULT_S3_HOST)
            .to_owned();
        Self {
            bucket: config.bucket.clone(),
            region: config.region.clone(),
            access_key_id: config.access_key_id.clone(),
            secret_access_key: config.secret_access_key.clone(),
            prefix: config
                .path
                .as_deref()
                .map(str::trim)
                .map(|value| value.trim_matches('/').to_owned())
                .filter(|value| !value.is_empty()),
            private_objects: config.private_objects,
            encryption: config.encryption.clone(),
            host_name,
            endpoint,
            style: if config.cdn_url.is_some() {
                UrlStyle::Alias
            } else {
                UrlStyle::parse(config.domain_style.as_deref())
            },
            cdn_url: config.cdn_url.clone(),
            protocol: protocol.to_owned(),
            client: OnceCell::new(),
        }
    }

    /// Paperclip key: `[S3_PATH/]cookbook_versions/tarballs/<id>/original/<file>`.
    pub fn tarball_key(&self, id: &str, file_name: &str) -> String {
        match &self.prefix {
            Some(prefix) => {
                format!("{prefix}/cookbook_versions/tarballs/{id}/original/{file_name}")
            }
            None => format!("cookbook_versions/tarballs/{id}/original/{file_name}"),
        }
    }

    /// The object's non-presigned URL, in the configured style.
    pub fn url(&self, key: &str) -> String {
        let scheme = if self.protocol.is_empty() {
            String::new()
        } else {
            format!("{}:", self.protocol)
        };
        let path = key.trim_start_matches('/');
        match self.style {
            UrlStyle::Domain => {
                format!("{scheme}//{}.{}/{}", self.bucket, self.host_name, path)
            }
            UrlStyle::Path => format!("{scheme}//{}/{}/{}", self.host_name, self.bucket, path),
            UrlStyle::Alias => {
                let host = self.cdn_url.as_deref().unwrap_or(&self.host_name);
                let host = host.trim_end_matches('/');
                format!(
                    "{scheme}//{}/{path}",
                    host.trim_start_matches("http://")
                        .trim_start_matches("https://")
                )
            }
        }
    }

    /// A presigned GET, `S3_URLS_EXPIRE` seconds of validity.
    pub async fn presigned_get(&self, key: &str, expires_in: u64) -> Result<String, StorageError> {
        let client = self.client().await?;
        let request = client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .presigned(
                aws_sdk_s3::presigning::PresigningConfig::expires_in(Duration::from_secs(
                    expires_in.max(1),
                ))
                .map_err(|error| StorageError(error.to_string()))?,
            )
            .await
            .map_err(|error| StorageError(error.to_string()))?;
        Ok(request.uri().to_owned())
    }

    /// Upload one object. `NoSuchBucket` creates the bucket once and retries,
    /// `SlowDown` backs off up to five times, both as paperclip does.
    pub async fn put(
        &self,
        key: &str,
        body: Vec<u8>,
        content_type: &str,
    ) -> Result<(), StorageError> {
        let client = self.client().await?;
        let mut created_bucket = false;
        let mut slowdowns = 0;
        loop {
            let mut request = client
                .put_object()
                .bucket(&self.bucket)
                .key(key)
                .body(ByteStream::from(body.clone()))
                .content_type(content_type)
                .acl(if self.private_objects {
                    ObjectCannedAcl::Private
                } else {
                    ObjectCannedAcl::PublicRead
                });
            if let Some(encryption) = &self.encryption {
                request =
                    request.server_side_encryption(ServerSideEncryption::from(encryption.as_str()));
            }
            match request.send().await {
                Ok(_) => return Ok(()),
                Err(error) => match error_code(&error).as_deref() {
                    Some("NoSuchBucket") if !created_bucket => {
                        created_bucket = true;
                        self.create_bucket(client).await?;
                    }
                    Some("SlowDown") if slowdowns < MAX_SLOWDOWN_RETRIES => {
                        slowdowns += 1;
                        let pause = Duration::from_millis(u64::from(2u32.pow(slowdowns)) * 500);
                        tokio::time::sleep(pause).await;
                    }
                    _ => return Err(StorageError(error.to_string())),
                },
            }
        }
    }

    /// Delete one object. Paperclip swallows every `ServiceError` here.
    pub async fn delete(&self, key: &str) {
        let Ok(client) = self.client().await else {
            return;
        };
        let _ = client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await;
    }

    async fn create_bucket(&self, client: &aws_sdk_s3::Client) -> Result<(), StorageError> {
        let mut request = client.create_bucket().bucket(&self.bucket);
        if self.region != "us-east-1" {
            request = request.create_bucket_configuration(
                aws_sdk_s3::types::CreateBucketConfiguration::builder()
                    .location_constraint(BucketLocationConstraint::from(self.region.as_str()))
                    .build(),
            );
        }
        request
            .send()
            .await
            .map(|_| ())
            .map_err(|error| StorageError(error.to_string()))
    }

    async fn client(&self) -> Result<&aws_sdk_s3::Client, StorageError> {
        self.client
            .get_or_try_init(|| async {
                let mut loader = aws_config::defaults(aws_config::BehaviorVersion::latest())
                    .region(Region::new(self.region.clone()));
                if let (Some(access_key_id), Some(secret_access_key)) =
                    (&self.access_key_id, &self.secret_access_key)
                {
                    loader = loader.credentials_provider(Credentials::new(
                        access_key_id,
                        secret_access_key,
                        None,
                        None,
                        "S3_ACCESS_KEY_ID",
                    ));
                }
                if let Some(endpoint) = &self.endpoint {
                    loader = loader.endpoint_url(endpoint);
                }
                let shared = loader.load().await;
                let mut builder = aws_sdk_s3::config::Builder::from(&shared);
                if let Some(endpoint) = &self.endpoint {
                    builder = builder.endpoint_url(endpoint).force_path_style(true);
                }
                Ok(aws_sdk_s3::Client::from_conf(builder.build()))
            })
            .await
    }
}

/// The S3 error code, when the SDK modelled one.
fn error_code(
    error: &aws_sdk_s3::error::SdkError<
        aws_sdk_s3::operation::put_object::PutObjectError,
        aws_sdk_s3::config::http::HttpResponse,
    >,
) -> Option<String> {
    error
        .as_service_error()
        .and_then(ProvideErrorMetadata::code)
        .map(str::to_owned)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageError(pub String);

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for StorageError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> S3Config {
        S3Config {
            bucket: "bucket".into(),
            region: "eu-west-1".into(),
            access_key_id: Some("AKIAEXAMPLE".into()),
            secret_access_key: Some("secret".into()),
            path: Some("harness".into()),
            private_objects: false,
            encryption: None,
            endpoint: None,
            domain_style: None,
            cdn_url: None,
            url_expiry: None,
        }
    }

    fn store() -> S3 {
        S3::from_config(&config(), "https")
    }

    #[test]
    fn key_matches_the_paperclip_layout() {
        assert_eq!(
            store().tarball_key("42", "redis-1.0.0.tgz"),
            "harness/cookbook_versions/tarballs/42/original/redis-1.0.0.tgz"
        );
        let mut without_prefix = config();
        without_prefix.path = None;
        assert_eq!(
            S3::from_config(&without_prefix, "https").tarball_key("42", "x.tgz"),
            "cookbook_versions/tarballs/42/original/x.tgz"
        );
    }

    #[test]
    fn domain_style_is_the_default() {
        assert_eq!(
            store().url("harness/a.tgz"),
            "https://bucket.s3.amazonaws.com/harness/a.tgz"
        );
    }

    #[test]
    fn path_style_puts_the_bucket_in_the_path() {
        let mut config = config();
        config.domain_style = Some(":s3_path_url".into());
        let store = S3::from_config(&config, "http");
        assert_eq!(store.url("a.tgz"), "http://s3.amazonaws.com/bucket/a.tgz");
    }

    #[test]
    fn an_endpoint_replaces_the_host() {
        let mut config = config();
        config.endpoint = Some("http://garage:3900/".into());
        config.domain_style = Some(":s3_path_url".into());
        let store = S3::from_config(&config, "http");
        assert_eq!(store.url("a.tgz"), "http://garage:3900/bucket/a.tgz");
    }

    #[test]
    fn cdn_url_wins_over_the_domain_style() {
        let mut config = config();
        config.domain_style = Some(":s3_path_url".into());
        config.cdn_url = Some("https://cdn.example.com".into());
        let store = S3::from_config(&config, "https");
        assert_eq!(
            store.url("harness/a.tgz"),
            "https://cdn.example.com/harness/a.tgz"
        );
    }

    #[test]
    fn an_empty_protocol_yields_a_protocol_relative_url() {
        let mut config = config();
        config.domain_style = Some(":s3_path_url".into());
        let store = S3::from_config(&config, "");
        assert_eq!(store.url("a.tgz"), "//s3.amazonaws.com/bucket/a.tgz");
    }

    #[tokio::test]
    async fn presigning_is_path_styled_and_expires() {
        let mut config = config();
        config.endpoint = Some("http://garage:3900".into());
        let store = S3::from_config(&config, "http");
        let url = store
            .presigned_get("harness/a.tgz", 900)
            .await
            .expect("presign without network");
        assert!(
            url.starts_with("http://garage:3900/bucket/harness/a.tgz?"),
            "{url}"
        );
        assert!(url.contains("X-Amz-Expires=900"), "{url}");
        assert!(url.contains("X-Amz-Signature="), "{url}");
    }
}
