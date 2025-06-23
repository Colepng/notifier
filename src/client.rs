#![warn(
    clippy::pedantic,
    clippy::nursery,
    clippy::perf,
    clippy::style,
    clippy::todo
)]
#![deny(
    clippy::suspicious,
    clippy::correctness,
    clippy::complexity,
    clippy::missing_const_for_fn,
    unsafe_code
)]

use bincode::{config::Configuration, serde::decode_from_slice};
use notifier::NotificationWrapper;
use notify_rust::Notification;
use std::io;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{tcp::{OwnedReadHalf, OwnedWriteHalf}, TcpSocket},
};

#[tokio::main]
async fn main() -> io::Result<()> {
    let socket = TcpSocket::new_v4()?;
    let stream = socket.connect("127.0.0.1:8080".parse().unwrap()).await?;
    let (read, write) = stream.into_split();

    let notfications_handler = tokio::spawn(async move {
        get_notifications_and_display(read).await;
    });

    let notification = NotificationWrapper::new("text for notifcation".into());
    send_notfication(write, notification).await?;

    notfications_handler.await?;

    Ok(())
}

async fn get_notifications_and_display(mut reader: OwnedReadHalf) {
    loop {
        let notification = read_notification(&mut reader).await.unwrap();
        notification.show_async().await.unwrap();
    }
}

async fn send_notfication(mut stream: OwnedWriteHalf, notification: NotificationWrapper) -> io::Result<()> {
    let config = bincode::config::standard();

    let vec = bincode::serde::encode_to_vec(notification, config).unwrap();

    stream.write_u64(vec.len() as u64).await?;
    stream.write_all(&vec).await?;

    Ok(())
}

async fn read_notification(stream: &mut OwnedReadHalf) -> io::Result<Notification> {
    let size = stream.read_u64().await?;

    let mut buf = vec![0u8; usize::try_from(size).unwrap()];
    stream.read_exact(&mut buf).await?;

    let config = bincode::config::standard();

    let (notifcation, _) = decode_from_slice::<NotificationWrapper, Configuration>(&buf, config).unwrap();

    let notify = notify_rust::Notification::new().summary(&notifcation.text).finalize();

    Ok(notify)
}
