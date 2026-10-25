# Wick

> 实时展示本机所有无线设备（**蓝牙 / 星闪 NearLink / 2.4G 接收器**）电量的桌面应用 —— 简约、优雅、原生。

![platform](https://img.shields.io/badge/platform-Windows%2010%2F11%20%7C%20Linux%20(X11%2FWayland)-blue) ![stack](https://img.shields.io/badge/Tauri%20v2-Rust%20%2B%20React%2018%20%2B%20TS-ff8800)

## 功能

- **蓝牙设备**电量：Windows 走原生 SetupAPI 读 PnP 电池属性（毫秒级），Linux 走 BlueZ D-Bus（回退 UPower）
- **2.4G 接收器**：罗技 Unifying / Bolt / Lightspeed（HID++ 协议），ATK / VXE / VGN（含星闪型号），Razer（HyperSpeed），SteelSeries（Arctis / Nova / Aerox / Rival），Corsair（Void / bragi），HyperX（Cloud 系列），8BitDo（手柄，只读监听）
- **星闪（NearLink）接收器**：以 USB HID 形式呈现，与 2.4G 统一处理；**UI 中与蓝牙同归「蓝牙」类**
- **只显示真实在连的无线设备**：已配对但未连接的（手机、收纳盒中的耳机）、读不出电量的链路一律不显示；有线键鼠、内置 I2C 触摸板同样排除
- **检测到即保留**：读到过电量的设备永久保留（重启也在）；键鼠睡眠断开时同一张卡片显示「离线」并保留最后读数
- **手动移除**：卡片右下角红色 ✕；被移除的设备从系统消失后重连会自动恢复
- **手动刷新**：底部按钮立即重新检测全部设备（含被移除的），与定时轮询互斥执行
- **产品名映射**：接收器的 HID 名是通用 dongle 名，Wick 按 VID/PID 显示实际设备名（如 373b:11fe → "ATK A9 Ultra Max 2.0 Tri-Mode"）
- 低电量（≤20%）系统通知，仅针对在线设备，同一设备 1 小时内不重复
- **设置面板**（标题栏齿轮）：开机自启（默认关闭）、关闭按钮行为（最小化到托盘 / 退出程序）、检测间隔（2..=60 秒）、背景不透明度
- 常驻系统托盘；默认 5 秒轮询，变化即推送；设备省电深睡期间保留最后一次成功读数

## 支持范围声明（请务必阅读）

**2.4G / 星闪接收器的电量读取没有业界统一标准**，全部为厂商私有 HID 协议。Wick 采用「已知协议注册表 + 通用只读探测」策略：

| 传输 | 协议来源 | 已验证设备 | 状态 |
|---|---|---|---|
| 蓝牙 | Windows DEVPKEY 电池属性（SetupAPI 原生） / BlueZ `Battery1` | 支持标准电量上报的耳机、键盘、鼠标等 | √ |
| 2.4G | 罗技 HID++（自实现，覆盖 1.0 寄存器与 2.0 `0x1000`/`0x1004`） | Unifying / Bolt / Lightspeed 接收器下的大部分键鼠 | √ |
| 2.4G / 星闪 | ATK 厂商协议（参考 atk-tool / libatk-rs，VID `0x373B`） | ATK A9 Ultra Max 2.0 Tri-Mode（`373b:11fe`，真机实测）、ATK A9 Plus（`373b:1115`）、Nearlink Mouse Dongle（`373b:10c9`） | √ |
| 2.4G | ATK 家族（Compx 另一 VID `0x3554`） | `3554:fa09` 真机实测：全部写入长度被固件拒绝（锁写），无法查询 | × |
| 2.4G | Razer 主协议（90 字节 feature report，参考 HaloBattery / OpenRazer，VID `0x1532`） | Viper / DeathAdder / Basilisk / Naga / Pro Click 等无线鼠标、BlackWidow / DeathStalker 无线键盘的全系接收器 PID | √ |
| 2.4G | Razer 变体：BlackShark V2 Pro 2020（feature report `0xFF`）、2023 与 Barracuda Pro（"PA" 输出报告） | 协议字节来自真机抓包与直读验证（HaloBattery） | √ |
| 2.4G | SteelSeries `0xB0` 状态交换（Nova 系列，参考 HaloBattery / HeadsetControl / Linux `hid-steelseries-arctis`） | Arctis Nova 7 / 5 / 3P / 7+ 全系接收器、GameBuds | √ |
| 2.4G | SteelSeries 经典交换（`06 12` / `06 18` / `00 20` / `06 b0` 等，带回显校验） | Arctis 1 / 7 / 9 / Pro Wireless / Nova Pro 底座 | √ |
| 2.4G | SteelSeries 鼠标（`00 aa 01` Rival 3 族 / `00 d2` Aerox 3·5·9 族） | Rival 3 Wireless、Aerox 3 / 5 / 9 Wireless | √ |
| 2.4G | Corsair：经典 Void（`c9 64`）、Void v2 握手（`0x0F`）、bragi/SlipStream get-property | Void / Void Pro / HS 系列、Void v2 / Virtuoso Max / HS80 Max、Dark Core RGB Pro SE / Virtuoso SE·XT | √ |
| 2.4G | HyperX Cloud 系列各型号私有交换 | Cloud II / II Core / III / III S / Alpha / Alpha 2 Wireless、Cloud Flight（电压折算） | √ |
| 蓝牙 | 8BitDo 增强输入报告（只读监听，绝不发送切换命令） | Pro 2 / Pro 3 / SN30 Pro / SF30 Pro（D-input 模式，Steam / 游戏使用时才有电量可读） | √ |
| 未知 VID/PID | 通用只读启发式（`get_feature_report` 双轮一致性校验），USB 上需无标准键盘接口 | — | 读到稳定电量才显示 |

读不出电量的设备不显示、不伪造数据；2.4G / 星闪接收器的电量协议全部为厂商私有，**没有业界统一标准**，Wick 采用「已知协议注册表 + 通用只读探测」策略。

## 安装与构建

### 依赖

**Windows 10/11 (x64)**

- [Node.js](https://nodejs.org) ≥ 18
- [Rust](https://rustup.rs)（MSVC 工具链）+ Visual Studio C++ 生成工具
- WebView2 运行时（Win11 自带，Win10 一般已随 Edge 安装）

```powershell
npm install
npm run tauri dev    # 开发
npm run tauri build  # 产出安装包（NSIS / MSI）
```

**Linux（Arch / Ubuntu / Debian，X11 + Wayland）**

```bash
# Ubuntu / Debian
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev \
  pkg-config libudev-dev

# Arch
sudo pacman -S --needed webkit2gtk-4.1 base-devel curl wget file xorg-server-xdo \
  openssl libayatana-appindicator librsvg udev pkgconf

npm install
npm run tauri dev
npm run tauri build  # 产出 .deb / .AppImage
```

> `libudev-dev` 是 `hidapi` 编译所需；`hidapi` crate 自带 C 源码编译，无需系统 hidapi 库。

### Linux udev 权限（重要）

Linux 下读取 HID 接收器需要 `/dev/hidraw*` 访问权限。**不配置 udev 规则时，HID 设备将无法读取**（蓝牙设备不受影响），Wick 会对相关设备弹出「协议探测失败」提示。

创建 `/etc/udev/rules.d/70-wick-hidraw.rules`：

```udev
# Wick：允许普通用户读取无线接收器的 hidraw 接口
# 罗技
SUBSYSTEM=="hidraw", ATTRS{idVendor}=="046d", MODE="0666"
# ATK / VXE / VGN（含星闪接收器）
SUBSYSTEM=="hidraw", ATTRS{idVendor}=="373b", MODE="0666"
# Razer
SUBSYSTEM=="hidraw", ATTRS{idVendor}=="1532", MODE="0666"
# SteelSeries
SUBSYSTEM=="hidraw", ATTRS{idVendor}=="1038", MODE="0666"
# Corsair
SUBSYSTEM=="hidraw", ATTRS{idVendor}=="1b1c", MODE="0666"
# HyperX（HP 与 Kingston 批次）
SUBSYSTEM=="hidraw", ATTRS{idVendor}=="03f0", MODE="0666"
SUBSYSTEM=="hidraw", ATTRS{idVendor}=="0951", MODE="0666"
# 8BitDo（蓝牙手柄一般无需，接收器场景预留）
SUBSYSTEM=="hidraw", ATTRS{idVendor}=="2dc8", MODE="0666"
# 如需放宽到全部 HID 设备（便于通用探测未知接收器），取消下一行注释
# SUBSYSTEM=="hidraw", MODE="0666"
```

然后重新加载：

```bash
sudo udevadm control --reload-rules
sudo udevadm trigger
# 重新插拔接收器
```

> 另注：较新内核的 `hid-logitech-hidpp` 驱动会把部分罗技接收器设备暴露为
> `/sys/class/power_supply/` 下的电源设备。Wick 目前未使用该路径
> （D-Bus / hidraw 已覆盖），仅作排障参考：`upower -d`。

### 数据存储

- 设置：`%APPDATA%\com.wick.battery\settings.json`（Linux：`~/.config/com.wick.battery/`）
- 已添加设备库：同目录 `known_devices.json`（删除该文件可清空设备列表）
- **设备资料库**：同目录 `devices/*.json`——每个文件描述一台设备（文件名即展示名），
  字段 `kind`（类型）、`aliases`（该设备的其他名称，用于把 2.4G 接收器名与蓝牙广播名
  归并为同一张卡片）、`vendor_ids`（专属接收器 `"vid:pid"`，接收器在场即认为设备走 2.4G）。
  首次启动自动生成内置资料与 README，可自行搜索设备信息后添加/修改，点击软件内刷新即生效
- 旧版 Pulse（`com.pulse.battery`）的设置会在首次启动时自动迁移

## 帮助扩展支持

遇到「协议未适配」的设备？欢迎提交信息帮助新增协议：

1. 记录设备 VID/PID：
   - Windows：设备管理器 → 详细信息 → 硬件 Id（如 `USB\VID_373B&PID_10C9`）
   - Linux：`lsusb`，或 `cat /sys/class/hidraw/hidraw*/device/uevent`
2. （可选）采集探测日志：开发模式运行 `npm run tauri dev`，终端会输出
   `[wick]` 前缀的探测日志（含 open 失败、超时、无应答等信息）；
   已登记品牌可用 `cargo run --example probe_vendor` 单独验证某一台设备的
   电量协议是否读到（Razer / SteelSeries / Corsair / HyperX / 8BitDo）。
3. 提交 Issue 附上：VID/PID、`lsusb -v` 输出、设备型号，以及（如果你熟悉
   HID 抓包）wireshark USBPcap 的厂商报文样本。

新增协议只需两步：在 `src-tauri/src/providers/hid/registry.rs` 登记 VID/PID，
实现一个返回 `DeviceBattery` 的解析器模块。

## 架构

```
src-tauri/src/
├── model.rs                  # DeviceBattery 统一模型（protocol_status 是关键）
├── known.rs                  # 已添加设备持久化（known_devices.json）
├── commands.rs               # Tauri 命令（get_devices / refresh_now / remove_device / 设置）
├── collector.rs              # 定时采集（采集锁互斥）+ 已添加库合并 + 离线合成 + 低电量通知
├── settings.rs               # 用户设置（含旧版 Pulse 配置迁移）
└── providers/
    ├── mod.rs                # 蓝牙与 HID 结果合并去重
    ├── bluetooth/            # windows.rs（SetupAPI DEVPKEY）/ linux.rs（BlueZ → UPower 回退）
    └── hid/
        ├── mod.rs            # hidapi 枚举、分组、超时调度、有线过滤（每协议独立超时）
        ├── registry.rs       # VID/PID → 解析器注册表 + 产品名映射
        ├── hidpp_logi.rs     # 罗技 HID++（1.0 寄存器 + 2.0 feature 0x1000/0x1004）
        ├── atk_vxe.rs        # ATK/VXE/VGN（含星闪接收器，VID 0x373B / 0x3554）
        ├── razer.rs          # Razer（主协议 + BlackShark/Barracuda PA 变体）
        ├── steelseries.rs    # SteelSeries（Nova / 经典 Arctis / Aerox·Rival 鼠标）
        ├── corsair.rs        # Corsair（经典 Void / Void v2 / bragi）
        ├── hyperx.rs         # HyperX（Cloud 系列耳机）
        ├── eightbitdo.rs     # 8BitDo 手柄（只读监听增强报告）
        └── generic.rs        # 未知设备只读启发式（双轮一致性，绝不写入）
```

前端：React 18 + TypeScript + 纯 CSS（CSS 变量），暗色玻璃拟态，传输徽标
（蓝牙=蓝 / 星闪=紫 / 2.4G=琥珀），`--` 虚线环表示协议未适配，
压暗卡片 + 「离线」徽标表示设备休眠/未连接。

## 许可

MIT
