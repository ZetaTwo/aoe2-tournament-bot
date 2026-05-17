use anyhow::{Context, Result};
use backon::Retryable;
use gcloud_storage::{
    client::{Client, ClientConfig},
    http::objects::upload::{Media, UploadObjectRequest, UploadType},
};
use tracing::info;

use crate::retry;

pub struct GcsClient {
    client: Client,
    bucket: String,
}

impl GcsClient {
    pub async fn new(bucket: String) -> Result<Self> {
        let config = ClientConfig::default()
            .with_auth()
            .await
            .context("building GCS ADC client (set GOOGLE_APPLICATION_CREDENTIALS, or run on a GCE VM with an attached service account)")?;
        let client = Client::new(config);
        Ok(Self { client, bucket })
    }

    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    pub async fn upload(&self, object_name: &str, bytes: Vec<u8>) -> Result<()> {
        let upload_type = UploadType::Simple(Media::new(object_name.to_string()));
        let req = UploadObjectRequest {
            bucket: self.bucket.clone(),
            ..Default::default()
        };
        let byte_count = bytes.len();
        // `upload_object` consumes the body, so each retry attempt gets its own
        // clone. Replays are small and this only re-clones on an actual retry.
        (|| async {
            self.client
                .upload_object(&req, bytes.clone(), &upload_type)
                .await
        })
        .retry(retry::backoff())
        .when(retry::gcs_retryable)
        .notify(retry::log_retry("gcs.upload_object"))
        .await
        .with_context(|| {
            format!(
                "uploading object '{object_name}' to bucket '{}'",
                self.bucket
            )
        })?;
        info!(
            "Uploaded {byte_count} bytes to gs://{}/{}",
            self.bucket, object_name
        );
        Ok(())
    }
}
