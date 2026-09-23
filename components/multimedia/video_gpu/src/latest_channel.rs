//! Single-slot asynchronous channels for replaceable state and commands.

use async_channel::{Receiver, Sender, TryRecvError, TrySendError};

/// Sender that replaces an unread value instead of accumulating stale work.
#[derive(Debug)]
pub struct LatestSender<T> {
    sender: Sender<T>,
    discard: Receiver<T>,
}

impl<T> LatestSender<T> {
    /// Publishes `value`, replacing the unread value when the slot is full.
    pub fn send(&self, value: T) -> Result<(), TrySendError<T>> {
        let mut pending = value;
        loop {
            match self.sender.try_send(pending) {
                Ok(()) => return Ok(()),
                Err(TrySendError::Full(value)) => {
                    pending = value;
                    match self.discard.try_recv() {
                        Ok(_) | Err(TryRecvError::Empty) => {}
                        Err(TryRecvError::Closed) => {
                            return Err(TrySendError::Closed(pending));
                        }
                    }
                }
                Err(TrySendError::Closed(value)) => return Err(TrySendError::Closed(value)),
            }
        }
    }
}

/// Creates a one-value channel whose sender keeps only the latest unread value.
pub fn latest_channel<T>() -> (LatestSender<T>, Receiver<T>) {
    let (sender, receiver) = async_channel::bounded(1);
    (
        LatestSender {
            sender,
            discard: receiver.clone(),
        },
        receiver,
    )
}

#[cfg(test)]
mod tests {
    use super::latest_channel;

    #[test]
    fn unread_value_is_replaced_without_growing_the_queue() {
        let (sender, receiver) = latest_channel();
        sender.send(1_u8).expect("first value must publish");
        sender.send(2_u8).expect("newest value must replace first");

        assert_eq!(
            receiver
                .try_recv()
                .expect("latest value must remain queued"),
            2
        );
        assert!(receiver.is_empty());
    }
}
