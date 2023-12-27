use smol::spawn;

pub fn task<Fut>(future: Fut)
where
    Fut: std::future::Future + Send + 'static,
    Fut::Output: Send + 'static,
{
    spawn(future).detach()
}
