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
use eframe::egui::{
    self,
    ahash::{HashMap, HashMapExt},
    mutex::Mutex,
};
use notifier::NotificationWrapper;
use notify_rust::Notification;
use serde::{Deserialize, Serialize};
use std::{
    io,
    net::{Ipv4Addr, SocketAddrV4},
    path::PathBuf,
    str::FromStr,
    sync::Arc,
};
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt},
    net::{
        TcpSocket,
        tcp::{OwnedReadHalf, OwnedWriteHalf},
    },
    runtime::Runtime,
    sync::{
        broadcast,
        mpsc::{self, Receiver, Sender},
    },
};
use tray_icon::{
    Icon, TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuItem},
};

use directories::ProjectDirs;

#[derive(Deserialize, Serialize)]
struct Config {
    ip: String,
    port: u16,
    quick_msgs: Vec<(String, String)>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            ip: "127.0.0.1".to_string(),
            port: 5200,
            quick_msgs: [
                ("send love".into(), "I love you, <3".into()),
                ("Send hug".into(), "Big hugs".into()),
                ("check phone".into(), "check your phone".into()),
            ]
            .to_vec(),
        }
    }
}

fn get_path() -> PathBuf {
    ProjectDirs::from("ca", "cole corp", "notifier")
        .map(|project_dirs| {
            let config_dir = project_dirs.config_dir();
            PathBuf::from(config_dir).join("config.toml")
        })
        .expect("was not able to find a location for config file, add support for this later")
}

async fn get_config_file() -> io::Result<Config> {
    let path = get_path();

    let exists = path.exists();

    if exists {
        let string = fs::read_to_string(path).await?;

        Ok(toml::from_str::<Config>(&string).expect("could not understand file"))
    } else {
        Ok(Config::default())
    }
}

fn get_config_file_blocking() -> Config {
    let path = get_path();

    let exists = path.exists();

    if exists {
        let string = std::fs::read_to_string(path).expect("failed to read file");

        toml::from_str::<Config>(&string).expect("could not understand file")
    } else {
        Config::default()
    }
}

async fn save_config_file(config: Config) -> io::Result<()> {
    let path = get_path();

    let exists = path.exists();

    if !exists {
        let mut dir_path = path.clone();
        dir_path.pop();
        fs::create_dir_all(dir_path)
            .await
            .expect("failed to make dir");
    }

    let string = toml::to_string_pretty(&config).expect("failed to parse config");

    fs::write(path, string).await?;

    Ok(())
}

enum Action {
    SendNotfication(String),
}

struct Gui {
    tx: Sender<Action>,
    text: String,
}

impl eframe::App for Gui {
    fn update(&mut self, ctx: &eframe::egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("enter message");

            let _response = ui.add(egui::TextEdit::singleline(&mut self.text));

            if ui.add(egui::Button::new("send")).clicked() && !self.text.is_empty() {
                self.tx
                    .blocking_send(Action::SendNotfication(self.text.clone()))
                    .unwrap();
                self.text.clear();
            }
        });
    }
}

fn main() {
    let runtime = Runtime::new().expect("Unable to spawn runtime");

    let _enter = runtime.enter();

    let (tx, rx) = mpsc::channel::<Action>(16);
    let (tx_exit, rx_exit) = broadcast::channel(1);

    let tx_cloned = tx.clone();
    let handle = std::thread::spawn(move || {
        runtime.block_on(main_late(tx_cloned, rx, rx_exit)).unwrap();
    });

    let gui = Gui {
        tx: tx.clone(),
        text: String::new(),
    };

    let config = get_config_file_blocking();

    let menu_options: Arc<Mutex<HashMap<String, String>>> = Arc::new(Mutex::new(HashMap::new()));

    let menu_options_tray = menu_options.clone();
    std::thread::spawn(move || {
        gtk::init().unwrap();

        let mut icon_data: Vec<u8> = Vec::with_capacity(16 * 16 * 4);
        for _ in 0..256 {
            // all red
            icon_data.extend_from_slice(&[255, 0, 0, 255]);
        }

        let menu = Menu::new();
        for msg in config.quick_msgs {
            let menu_item = MenuItem::new(msg.0, true, None);
            menu_options_tray
                .lock()
                .insert(menu_item.id().0.clone(), msg.1);
            menu.append(&menu_item).expect("failed to add menu item");
        }

        let icon = Icon::from_rgba(icon_data, 16, 16).unwrap();

        let _tray_icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_icon(icon)
            .build()
            .unwrap();

        gtk::main();
    });

    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "notifier",
        options,
        Box::new(|_cc| {
            MenuEvent::set_event_handler(Some(move |event| {
                let MenuEvent { ref id } = event;
                if let Some(text) = menu_options.lock().get(&id.0) {
                    tx.blocking_send(Action::SendNotfication(text.to_string()))
                        .expect("failed to send notification");
                }
            }));
            Ok(Box::new(gui))
        }),
    )
    .unwrap();

    tx_exit.send(()).unwrap();
    handle.join().unwrap();
}

async fn main_late(
    _tx: Sender<Action>,
    mut rx: Receiver<Action>,
    mut rx_exit: broadcast::Receiver<()>,
) -> io::Result<()> {
    let config = get_config_file().await?;
    let addr = SocketAddrV4::new(Ipv4Addr::from_str(&config.ip).unwrap(), config.port);

    let socket = TcpSocket::new_v4()?;
    let stream = socket.connect(std::net::SocketAddr::V4(addr)).await?;
    let (read, mut write) = stream.into_split();

    let send_handler = tokio::spawn(async move {
        loop {
            if let Some(action) = rx.recv().await {
                match action {
                    Action::SendNotfication(text) => {
                        let notification = NotificationWrapper::new(text);
                        send_notfication(&mut write, notification)
                            .await
                            .expect("failed to send notification");
                    }
                }
            }
        }
    });

    let display_abort = tokio::spawn(async move {
        get_notifications_and_display(read).await;
    })
    .abort_handle();

    if rx_exit.recv().await == Ok(()) {
        save_config_file(config)
            .await
            .expect("failed to save config");

        display_abort.abort();
        send_handler.abort();
    }

    Ok(())
}

async fn get_notifications_and_display(mut reader: OwnedReadHalf) {
    loop {
        let notification = read_notification(&mut reader).await.unwrap();
        notification.show_async().await.unwrap();
    }
}

async fn send_notfication(
    stream: &mut OwnedWriteHalf,
    notification: NotificationWrapper,
) -> io::Result<()> {
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

    let (notifcation, _) =
        decode_from_slice::<NotificationWrapper, Configuration>(&buf, config).unwrap();

    let notify = notify_rust::Notification::new()
        .summary(&notifcation.text)
        .finalize();

    Ok(notify)
}
