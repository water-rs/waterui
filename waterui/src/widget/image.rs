use crate::{View, ViewExt};
use smol::{fs::File, io::AsyncReadExt, io::BufReader};
use url::Url;
use waterui_core::{component::RawImage, BoxView};

use super::AsyncView;

pub struct Image {
    url: Url,
}

impl View for Image {
    fn view(self) -> BoxView {
        let url = self.url.clone();

        AsyncView::new(move || {
            let url = url.clone();
            async move {
                match url.scheme() {
                    "file" => {
                        let file = File::open(url.path()).await?;
                        let len = file.metadata().await?.len();
                        let mut file = BufReader::new(file);
                        let mut buf = Vec::with_capacity(len as usize);
                        file.read_to_end(&mut buf).await?;
                        Ok(RawImage::new(buf))
                    }
                    "http" | "https" => {
                        todo!()
                    }
                    _ => panic!("Unexpected scheme"),
                }
            }
        })
        .boxed()
    }
}
