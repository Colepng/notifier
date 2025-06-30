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

use bincode::config::Configuration;
use bincode::serde::decode_from_slice;
use notify_rust::Notification;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};

use notifier::NotificationWrapper as NotificationConfig;
use tokio::sync::broadcast::{self, Receiver, Sender};

use std::io;

#[tokio::main]
async fn main() -> io::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:5200").await?;

    let (tx, rx) = broadcast::channel::<(Notification, u8)>(16);

    tokio::spawn(async move {
        print_notifications(rx).await;
    });

    let mut client_id = 0;

    loop {
        let (socket, addr) = listener.accept().await?;
        println!("new client at {addr:?}");

        let tx2 = tx.clone();
        let rx2 = tx2.subscribe();
        tokio::spawn(async move {
            process_socket(socket, tx2, rx2, client_id).await.unwrap();
        });
        client_id += 1;
    }
}

async fn print_notifications(mut rx: Receiver<(Notification, u8)>) {
    loop {
        let notification = rx.recv().await.unwrap();

        println!("client_id: {} \n {:#?}", notification.1, notification.0);
    }
}

async fn process_socket(
    stream: TcpStream,
    tx: Sender<(Notification, u8)>,
    rx: Receiver<(Notification, u8)>,
    client_id: u8,
) -> io::Result<()> {
    let (reader, writer) = stream.into_split();

    let writer_handle = tokio::spawn(async move { handle_writing(writer, rx, client_id).await });

    let reader_handle = tokio::spawn(async move { handle_reading(reader, tx, client_id).await });

    let _ = writer_handle.await.unwrap();
    let _ = reader_handle.await.unwrap();

    Ok(())
}

async fn handle_writing(
    mut writer: OwnedWriteHalf,
    mut rx: Receiver<(Notification, u8)>,
    client_id: u8,
) -> io::Result<()> {
    let config = bincode::config::standard();

    loop {
        let notifcation = rx.recv().await.unwrap();
        if notifcation.1 != client_id {
            let notification_wrapper: NotificationConfig = notifcation.0.into();
            let bytes = bincode::serde::encode_to_vec(notification_wrapper, config).unwrap();
            writer.write_u64(bytes.len() as u64).await?;
            writer.write_all(&bytes).await?;
        }
    }
}

async fn handle_reading(
    mut reader: OwnedReadHalf,
    tx: Sender<(Notification, u8)>,
    client_id: u8,
) -> io::Result<()> {
    let config = bincode::config::standard();

    loop {
        let size = reader.read_u64().await?;
        let mut buf = vec![0u8; usize::try_from(size).unwrap()];
        reader.read_exact(&mut buf).await?;

        let (notification, _) =
            decode_from_slice::<NotificationConfig, Configuration>(&buf, config).unwrap();

        let notification = Notification::new().summary(&notification.text).finalize();

        tx.send((notification, client_id)).unwrap();
    }
}
