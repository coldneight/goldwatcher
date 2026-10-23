//! System tray integration.
//!
//! Linux uses `tray-icon`'s GTK/AppIndicator backend on a dedicated thread so
//! the winit event loop stays responsive. Windows uses the native Win32 tray
//! implementation directly on the main thread. In both cases failures are
//! non-fatal and the main window keeps working.

use std::sync::mpsc::Receiver;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    Show,
    Refresh,
    Quit,
}

#[cfg(target_os = "linux")]
mod imp {
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::thread;
    use std::time::Duration;

    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, TrayIconBuilder};

    use super::TrayCommand;

    pub struct TrayHandle {
        pub commands: Receiver<TrayCommand>,
    }

    pub fn spawn(rgba: Vec<u8>, width: u32, height: u32) -> Option<TrayHandle> {
        let (ready_tx, ready_rx) = mpsc::channel::<bool>();
        let (command_tx, command_rx) = mpsc::channel::<TrayCommand>();
        thread::Builder::new()
            .name("goldwatcher-tray".to_owned())
            .spawn(move || tray_thread(rgba, width, height, ready_tx, command_tx))
            .ok()?;
        match ready_rx.recv_timeout(Duration::from_secs(4)) {
            Ok(true) => Some(TrayHandle {
                commands: command_rx,
            }),
            _ => None,
        }
    }

    fn tray_thread(
        rgba: Vec<u8>,
        width: u32,
        height: u32,
        ready_tx: Sender<bool>,
        command_tx: Sender<TrayCommand>,
    ) {
        if gtk::init().is_err() {
            let _ = ready_tx.send(false);
            return;
        }

        let menu = Menu::new();
        let show = MenuItem::with_id("show", "显示 GoldWatcher", true, None);
        let refresh = MenuItem::with_id("refresh", "立即刷新行情", true, None);
        let separator = PredefinedMenuItem::separator();
        let quit = MenuItem::with_id("quit", "退出", true, None);
        let _ = menu.append(&show);
        let _ = menu.append(&refresh);
        let _ = menu.append(&separator);
        let _ = menu.append(&quit);

        let Ok(icon) = Icon::from_rgba(rgba, width, height) else {
            let _ = ready_tx.send(false);
            return;
        };

        let Ok(tray) = TrayIconBuilder::new()
            .with_tooltip("金瞳 GoldWatcher · 全球黄金监控")
            .with_menu(Box::new(menu))
            .with_icon(icon)
            .build()
        else {
            let _ = ready_tx.send(false);
            return;
        };

        let (menu_tx, menu_rx) = mpsc::channel::<String>();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let _ = menu_tx.send(event.id.0.clone());
        }));

        let _ = gtk::glib::timeout_add_local(Duration::from_millis(120), move || {
            while let Ok(id) = menu_rx.try_recv() {
                let command = match id.as_str() {
                    "show" => Some(TrayCommand::Show),
                    "refresh" => Some(TrayCommand::Refresh),
                    "quit" => Some(TrayCommand::Quit),
                    _ => None,
                };
                if let Some(command) = command {
                    let _ = command_tx.send(command);
                }
            }
            gtk::glib::ControlFlow::Continue
        });

        let _ = ready_tx.send(true);
        gtk::main();
        drop(tray);
    }

    pub fn poll(handle: &TrayHandle) -> Vec<TrayCommand> {
        let mut out = Vec::new();
        while let Ok(command) = handle.commands.try_recv() {
            out.push(command);
        }
        out
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

    use super::TrayCommand;

    pub struct TrayHandle {
        _tray: TrayIcon,
    }

    pub fn spawn(rgba: Vec<u8>, width: u32, height: u32) -> Option<TrayHandle> {
        let menu = Menu::new();
        let show = MenuItem::with_id("show", "显示 GoldWatcher", true, None);
        let refresh = MenuItem::with_id("refresh", "立即刷新行情", true, None);
        let separator = PredefinedMenuItem::separator();
        let quit = MenuItem::with_id("quit", "退出", true, None);
        let _ = menu.append(&show);
        let _ = menu.append(&refresh);
        let _ = menu.append(&separator);
        let _ = menu.append(&quit);

        let icon = Icon::from_rgba(rgba, width, height).ok()?;
        let tray = TrayIconBuilder::new()
            .with_tooltip("金瞳 GoldWatcher · 全球黄金监控")
            .with_menu(Box::new(menu))
            .with_icon(icon)
            .build()
            .ok()?;
        Some(TrayHandle { _tray: tray })
    }

    pub fn poll(_handle: &TrayHandle) -> Vec<TrayCommand> {
        let mut out = Vec::new();
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            match event.id.0.as_str() {
                "show" => out.push(TrayCommand::Show),
                "refresh" => out.push(TrayCommand::Refresh),
                "quit" => out.push(TrayCommand::Quit),
                _ => {}
            }
        }
        out
    }
}

pub use imp::{poll, spawn, TrayHandle};

/// The receiver type is shared with the app through the platform handle.
#[allow(dead_code)]
pub type CommandReceiver = Receiver<TrayCommand>;
