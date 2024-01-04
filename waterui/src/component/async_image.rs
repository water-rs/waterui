use url::Url;

use super::Image;
use crate::{async_view::AsyncView, View};
use std::fmt::Debug;

pub struct AsyncImage<Loading, Error> {
    resource: Url,
    loading: Loading,
    error: Error,
}

impl AsyncImage<(), ()> {
    pub fn new<R>(resource: R) -> Self
    where
        R: TryInto<Url>,
        R::Error: Debug,
    {
        Self {
            resource: resource.try_into().unwrap(),
            loading: (),
            error: (),
        }
    }
}

impl AsyncView for AsyncImage<(), ()> {
    async fn body(self, _env: crate::Environment) -> Result<impl View, anyhow::Error> {
        pull_image(self.resource).await
    }

    fn error(error: anyhow::Error, env: crate::Environment) -> impl View {
        println!("{:?}", error);
    }
}

async fn pull_image(resource: Url) -> Result<Image, anyhow::Error> {
    let mut response = surf::get(resource).await.map_err(|v| v.into_inner())?;
    let data = response.body_bytes().await.map_err(|v| v.into_inner())?;
    println!("image ready");
    Ok(Image::new(data))
}

impl<Loading, LoadingView, Error, ErrorView> AsyncView for AsyncImage<Loading, Error>
where
    Loading: Send + Sync + Fn() -> LoadingView,
    LoadingView: View,
    Error: Send + Sync + Fn() -> ErrorView,
    ErrorView: View,
{
    async fn body(self, _env: crate::Environment) -> Result<impl View, anyhow::Error> {
        pull_image(self.resource).await
    }

    fn loading(env: crate::Environment) -> impl View {}

    fn error(error: anyhow::Error, env: crate::Environment) -> impl View {}
}
