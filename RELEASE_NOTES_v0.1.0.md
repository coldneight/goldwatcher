# 金瞳 GoldWatcher v0.1.0

首个公开版本：一个用 Rust 编写的全球黄金市场实时监控与提醒桌面工具。

## 下载

| 平台 | 文件 | 说明 |
|---|---|---|
| Linux x86_64 | `goldwatcher-v0.1.0-linux-x86_64.tar.gz` | 解压后运行 `./gold-watcher`，或执行包内 `./install.sh` 安装图标和桌面入口 |
| Windows x86_64 | `goldwatcher-v0.1.0-windows-x86_64.zip` | 解压后双击 `GoldWatcher.exe`；可用 `install.ps1` 创建快捷方式 |

SHA-256 校验值见 `SHA256SUMS`。

## 功能

- 伦敦金 XAU/USD、COMEX GC、上海金 Au99.99、黄金 T+D、沪金主力
- Gold-API 秒级现货与新浪多市场行情并发抓取，按源时间戳合并
- 15 / 60 / 120 / 300 秒刷新间隔，默认 15 秒
- 价格、日涨跌幅、COMEX 基差、上海金溢价、USD/CNY 告警
- 伦敦 / 纽约 / 上海日盘夜盘 / 东京日盘夜盘 / 印度 / 迪拜开收盘提醒
- 桌面通知、窗口闪烁、重复冷却、触发记录
- 今日分钟走势图、昨收虚线、十字光标
- Linux / Windows 系统托盘、开机自启、本地持久化
- 深色金色界面、应用图标、内置中文字体

## 构建与验证

- Linux 与 Windows 由 GitHub Actions 分别在对应平台构建
- `cargo test`：16 项测试全部通过
- `cargo clippy --all-targets -- -D warnings`：通过
- 首个版本使用 Rust 1.97.1 / edition 2021

## 数据说明

行情来自免费公开接口，可能延迟、限流或中断；交易时段为北京时间近似值，不处理节假日和临时调整。本项目仅供信息参考，不构成投资建议。
