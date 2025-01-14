use std::convert::TryFrom;

use crate::channel::receiver::ChannelReceiver;
use crate::channel::sender::ChannelSender;
use crate::core::element::Element;
use crate::metrics::metric::Tag;
use crate::metrics::{register_counter, register_gauge};

pub const CHANNEL_CAPACITY_PREFIX: &str = "Channel.Capacity.";
pub const CHANNEL_SIZE_PREFIX: &str = "Channel.Size.";
pub const CHANNEL_ACCEPTED_PREFIX: &str = "Channel.Accepted.";
pub const CHANNEL_DRAIN_PREFIX: &str = "Channel.Drain.";

pub type TrySendError<T> = crossbeam::channel::TrySendError<T>;
pub type TryRecvError = crossbeam::channel::TryRecvError;
pub type RecvTimeoutError = crossbeam::channel::RecvTimeoutError;
pub type SendTimeoutError<T> = crossbeam::channel::SendTimeoutError<T>;
pub type RecvError = crossbeam::channel::RecvError;
pub type SendError<T> = crossbeam::channel::SendError<T>;

pub type Receiver<T> = crossbeam::channel::Receiver<T>;
pub type Sender<T> = crossbeam::channel::Sender<T>;
pub type Select<'a> = crossbeam::channel::Select<'a>;

pub fn unbounded<T>() -> (Sender<T>, Receiver<T>) {
    crossbeam::channel::unbounded()
}

pub fn bounded<T>(cap: usize) -> (Sender<T>, Receiver<T>) {
    crossbeam::channel::bounded(cap)
}

pub mod receiver;
pub mod select;
pub mod sender;
pub mod utils;

pub type ElementReceiver = ChannelReceiver<Element>;
pub type ElementSender = ChannelSender<Element>;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ChannelBaseOn {
    Unbounded,
    Bounded,
}

impl<'a> TryFrom<&'a str> for ChannelBaseOn {
    type Error = anyhow::Error;

    fn try_from(mode_str: &'a str) -> Result<Self, Self::Error> {
        let mode_str = mode_str.to_lowercase();
        match mode_str.as_str() {
            "bounded" => Ok(Self::Bounded),
            "unbounded" => Ok(Self::Unbounded),
            _ => Err(anyhow!("Unsupported mode {}", mode_str)),
        }
    }
}

impl std::fmt::Display for ChannelBaseOn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChannelBaseOn::Bounded => write!(f, "Bounded"),
            ChannelBaseOn::Unbounded => write!(f, "Unbounded"),
        }
    }
}

pub fn named_channel<T>(
    name: &str,
    tags: Vec<Tag>,
    cap: usize,
) -> (ChannelSender<T>, ChannelReceiver<T>)
where
    T: Sync + Send,
{
    named_channel_with_base(name, tags, cap, ChannelBaseOn::Unbounded)
}

pub fn named_channel_with_base<T>(
    name: &str,
    tags: Vec<Tag>,
    cap: usize,
    base_on: ChannelBaseOn,
) -> (ChannelSender<T>, ChannelReceiver<T>)
where
    T: Sync + Send,
{
    info!(
        "Create channel named with {}, capacity: {}, base on: {}",
        name, cap, base_on
    );

    let (sender, receiver) = match base_on {
        ChannelBaseOn::Bounded => bounded(cap),
        ChannelBaseOn::Unbounded => unbounded(),
    };

    // add_channel_metric(name.to_string(), size.clone(), capacity.clone());
    let size = register_gauge(CHANNEL_SIZE_PREFIX.to_owned() + name, tags.clone());
    let accepted_counter =
        register_counter(CHANNEL_ACCEPTED_PREFIX.to_owned() + name, tags.clone());
    let drain_counter = register_counter(CHANNEL_DRAIN_PREFIX.to_owned() + name, tags);

    (
        ChannelSender::new(name, sender, base_on, cap, size.clone(), accepted_counter),
        ChannelReceiver::new(name, receiver, size.clone(), drain_counter),
    )
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::channel::{named_channel_with_base, ChannelBaseOn};
    use crate::utils::date_time::current_timestamp;
    use crate::utils::thread::spawn;

    #[test]
    pub fn bounded_test() {
        let (sender, receiver) = crate::channel::unbounded();
        // let (sender, receiver) = crate::channel::bounded(10000 * 100);

        std::thread::sleep(Duration::from_secs(2));

        for n in 0..100 {
            let sender = sender.clone();
            spawn(n.to_string().as_str(), move || {
                for i in 0..10000 {
                    sender.send(i.to_string()).unwrap();
                }
            });
        }
        {
            let _a = sender;
        }

        let mut begin = Duration::default();
        while let Ok(_n) = receiver.recv() {
            if begin.as_nanos() == 0 {
                begin = current_timestamp();
            }
        }
        let end = current_timestamp();

        println!("{}", end.checked_sub(begin).unwrap().as_nanos());
    }

    #[test]
    pub fn channel_sender_test() {
        let cap = 1 * 1;
        let (sender, receiver) = named_channel_with_base("", vec![], cap, ChannelBaseOn::Bounded);

        let recv_thread_handle = spawn("recv_thread", move || {
            std::thread::sleep(Duration::from_secs(1));

            let begin = current_timestamp();
            while let Ok(_n) = receiver.recv() {}
            let end = current_timestamp();

            println!("{}", end.checked_sub(begin).unwrap().as_nanos());
        });

        let mut bs = Vec::with_capacity(1024 * 8);
        for _n in 0..bs.capacity() {
            bs.push('a' as u8);
        }
        let s = String::from_utf8(bs).unwrap();

        let send_thread_handle = spawn("send_thread", move || {
            for _n in 0..100 * 10000 {
                sender.send(s.clone()).unwrap();
            }

            std::thread::sleep(Duration::from_secs(10));
            for _n in 0..cap {
                sender.send("".to_string()).unwrap();
            }
        });

        send_thread_handle.join().unwrap();
        recv_thread_handle.join().unwrap();
        println!("finish");
        std::thread::park();
    }
}
