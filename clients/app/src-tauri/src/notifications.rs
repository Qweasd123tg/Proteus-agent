//! System notifications of the desktop shell. WebKitGTK fixes notification
//! permissions when its web process starts, before the page can ask, so the
//! Linux shell sends them through the freedesktop D-Bus service itself.

use tauri::AppHandle;

#[derive(Default)]
pub(crate) struct Notifier {
    #[cfg(target_os = "linux")]
    center: tauri::async_runtime::Mutex<Option<linux::Center>>,
}

/// Shows a notification about a chat; clicking it focuses the window and
/// emits `proteus-notification-open` with that chat to the page.
#[tauri::command]
pub(crate) async fn notify(
    app: AppHandle,
    state: tauri::State<'_, Notifier>,
    title: String,
    body: String,
    session_dir: String,
) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        let mut center = state.center.lock().await;
        if center.is_none() {
            let connection = zbus::Connection::session()
                .await
                .map_err(|error| error.to_string())?;
            *center = Some(
                linux::Center::connect(&connection, move |session_dir| {
                    use tauri::Emitter;
                    let _ = crate::windows::focus(&app, "main");
                    let _ = app.emit_to("main", "proteus-notification-open", session_dir);
                })
                .await
                .map_err(|error| error.to_string())?,
            );
        }
        let center = center.as_ref().expect("notification center");
        center
            .show(&title, &body, &session_dir)
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (app, state, title, body, session_dir);
        Err("уведомления оболочки есть только в Linux".to_owned())
    }
}

#[cfg(target_os = "linux")]
pub(crate) mod linux {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

    use futures_util::StreamExt;

    #[zbus::proxy(
        interface = "org.freedesktop.Notifications",
        default_service = "org.freedesktop.Notifications",
        default_path = "/org/freedesktop/Notifications"
    )]
    trait Notifications {
        #[allow(clippy::too_many_arguments)]
        fn notify(
            &self,
            app_name: &str,
            replaces_id: u32,
            app_icon: &str,
            summary: &str,
            body: &str,
            actions: &[&str],
            hints: HashMap<&str, zbus::zvariant::Value<'_>>,
            expire_timeout: i32,
        ) -> zbus::Result<u32>;

        #[zbus(signal)]
        fn action_invoked(&self, id: u32, action_key: String) -> zbus::Result<()>;
    }

    /// One notification per chat: a newer one replaces the previous.
    #[derive(Default)]
    struct Chats {
        by_id: HashMap<u32, String>,
        by_chat: HashMap<String, u32>,
    }

    pub(crate) struct Center {
        proxy: NotificationsProxy<'static>,
        chats: Arc<Mutex<Chats>>,
    }

    impl Center {
        pub(crate) async fn connect(
            connection: &zbus::Connection,
            on_open: impl Fn(String) + Send + 'static,
        ) -> zbus::Result<Self> {
            let proxy = NotificationsProxy::new(connection).await?;
            let chats = Arc::new(Mutex::new(Chats::default()));
            let mut clicks = proxy.receive_action_invoked().await?;
            let opened = chats.clone();
            tauri::async_runtime::spawn(async move {
                while let Some(signal) = clicks.next().await {
                    let Ok(args) = signal.args() else { continue };
                    if args.action_key != "default" {
                        continue;
                    }
                    let chat = opened.lock().unwrap().by_id.get(&args.id).cloned();
                    if let Some(chat) = chat {
                        on_open(chat);
                    }
                }
            });
            Ok(Self { proxy, chats })
        }

        pub(crate) async fn show(&self, title: &str, body: &str, chat: &str) -> zbus::Result<u32> {
            let replaces = self
                .chats
                .lock()
                .unwrap()
                .by_chat
                .get(chat)
                .copied()
                .unwrap_or(0);
            let id = self
                .proxy
                .notify(
                    "Proteus",
                    replaces,
                    "",
                    title,
                    body,
                    &["default", "Открыть чат"],
                    HashMap::new(),
                    -1,
                )
                .await?;
            let mut chats = self.chats.lock().unwrap();
            chats.by_id.retain(|_, known| known != chat);
            chats.by_id.insert(id, chat.to_owned());
            chats.by_chat.insert(chat.to_owned(), id);
            Ok(id)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::{
            io::{BufRead, BufReader},
            process::{Command, Stdio},
            sync::mpsc,
            time::Duration,
        };

        #[derive(Default)]
        struct Daemon {
            calls: Arc<Mutex<Vec<(u32, String, String)>>>,
        }

        #[zbus::interface(name = "org.freedesktop.Notifications")]
        impl Daemon {
            #[allow(clippy::too_many_arguments)]
            fn notify(
                &self,
                _app_name: &str,
                replaces_id: u32,
                _app_icon: &str,
                summary: &str,
                body: &str,
                _actions: Vec<&str>,
                _hints: HashMap<&str, zbus::zvariant::Value<'_>>,
                _expire_timeout: i32,
            ) -> u32 {
                let mut calls = self.calls.lock().unwrap();
                calls.push((replaces_id, summary.to_owned(), body.to_owned()));
                if replaces_id == 0 {
                    calls.len() as u32
                } else {
                    replaces_id
                }
            }

            #[zbus(signal)]
            async fn action_invoked(
                emitter: &zbus::object_server::SignalEmitter<'_>,
                id: u32,
                action_key: &str,
            ) -> zbus::Result<()>;
        }

        #[test]
        fn notifications_replace_per_chat_and_clicks_open_the_chat() {
            let mut bus = Command::new("dbus-daemon")
                .args(["--session", "--nofork", "--nopidfile", "--print-address"])
                .stdout(Stdio::piped())
                .spawn()
                .expect("dbus-daemon");
            let mut address = String::new();
            BufReader::new(bus.stdout.take().unwrap())
                .read_line(&mut address)
                .unwrap();
            let result = tauri::async_runtime::block_on(async {
                let daemon = Daemon::default();
                let calls = daemon.calls.clone();
                let server = zbus::connection::Builder::address(address.trim())?
                    .name("org.freedesktop.Notifications")?
                    .serve_at("/org/freedesktop/Notifications", daemon)?
                    .build()
                    .await?;
                let client = zbus::connection::Builder::address(address.trim())?
                    .build()
                    .await?;
                let (opened, open) = mpsc::channel();
                let center =
                    Center::connect(&client, move |chat| opened.send(chat).unwrap()).await?;
                let first = center.show("Чат A", "Готово", "/a").await?;
                center.show("Чат B", "Ждёт ответа", "/b").await?;
                let again = center.show("Чат A", "Нужно подтверждение", "/a").await?;
                assert_eq!(again, first, "a chat keeps one notification");
                assert_eq!(
                    calls
                        .lock()
                        .unwrap()
                        .iter()
                        .map(|call| call.0)
                        .collect::<Vec<_>>(),
                    [0, 0, first]
                );
                let iface = server
                    .object_server()
                    .interface::<_, Daemon>("/org/freedesktop/Notifications")
                    .await?;
                Daemon::action_invoked(iface.signal_emitter(), first, "Открыть чат").await?;
                Daemon::action_invoked(iface.signal_emitter(), first, "default").await?;
                zbus::Result::Ok(open.recv_timeout(Duration::from_secs(5)))
            });
            bus.kill().unwrap();
            bus.wait().unwrap();
            assert_eq!(result.unwrap().unwrap(), "/a");
        }
    }
}
