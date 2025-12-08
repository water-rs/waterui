pub struct HotreloadServer {}

#[derive(Debug)]
pub enum FailToLaunch {}

impl HotreloadServer {
    pub async fn launch(_port: u16) -> Result<Self, FailToLaunch> {
        todo!()
    }
}
