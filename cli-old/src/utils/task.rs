use color_eyre::eyre;
use smol::{
    channel::{Receiver, Sender},
    spawn,
};

pub struct Task {
    name: String,
    description: String,
    status: Receiver<Status>,
}

impl Task {
    pub fn new<F, Fut, E>(name: impl Into<String>, description: impl Into<String>, f: F) -> Self
    where
        F: FnOnce(Progress) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), E>> + Send + 'static,
        E: Into<eyre::Report> + Send + 'static,
    {
        let (sender, receiver) = smol::channel::unbounded();

        let progress = Progress::new(sender.clone());

        spawn(async move {
            let result = f(progress).await;
            match result {
                Ok(_) => {
                    sender
                        .send(Status::Done)
                        .await
                        .expect("Failed to send done status");
                }
                Err(e) => {
                    sender
                        .send(Status::Failed(e.into()))
                        .await
                        .expect("Failed to send failed status");
                }
            }
        })
        .detach();

        Self {
            name: name.into(),
            description: description.into(),
            status: receiver,
        }
    }
}

pub struct Progress {
    sender: Sender<Status>,
}

impl Progress {
    fn new(sender: Sender<Status>) -> Self {
        Self { sender }
    }

    pub fn start(&self, message: impl Into<String>) {
        self.sender
            .try_send(Status::Started {
                message: message.into(),
            })
            .expect("Failed to send start status");
    }

    pub fn spawn_task<F, Fut>(&self, name: impl Into<String>, f: F) -> Task
    where
        F: FnOnce(Progress) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        todo!()
    }
}

#[derive(Debug, Default)]
pub enum Status {
    /// Task is still ongoing.
    #[default]
    Pending,

    /// Task started.
    Started { message: String },
    /// Progress percentage (0-100) with message.
    Progress { percent: u8, message: String },
    /// Task completed.
    Done,
    /// Task failed.
    Failed(eyre::Report),
}
