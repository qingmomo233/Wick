# Wick

跨平台无线设备电量监控桌面应用：实时显示蓝牙 / 星闪 / 2.4G 接收器连接的外设电量。Tauri v2 + Rust + React 18 + TypeScript。

## 功能

- 蓝牙设备电量：Windows 用 SetupAPI 读 PnP 电池属性，Linux 用 BlueZ D-Bus（回退 UPower）
- 2.4G / 星闪接收器：罗技 Unifying / Bolt / Lightspeed（HID++ 协议），ATK / VXE / VGN 家族
- 只显示真实在连的设备；已配对未连接、读不出电量的链路不显示
- 读到过电量的设备永久保留，睡眠断开时在同一张卡片显示离线并保留最后读数
- 设备资料库 `devices/*.json`：把同一设备的不同链路（接收器名 / 蓝牙名）归并为一张卡片，切换连接方式时卡片自动换徽标
- 卡片右下角红色按钮移除设备；长按卡片拖拽排序，新设备置顶；底部按钮立即重新检测
- 低电量（≤20%）系统通知，一小时去重；系统托盘常驻；窗口置顶（图钉按钮）
- 设置：开机自启、关闭行为（托盘/退出）、检测间隔（2-60 秒）、背景不透明度

## 协议支持

| 传输 | 来源 | 状态 |
|---|---|---|
| 蓝牙 | Windows DEVPKEY / BlueZ Battery1 | 支持 |
| 2.4G | 罗技 HID++（1.0 寄存器 + 2.0 feature） | 支持 |
| 2.4G / 星闪 | ATK 家族协议（VID 0x373B） | 支持 |
| 2.4G | ATK 家族另一批次 VID 0x3554（如 MCHOSE 接收器，固件锁写） | 无读数不显示 |
| 未知设备 | 只读启发式探测（双轮一致性校验，绝不写入） | 读到稳定电量才显示 |

## 环境要求

- Node.js ≥ 18
- Rust（MSVC 工具链）+ VS C++ 生成工具
- Windows 10/11 需要 WebView2 运行时；Linux 需要 webkit2gtk-4.1 等依赖（见下）

## 开发运行

```bash
npm install
npm run tauri dev
```

## 打包

```bash
npm run tauri build
```

产物位置：

- Windows：`src-tauri/target/release/bundle/nsis/Wick_x.y.z_x64-setup.exe`（NSIS 安装包）与 `bundle/msi/*.msi`
- Linux：`bundle/deb/*.deb` 与 `bundle/appimage/*.AppImage`

单文件可执行程序在 `src-tauri/target/release/wick.exe`。

## Linux 依赖与 udev 权限

```bash
# Ubuntu / Debian
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev pkg-config libudev-dev

# Arch
sudo pacman -S --needed webkit2gtk-4.1 base-devel curl wget file xorg-server-xdo \
  openssl libayatana-appindicator librsvg udev pkgconf
```

读取 HID 接收器需要 udev 规则（否则只显示蓝牙设备）。创建 `/etc/udev/rules.d/70-wick-hidraw.rules`：

```udev
SUBSYSTEM=="hidraw", ATTRS{idVendor}=="046d", MODE="0666"
SUBSYSTEM=="hidraw", ATTRS{idVendor}=="373b", MODE="0666"
```

然后 `sudo udevadm control --reload-rules && sudo udevadm trigger`，重新插拔接收器。

## 数据存储

配置目录：Windows `%APPDATA%\com.wick.battery`，Linux `~/.config/com.wick.battery`

- `settings.json`：用户设置
- `known_devices.json`：已添加设备库（删除即清空设备列表）
- `devices/*.json`：设备资料库。文件名即设备展示名，字段：
  - `kind`：mouse / keyboard / headset / gamepad / speaker / phone / other
  - `aliases`：该设备的其他名称（蓝牙广播名等），用于归并不同链路
  - `vendor_ids`：专属接收器 `"vid:pid"`（小写十六进制），接收器在场即认为设备走 2.4G

资料库可自行编辑，软件内点击刷新按钮即生效，无需重启。首次启动自动生成内置设备与 README。

## 新增协议支持

在 `src-tauri/src/providers/hid/registry.rs` 登记 VID/PID，实现一个返回 `DeviceBattery` 的解析器模块即可。排查工具：

```bash
cargo run --example list_hid        # 列出全部 HID 设备
cargo run --example dump_devices    # 采集管线原始输出
cargo run --example dump_bt         # 蓝牙 PnP 节点原始数据
cargo run --example dump_bt_radio   # 蓝牙无线电层连接状态
cargo run --example dump_raw        # 接收器被动监听 + feature 扫描
```

## 许可

MIT
