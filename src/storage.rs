//! Persistence and desktop integration helpers.

use std::fs;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;

use crate::model::{AppConfig, HistoryStore};

#[derive(Debug, Clone)]
pub struct Paths {
    pub config_file: PathBuf,
    pub history_file: PathBuf,
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
}

impl Paths {
    pub fn discover() -> Self {
        let pd = ProjectDirs::from("io.github", "GoldWatcher", "GoldWatcher");
        let (config_dir, data_dir) = match pd {
            Some(pd) => (pd.config_dir().to_path_buf(), pd.data_dir().to_path_buf()),
            None => {
                let base = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
                (
                    base.join("goldwatcher-config"),
                    base.join("goldwatcher-data"),
                )
            }
        };
        Self {
            config_file: config_dir.join("config.json"),
            history_file: data_dir.join("history.json"),
            config_dir,
            data_dir,
        }
    }

    pub fn ensure(&self) -> std::io::Result<()> {
        fs::create_dir_all(&self.config_dir)?;
        fs::create_dir_all(&self.data_dir)?;
        Ok(())
    }
}

pub fn load_config(paths: &Paths) -> AppConfig {
    let Ok(text) = fs::read_to_string(&paths.config_file) else {
        return AppConfig::default();
    };
    match serde_json::from_str::<AppConfig>(&text) {
        Ok(mut config) => {
            config.poll_secs = config.poll_secs.clamp(5, 600);
            config.events.truncate(100);
            config
        }
        Err(err) => {
            eprintln!("GoldWatcher: 配置文件解析失败，将使用默认设置：{err}");
            let backup = paths.config_dir.join(format!(
                "config.corrupt.{}.json",
                chrono::Local::now().format("%Y%m%d-%H%M%S")
            ));
            let _ = fs::rename(&paths.config_file, backup);
            AppConfig::default()
        }
    }
}

pub fn save_config(paths: &Paths, config: &AppConfig) -> Result<(), String> {
    paths
        .ensure()
        .map_err(|e| format!("创建配置目录失败：{e}"))?;
    atomic_write_json(&paths.config_file, config)
}

pub fn load_history(paths: &Paths) -> HistoryStore {
    let Ok(text) = fs::read_to_string(&paths.history_file) else {
        return HistoryStore::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

pub fn save_history(paths: &Paths, history: &HistoryStore) -> Result<(), String> {
    paths
        .ensure()
        .map_err(|e| format!("创建数据目录失败：{e}"))?;
    atomic_write_json(&paths.history_file, history)
}

fn atomic_write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let text = serde_json::to_vec_pretty(value).map_err(|e| format!("序列化失败：{e}"))?;
    let tmp = path.with_extension("json.tmp");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建目录失败：{e}"))?;
    }
    fs::write(&tmp, text).map_err(|e| format!("写入失败：{e}"))?;
    match fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(_) => {
            let _ = fs::remove_file(path);
            fs::rename(&tmp, path).map_err(|e| format!("保存失败：{e}"))
        }
    }
}

pub fn autostart_supported() -> bool {
    cfg!(any(target_os = "linux", target_os = "windows"))
}

#[cfg(target_os = "linux")]
pub fn set_autostart(enable: bool) -> Result<(), String> {
    let home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .ok_or_else(|| "无法定位用户配置目录".to_owned())?;
    let dir = home.join("autostart");
    let file = dir.join("gold-watcher.desktop");
    if !enable {
        if file.exists() {
            fs::remove_file(&file).map_err(|e| format!("移除开机自启失败：{e}"))?;
        }
        return Ok(());
    }
    fs::create_dir_all(&dir).map_err(|e| format!("创建 autostart 目录失败：{e}"))?;
    let exe = std::env::current_exe().map_err(|e| format!("获取程序路径失败：{e}"))?;
    let exec = exe
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let content = format!(
        "[Desktop Entry]\nType=Application\nVersion=1.0\nName=GoldWatcher\nName[zh_CN]=金瞳 GoldWatcher\nComment=全球黄金市场实时监控与告警\nExec=\"{exec}\"\nIcon=gold-watcher\nTerminal=false\nCategories=Office;Finance;\nX-GNOME-Autostart-enabled=true\n"
    );
    fs::write(&file, content).map_err(|e| format!("写入开机自启失败：{e}"))
}

#[cfg(target_os = "windows")]
pub fn set_autostart(enable: bool) -> Result<(), String> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey(r"Software\Microsoft\Windows\CurrentVersion\Run")
        .map_err(|e| format!("打开注册表启动项失败：{e}"))?;
    if enable {
        let exe = std::env::current_exe().map_err(|e| format!("获取程序路径失败：{e}"))?;
        let command = format!("\"{}\"", exe.display());
        key.set_value("GoldWatcher", &command)
            .map_err(|e| format!("写入注册表启动项失败：{e}"))
    } else {
        match key.delete_value("GoldWatcher") {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(format!("移除注册表启动项失败：{err}")),
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub fn set_autostart(_enable: bool) -> Result<(), String> {
    Err("当前平台请使用系统自带的登录项设置".to_owned())
}

#[cfg(target_os = "linux")]
pub fn autostart_enabled() -> bool {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .map(|dir| dir.join("autostart/gold-watcher.desktop").exists())
        .unwrap_or(false)
}

#[cfg(target_os = "windows")]
pub fn autostart_enabled() -> bool {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    hkcu.open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Run")
        .and_then(|key| key.get_value::<String, _>("GoldWatcher"))
        .is_ok()
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub fn autostart_enabled() -> bool {
    false
}

pub fn open_path(path: &Path) {
    #[cfg(target_os = "linux")]
    let program = "xdg-open";
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(target_os = "windows")]
    let program = "explorer";
    let _ = std::process::Command::new(program).arg(path).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{SessionAlert, SessionId};

    #[test]
    fn config_roundtrip_in_temp_dir() {
        let dir = std::env::temp_dir().join(format!("goldwatcher-test-{}", std::process::id()));
        let paths = Paths {
            config_file: dir.join("config.json"),
            history_file: dir.join("history.json"),
            config_dir: dir.clone(),
            data_dir: dir.clone(),
        };
        let config = AppConfig {
            poll_secs: 60,
            session_alerts: vec![SessionAlert::new(SessionId::NewYork, true, true)],
            ..Default::default()
        };
        save_config(&paths, &config).expect("save");
        let loaded = load_config(&paths);
        assert_eq!(loaded.poll_secs, 60);
        assert_eq!(loaded.session_alerts.len(), 1);
        assert_eq!(loaded.session_alerts[0].session, SessionId::NewYork);
        let _ = fs::remove_dir_all(dir);
    }
}
