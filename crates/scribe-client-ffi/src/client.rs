//! The document-conversion client.

use std::{collections::HashMap, sync::Arc};

use scribe_client_core::{DocumentSource, ScribeClient, SettingsUpdate as CoreSettingsUpdate};

use crate::{
    http_client, parse_url, runtime, AccountInfo, BrailleTable, CreatedDocument, Dialect,
    DocumentList, FfiDocumentChannel, Language, NotificationSettings, Output, OutputFormat,
    ScribeError, Settings, SettingsUpdate, TokenSet, TrashedDocument, Voice,
};

/// A client for the document-conversion endpoints. Holds a token set and
/// refreshes it automatically. Call [`FfiScribeClient::current_tokens`] after
/// any operation to persist the potentially-refreshed token set.
#[derive(uniffi::Object)]
pub struct FfiScribeClient {
    inner: ScribeClient,
}

#[uniffi::export]
impl FfiScribeClient {
    #[uniffi::constructor]
    pub fn new(
        base_url: String,
        client_id: String,
        tokens: TokenSet,
    ) -> Result<Arc<Self>, ScribeError> {
        let base_url = parse_url(&base_url)?;
        let http = http_client();
        let core_tokens: scribe_client_core::TokenSet = tokens.into();
        Ok(Arc::new(FfiScribeClient {
            inner: ScribeClient::new(http, base_url, client_id, core_tokens),
        }))
    }

    /// Returns the current token set, including any access token that was
    /// auto-refreshed since construction. Persist this after each operation.
    pub fn current_tokens(&self) -> TokenSet {
        runtime().block_on(self.inner.current_tokens()).into()
    }

    pub fn create_document_from_file(
        &self,
        file_name: String,
        bytes: Vec<u8>,
    ) -> Result<CreatedDocument, ScribeError> {
        let source = DocumentSource::File { file_name, bytes };
        runtime()
            .block_on(self.inner.create_document(source))
            .map(|d| CreatedDocument {
                document_id: d.document_id,
            })
            .map_err(Into::into)
    }

    pub fn create_document_from_url(&self, url: String) -> Result<CreatedDocument, ScribeError> {
        let source = DocumentSource::Url(url);
        runtime()
            .block_on(self.inner.create_document(source))
            .map(|d| CreatedDocument {
                document_id: d.document_id,
            })
            .map_err(Into::into)
    }

    pub fn list_documents(&self) -> Result<DocumentList, ScribeError> {
        runtime()
            .block_on(self.inner.list_documents())
            .map(|dl| DocumentList {
                documents: dl.documents.into_iter().map(Into::into).collect(),
                pages_remaining: dl.pages_remaining,
            })
            .map_err(Into::into)
    }

    /// Moves a document to the trash. It's permanently deleted 7 days
    /// later, or sooner per the owner's org retention policy, unless
    /// recovered first with `recover_document`.
    pub fn trash_document(&self, document_id: String) -> Result<(), ScribeError> {
        runtime()
            .block_on(self.inner.trash_document(&document_id))
            .map_err(Into::into)
    }

    /// Permanently deletes a document and all of its outputs. The document
    /// must already be in the trash (see `trash_document`).
    pub fn delete_document_permanently(&self, document_id: String) -> Result<(), ScribeError> {
        runtime()
            .block_on(self.inner.delete_document_permanently(&document_id))
            .map_err(Into::into)
    }

    /// Restores a trashed document, clearing its trash state.
    pub fn recover_document(&self, document_id: String) -> Result<(), ScribeError> {
        runtime()
            .block_on(self.inner.recover_document(&document_id))
            .map_err(Into::into)
    }

    /// Lists the caller's trashed documents, most recently trashed first.
    pub fn list_trashed_documents(&self) -> Result<Vec<TrashedDocument>, ScribeError> {
        runtime()
            .block_on(self.inner.list_trashed_documents())
            .map(|documents| documents.into_iter().map(Into::into).collect())
            .map_err(Into::into)
    }

    pub fn submit_document_feedback(
        &self,
        document_id: String,
        comment: String,
    ) -> Result<(), ScribeError> {
        runtime()
            .block_on(self.inner.submit_document_feedback(&document_id, &comment))
            .map_err(Into::into)
    }

    /// Opens a real-time channel for `document_id`. This is the only way
    /// to start converting a format other than the `html_stream` preview
    /// that document creation already starts.
    ///
    /// # Errors
    ///
    /// Returns [`ScribeError::NotFound`]/[`ScribeError::Forbidden`] if the
    /// document doesn't exist or isn't owned by the caller, or an error if
    /// the connection fails.
    pub fn open_document_channel(
        &self,
        document_id: String,
    ) -> Result<Arc<FfiDocumentChannel>, ScribeError> {
        runtime()
            .block_on(self.inner.open_document_channel(&document_id))
            .map(|inner| Arc::new(FfiDocumentChannel::new(inner)))
            .map_err(Into::into)
    }

    pub fn list_outputs(&self, document_id: String) -> Result<Vec<Output>, ScribeError> {
        runtime()
            .block_on(self.inner.list_outputs(&document_id))
            .map(|outs| outs.into_iter().map(Into::into).collect())
            .map_err(Into::into)
    }

    /// Downloads the raw bytes of a completed output.
    /// Returns `ScribeError::ConversionNotComplete` if still in progress.
    pub fn download_output(
        &self,
        document_id: String,
        format: OutputFormat,
    ) -> Result<Vec<u8>, ScribeError> {
        runtime()
            .block_on(self.inner.download_output(&document_id, format.into()))
            .map_err(Into::into)
    }

    pub fn get_settings(&self, document_id: String) -> Result<Settings, ScribeError> {
        runtime()
            .block_on(self.inner.get_settings(&document_id))
            .map(Into::into)
            .map_err(Into::into)
    }

    pub fn update_settings(
        &self,
        document_id: String,
        update: SettingsUpdate,
    ) -> Result<Settings, ScribeError> {
        let core_update: CoreSettingsUpdate = update.into();
        runtime()
            .block_on(self.inner.update_settings(&document_id, &core_update))
            .map(Into::into)
            .map_err(Into::into)
    }

    pub fn register_device(&self, token: String, platform: String) -> Result<(), ScribeError> {
        runtime()
            .block_on(self.inner.register_device(&token, &platform))
            .map_err(Into::into)
    }

    pub fn unregister_device(&self, token: String) -> Result<(), ScribeError> {
        runtime()
            .block_on(self.inner.unregister_device(&token))
            .map_err(Into::into)
    }

    pub fn get_notification_settings(&self) -> Result<NotificationSettings, ScribeError> {
        runtime()
            .block_on(self.inner.get_notification_settings())
            .map(Into::into)
            .map_err(Into::into)
    }

    pub fn update_notification_settings(
        &self,
        push_notify_when_complete: bool,
    ) -> Result<NotificationSettings, ScribeError> {
        runtime()
            .block_on(
                self.inner
                    .update_notification_settings(push_notify_when_complete),
            )
            .map(Into::into)
            .map_err(Into::into)
    }

    pub fn get_account_info(&self) -> Result<AccountInfo, ScribeError> {
        runtime()
            .block_on(self.inner.get_account_info())
            .map(Into::into)
            .map_err(Into::into)
    }

    /// Lists every language available for TTS narration.
    pub fn languages(&self) -> Result<Vec<Language>, ScribeError> {
        runtime()
            .block_on(self.inner.languages())
            .map(|langs| {
                langs
                    .into_iter()
                    .map(|l| Language {
                        display_name: l.0,
                        code: l.1,
                    })
                    .collect()
            })
            .map_err(Into::into)
    }

    /// Lists every dialect available for TTS narration, keyed by language code.
    pub fn dialects(&self) -> Result<HashMap<String, Vec<Dialect>>, ScribeError> {
        runtime()
            .block_on(self.inner.dialects())
            .map(|map| {
                map.into_iter()
                    .map(|(k, v)| {
                        let dialects = v
                            .into_iter()
                            .map(|d| Dialect {
                                display_name: d.0,
                                locale: d.1,
                            })
                            .collect();
                        (k, dialects)
                    })
                    .collect()
            })
            .map_err(Into::into)
    }

    /// Lists every Braille translation table available for `brf` output.
    pub fn braille_tables(&self) -> Result<Vec<BrailleTable>, ScribeError> {
        runtime()
            .block_on(self.inner.braille_tables())
            .map(|tables| {
                tables
                    .into_iter()
                    .map(|t| BrailleTable {
                        display_name: t.0,
                        id: t.1,
                    })
                    .collect()
            })
            .map_err(Into::into)
    }

    /// Lists every TTS voice available, keyed by dialect locale.
    pub fn voices(&self) -> Result<HashMap<String, Vec<Voice>>, ScribeError> {
        runtime()
            .block_on(self.inner.voices())
            .map(|map| {
                map.into_iter()
                    .map(|(k, v)| {
                        let voices = v
                            .into_iter()
                            .map(|voice| Voice {
                                display_name: voice.0,
                                short_name: voice.1,
                                has_sample: voice.2,
                            })
                            .collect();
                        (k, voices)
                    })
                    .collect()
            })
            .map_err(Into::into)
    }
}
