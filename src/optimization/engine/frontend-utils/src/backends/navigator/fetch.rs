use reqwest::Response as ReqwestResponse;
use ruffle_core::backend::navigator::{OwnedFuture, SuccessResponse};
use ruffle_core::loader::Error;
use ruffle_core::swf::Encoding;
use std::fs::File;
use std::io::Read;
use std::sync::{Arc, Mutex};

pub enum ResponseBody {
    /// The response's body comes from a file.
    File(Result<Vec<u8>, std::io::Error>),

    /// An opened disk cache entry, read only when the caller consumes it.
    Disk(Arc<Mutex<File>>, u64),

    /// The response's body comes from the network.
    ///
    /// This has to be stored in shared ownership so that we can return
    /// owned futures. A synchronous lock is used here as we do not
    /// expect contention on this lock.
    Network(Arc<Mutex<Option<ReqwestResponse>>>),
}

impl ResponseBody {
    pub(super) fn disk(file: File, length: u64) -> Self {
        Self::Disk(Arc::new(Mutex::new(file)), length)
    }
}

pub struct Response {
    pub url: String,
    pub response_body: ResponseBody,
    pub text_encoding: Option<&'static Encoding>,
    pub status: u16,
    pub redirected: bool,
}

/// MovieLoader and URLLoader request the entire body. Other consumers may
/// stream the same URL and must keep their original chunk boundaries.
pub(super) trait MovieBodyFilter: 'static {
    async fn complete(self, bytes: Vec<u8>) -> Vec<u8>;
    fn fallback(&self, reason: &'static str);
}

struct MovieResponse<F: MovieBodyFilter> {
    inner: Box<dyn SuccessResponse>,
    filter: Option<F>,
}

pub(super) fn filter_movie<F: MovieBodyFilter>(
    inner: Box<dyn SuccessResponse>, filter: F,
) -> Box<dyn SuccessResponse> {
    Box::new(MovieResponse { inner, filter: Some(filter) })
}

impl<F: MovieBodyFilter> SuccessResponse for MovieResponse<F> {
    fn url(&self) -> std::borrow::Cow<'_, str> { self.inner.url() }
    fn set_url(&mut self, url: String) { self.inner.set_url(url); }
    fn text_encoding(&self) -> Option<&'static Encoding> { self.inner.text_encoding() }
    fn status(&self) -> u16 { self.inner.status() }
    fn redirected(&self) -> bool { self.inner.redirected() }
    fn expected_length(&self) -> Result<Option<u64>, Error> { self.inner.expected_length() }

    fn body(self: Box<Self>) -> OwnedFuture<Vec<u8>, Error> {
        let Self { inner, filter } = *self;
        let Some(filter) = filter else { return inner.body() };
        Box::pin(async move {
            let bytes = inner.body().await.inspect_err(|_| filter.fallback("read-failed"))?;
            Ok(filter.complete(bytes).await)
        })
    }

    fn next_chunk(&mut self) -> OwnedFuture<Option<Vec<u8>>, Error> {
        if let Some(filter) = self.filter.take() {
            filter.fallback("streaming-official");
        }
        self.inner.next_chunk()
    }
}

impl SuccessResponse for Response {
    fn url(&self) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed(&self.url)
    }

    fn set_url(&mut self, url: String) {
        self.url = url;
    }

    #[expect(clippy::await_holding_lock)]
    fn body(self: Box<Self>) -> OwnedFuture<Vec<u8>, Error> {
        let diagnostic_url = ruffle_core::aola_panel_diagnostics::enabled().then(|| self.url.clone());
        match self.response_body {
            ResponseBody::File(file) => {
                Box::pin(async move { file.map_err(|e| Error::FetchError(e.to_string())) })
            }
            ResponseBody::Disk(file, length) => Box::pin(async move {
                read_disk(file, length as usize, true).await
            }),
            ResponseBody::Network(response) => Box::pin(async move {
                let result = response
                    .lock()
                    .expect("working lock during fetch body read")
                    .take()
                    .expect("Body cannot already be consumed")
                    .bytes()
                    .await
                    .map_err(|e| Error::FetchError(e.to_string()));
                match result {
                    Ok(bytes) => {
                        if let Some(url) = &diagnostic_url { ruffle_core::aola_panel_diagnostics::resource(url, "resource-body", bytes.len() as u64); }
                        Ok(bytes.to_vec())
                    }
                    Err(error) => {
                        if let Some(url) = &diagnostic_url { ruffle_core::aola_panel_diagnostics::resource(url, "resource-body-error", 0); }
                        Err(error)
                    }
                }
            }),
        }
    }

    fn text_encoding(&self) -> Option<&'static Encoding> {
        self.text_encoding
    }

    fn status(&self) -> u16 {
        self.status
    }

    fn redirected(&self) -> bool {
        self.redirected
    }

    #[expect(clippy::await_holding_lock)]
    fn next_chunk(&mut self) -> OwnedFuture<Option<Vec<u8>>, Error> {
        match &mut self.response_body {
            ResponseBody::File(file) => {
                let res = file
                    .as_mut()
                    .map(std::mem::take)
                    .map_err(|e| Error::FetchError(e.to_string()));

                Box::pin(async move {
                    match res {
                        Ok(bytes) if !bytes.is_empty() => Ok(Some(bytes)),
                        Ok(_) => Ok(None),
                        Err(e) => Err(e),
                    }
                })
            }
            ResponseBody::Disk(file, _) => {
                let file = file.clone();
                Box::pin(async move {
                    let bytes = read_disk(file, 64 * 1024, false).await?;
                    Ok((!bytes.is_empty()).then_some(bytes))
                })
            }
            ResponseBody::Network(response) => {
                let response = response.clone();
                Box::pin(async move {
                    let lock = response.try_lock();
                    if matches!(lock, Err(std::sync::TryLockError::WouldBlock)) {
                        return Err(Error::FetchError(
                            "Concurrent read operations on the same stream are not supported."
                                .to_string(),
                        ));
                    }

                    let result = lock
                        .expect("desktop network lock")
                        .as_mut()
                        .expect("Body cannot already be consumed")
                        .chunk()
                        .await;

                    match result {
                        Ok(Some(bytes)) => Ok(Some(bytes.to_vec())),
                        Ok(None) => Ok(None),
                        Err(e) => Err(Error::FetchError(e.to_string())),
                    }
                })
            }
        }
    }

    fn expected_length(&self) -> Result<Option<u64>, Error> {
        match &self.response_body {
            ResponseBody::File(file) => Ok(file.as_ref().map(|file| file.len() as u64).ok()),
            ResponseBody::Disk(_, length) => Ok(Some(*length)),
            ResponseBody::Network(response) => {
                let lock = response.lock().expect("no recursive locks");
                let response = lock.as_ref().expect("Body cannot already be consumed");
                Ok(response.content_length())
            }
        }
    }
}

async fn read_disk(file: Arc<Mutex<File>>, length: usize, complete: bool) -> Result<Vec<u8>, Error> {
    super::spawn_tokio(async move {
        tokio::task::spawn_blocking(move || {
            let mut file = file.lock().map_err(|_| std::io::Error::other("Cache read lock poisoned"))?;
            let mut bytes = Vec::with_capacity(length);
            (&mut *file).take(length as u64).read_to_end(&mut bytes)?;
            if complete && bytes.len() != length {
                return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "Adapted cache entry truncated"));
            }
            Ok(bytes)
        }).await
    }).await.map_err(|error| Error::FetchError(error.to_string()))?
        .map_err(|error| Error::FetchError(error.to_string()))
}
