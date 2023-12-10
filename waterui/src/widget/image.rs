use crate::{view, View};
use smol::{fs::File, io::AsyncReadExt, io::BufReader};
use url::Url;
use waterui_core::component::RawImage;

use super::AsyncView;

#[view(use_core)]
pub struct Image {
    url: Url,
}

#[view(use_core)]
impl View for Image {
    fn view(&mut self) -> impl View {
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
    }
}
