use std::{error::Error, future::Future, pin::Pin};

use axum::{extract::Request, http::header};
use tower_service::Service;

pub struct Decompressor<T> {
    inner: T,
}

impl<T> Service<Request> for Decompressor<T>
where
    T: Service<Request>,
    T::Future: 'static,
    T::Error: Into<Box<dyn Error + Send + Sync>> + 'static,
    T::Response: 'static,
{
    type Response = T::Response;

    type Error = Box<dyn Error + Send + Sync>;

    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>>>>;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, req: Request) -> Self::Future {
        //find out wheather client supports zstd
        let decomp = req
            .headers()
            .get(header::ACCEPT_ENCODING)
            .map(|h| !h.to_str().unwrap_or_default().contains("zstd"))
            .unwrap_or(true);
        let resp = self.inner.call(req);
        if decomp {
            let real_resp = resp.await.unwrap();
        } else {
            //zstd is ok therefore no processing needed
            Box::pin(resp)
        }
    }
}
