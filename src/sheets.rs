use anyhow::{anyhow, Context, Result};
use google_sheets4::{
    api::{Sheets, ValueRange},
    hyper_rustls, hyper_util,
    yup_oauth2::{
        authenticator::ApplicationDefaultCredentialsTypes,
        ApplicationDefaultCredentialsAuthenticator, ApplicationDefaultCredentialsFlowOpts,
    },
};
use hyper_rustls::HttpsConnector;
use hyper_util::client::legacy::connect::HttpConnector;
use tracing::info;

type Hub = Sheets<HttpsConnector<HttpConnector>>;

pub struct SheetsClient {
    hub: Hub,
    sheet_id: String,
}

impl SheetsClient {
    pub async fn new(sheet_id: String) -> Result<Self> {
        let opts = ApplicationDefaultCredentialsFlowOpts::default();
        let auth = match ApplicationDefaultCredentialsAuthenticator::builder(opts).await {
            ApplicationDefaultCredentialsTypes::InstanceMetadata(builder) => builder
                .build()
                .await
                .context("building GCE-metadata authenticator")?,
            ApplicationDefaultCredentialsTypes::ServiceAccount(builder) => builder
                .build()
                .await
                .context("building service-account authenticator")?,
        };

        let connector = hyper_rustls::HttpsConnectorBuilder::new()
            .with_native_roots()
            .context("loading native TLS roots")?
            .https_or_http()
            .enable_http1()
            .enable_http2()
            .build();
        let client =
            hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
                .build(connector);

        let hub = Sheets::new(client, auth);
        Ok(Self { hub, sheet_id })
    }

    pub async fn list_tabs(&self) -> Result<Vec<String>> {
        let (_, sheet) = self
            .hub
            .spreadsheets()
            .get(&self.sheet_id)
            .doit()
            .await
            .with_context(|| format!("fetching spreadsheet metadata for {}", self.sheet_id))?;
        let title = sheet
            .properties
            .as_ref()
            .and_then(|p| p.title.clone())
            .unwrap_or_else(|| "(untitled)".into());
        info!("Writing to sheet titled \"{}\"", title);

        let tabs = sheet
            .sheets
            .unwrap_or_default()
            .into_iter()
            .filter_map(|s| s.properties?.title)
            .collect();
        Ok(tabs)
    }

    pub async fn append_row(&self, tab: &str, row: Vec<String>) -> Result<()> {
        let values: Vec<serde_json::Value> =
            row.into_iter().map(serde_json::Value::String).collect();
        let req = ValueRange {
            values: Some(vec![values]),
            ..Default::default()
        };
        let range = format!("{tab}!A1");
        let (_, response) = self
            .hub
            .spreadsheets()
            .values_append(req, &self.sheet_id, &range)
            .value_input_option("RAW")
            .doit()
            .await
            .with_context(|| format!("appending row to sheet tab '{tab}'"))?;
        let updated_rows = response
            .updates
            .as_ref()
            .and_then(|u| u.updated_rows)
            .ok_or_else(|| anyhow!("Sheets append did not report updated rows"))?;
        info!(
            "Inserted {} new row(s) in spreadsheet tab \"{}\"",
            updated_rows, tab
        );
        Ok(())
    }
}
