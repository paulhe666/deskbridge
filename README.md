# Deskbridge

Deskbridge is a Deskflow-like keyboard, mouse, clipboard, and file bridge for
Windows and macOS systems.

Deskbridge was created mainly to work around the macOS support gaps and
compatibility issues encountered when using Deskflow-like workflows. The code
therefore puts extra attention on macOS input injection, keyboard mapping,
clipboard formats, wheel behavior, packaging, and permission handling.

The project is experimental but usable for daily testing. It was developed with
substantial assistance from OpenAI Codex for code generation, debugging,
packaging, documentation, and iterative user testing.

Because of macOS input compatibility limits, the mouse may still occasionally
stutter or pause. This is a known issue and will be improved in later versions.

## Features

- One binary with a bilingual GUI control panel
- Windows and macOS server/client modes
- Keyboard and mouse sharing in both Windows-to-macOS and macOS-to-Windows directions
- Shortcut-oriented modifier mapping for cross-platform control
- Caps Lock state emulation for remote macOS letter input
- macOS screenshot mapping through the Windows Print Screen key
- Tunable macOS wheel smoothing
- Bidirectional clipboard sync for text, bitmap images, and files
- GUI file drop zone on both systems
- Windows edge drop strip for Explorer file transfer
- macOS `.app`, `.pkg`, and Windows Inno Setup packaging support
- macOS `.icns` icon and Windows `.ico` installer/shortcut icon support

## Status

This repository supports these practical setups:

- Server: Windows
- Client: macOS
- Server: macOS
- Client: Windows
- Transport: TCP over LAN, hotspot LAN, or Tailscale

Direct drag-and-drop of a file across the screen edge into an arbitrary Finder or
Explorer window is not fully implemented. File transfer currently works through
copy/paste, the GUI drop zone, and the Windows edge drop strip. The macOS server
path is a beta feature: it is functional, but its pointer capture, modifier
mapping, and latency behavior are not yet as optimized as the Windows server
path and still need more real-device tuning.

## Install

Download the latest release from:

https://github.com/paulhe666/deskbridge/releases

### Windows

1. Download `Deskbridge-Setup-1.0.3.exe`.
2. Open the installer and follow the setup wizard.
3. Start `Deskbridge` from the Start menu.
4. If Windows Firewall asks for permission, allow Deskbridge on private
   networks.

### macOS

1. Download `Deskbridge-1.0.3.pkg`.
2. Open the package and install Deskbridge.
3. Open `System Settings -> Privacy & Security -> Accessibility`.
4. Add and enable `Deskbridge.app`.
5. Start `Deskbridge` from Applications.

## Quick Start

Launch `Deskbridge` directly.

Windows server, macOS client:

1. Set role to `Server`.
2. Keep bind as `0.0.0.0:24920`.
3. Set edge to `right` if the Mac is to the right of the Windows screen.
4. On macOS, set role to `Client` and server to `WINDOWS_IP:24920`.
5. Click `Start` on both systems.

macOS server, Windows client:

1. On macOS, set role to `Server`.
2. Keep bind as `0.0.0.0:24920`.
3. Set edge to `right` if the Windows screen is to the right of the Mac screen.
4. On Windows, set role to `Client` and server to `MAC_IP:24920`.
5. Click `Start` on both systems.

The GUI saves config to:

```text
~/.deskbridge/config.ini
```

The GUI supports Chinese and English. If Chinese text renders as boxes, ensure
the operating system has a CJK UI font such as Hiragino Sans GB, STHeiti,
Microsoft YaHei, SimHei, or Noto Sans CJK.

## Modifier Mapping

When macOS runs as the server and controls Windows, the app can map the physical
macOS Command, Control, and Option keys to Windows Ctrl, Win, Alt, or disabled.
Open `Settings` next to the Start/Stop button in the GUI to adjust these values.
Restart the service after saving mapping changes.

The default keeps compatibility with earlier builds:

- Command -> Ctrl
- Control -> Ctrl
- Option -> Alt

Command can be changed to Win if your workflow treats it more like the Windows
logo key.

## Uninstall

On Windows, uninstall Deskbridge from Settings or Control Panel.

On macOS, remove:

```text
/Applications/Deskbridge.app
~/.deskbridge
```

The macOS release package also includes `Uninstall Deskbridge.command` in the
app zip artifact.

## Developer Build

End users should use the release installers above. Developers can still build
packages from source:

macOS:

```bash
./scripts/package_macos.sh
```

Windows:

```powershell
.\scripts\package_windows.ps1
```

Windows packaging requires Rust, Visual Studio Build Tools with the Windows SDK,
and Inno Setup 6.

## macOS Permissions

The macOS app needs Accessibility permission to inject or capture keyboard and
mouse input. macOS server mode may also require Input Monitoring permission.

Open:

```text
System Settings -> Privacy & Security -> Accessibility
```

Add and enable `Deskbridge.app`, or the terminal app if running from terminal.

## Command Line

The GUI starts these same service commands internally.

Windows server:

```powershell
deskbridge.exe server --bind 0.0.0.0:24920 --edge right
```

macOS client:

```bash
deskbridge client --server WINDOWS_IP:24920
```

macOS server:

```bash
deskbridge server --bind 0.0.0.0:24920 --edge right
```

macOS server with Command mapped to Win:

```bash
DESKBRIDGE_MAC_COMMAND_MAPPING=win \
deskbridge server --bind 0.0.0.0:24920 --edge right
```

Windows client:

```powershell
deskbridge.exe client --server MAC_IP:24920
```

macOS client with scroll tuning:

```bash
DESKBRIDGE_SCROLL_SCALE=1.35 \
DESKBRIDGE_SCROLL_RESPONSE=0.38 \
DESKBRIDGE_SCROLL_MAX_STEP=120 \
DESKBRIDGE_SCROLL_FRAME_MS=8 \
deskbridge client --server WINDOWS_IP:24920
```

## Low Latency Notes

Use a direct LAN IP when possible. Relay paths can noticeably affect keyboard,
mouse, and wheel responsiveness.

Deskbridge coalesces high-frequency mouse and wheel deltas and keeps keyboard and
button events immediate. File transfer chunks are intentionally modest so large
clipboard/file transfers do not monopolize the input stream for too long.

On macOS, the input backend uses a small native C shim. It initializes
`IOHIDManager` for HID device discovery/diagnostics, then posts synthetic input
through the public CoreGraphics event APIs. The shim also disables the default
local event suppression interval on its `CGEventSource` to reduce short pauses
after synthetic events.

The input coalescing window defaults to 2 ms. It can be tuned on both sides:

```bash
DESKBRIDGE_INPUT_FLUSH_MS=2 deskbridge client --server WINDOWS_IP:24920
```

Allowed values are clamped to 1-16 ms. If pointer motion still occasionally
pauses under a high-polling-rate mouse or a relay network path, try 3-4 ms.

## License

Deskbridge is licensed under the GNU General Public License v3.0. See
[`LICENSE`](LICENSE) for details.

---

# Deskbridge 中文说明

Deskbridge 是一个类似 Deskflow 的跨设备工具，目标是在 Windows 和 macOS
之间实现键鼠共享、剪贴板同步和文件投递。

本项目主要是为了解决 Deskflow 类工作流在 macOS 支持和兼容性上的不足，
因此对 macOS 输入注入、键盘映射、剪贴板格式、滚轮行为、打包和权限处理做了
较多优化。

本项目仍然是实验性质，但已经可以用于日常测试。项目开发过程中大量使用了
OpenAI Codex 辅助完成代码生成、调试、打包、文档编写和迭代测试。

由于 macOS 输入兼容性本身存在限制，鼠标仍然可能出现偶发卡顿或短暂停顿。
这是已知问题，将在后续版本继续优化。

## 功能

- 一个二进制文件，内置中英文 GUI 控制面板
- Windows 和 macOS 服务端/客户端模式
- 支持 Windows 控制 macOS，也支持 macOS 控制 Windows
- 跨平台控制时面向常用快捷键的修饰键映射
- 远程 Caps Lock 状态模拟，用于 macOS 字母大小写输入
- Windows Print Screen 映射到 macOS 截图
- 可调节的 macOS 滚轮平滑参数
- 文本、图片、文件三类剪贴板双向同步
- 两端 GUI 文件拖放投递区
- Windows 屏幕边缘文件投递条
- macOS `.app`、`.pkg` 和 Windows Inno Setup 打包
- macOS `.icns` 图标和 Windows `.ico` 安装器/快捷方式图标

## 当前状态

当前支持这些实际使用场景：

- 服务端：Windows
- 客户端：macOS
- 服务端：macOS
- 客户端：Windows
- 传输：局域网、热点局域网或 Tailscale TCP 连接

目前还没有完整实现“把文件从一台电脑直接拖过屏幕边缘并放到另一台电脑的
任意 Finder/Explorer 窗口”。现在可用的是复制粘贴、GUI 拖放投递区，以及
Windows 边缘投递条。macOS 服务端路径是新增能力，可能还需要更多真实设备
手感调优。macOS 作为服务端目前属于 Beta 功能：已经可用，但指针捕获、
修饰键映射和延迟表现还没有完全优化到 Windows 服务端路径的成熟度。

## 安装

从 GitHub Release 下载最新版：

https://github.com/paulhe666/deskbridge/releases

### Windows

1. 下载 `Deskbridge-Setup-1.0.3.exe`。
2. 双击安装包并按安装向导安装。
3. 从开始菜单启动 `Deskbridge`。
4. 如果 Windows 防火墙弹窗，请允许 Deskbridge 访问专用网络。

### macOS

1. 下载 `Deskbridge-1.0.3.pkg`。
2. 双击 pkg 安装。
3. 打开 `系统设置 -> 隐私与安全性 -> 辅助功能`。
4. 添加并启用 `Deskbridge.app`。
5. 从应用程序里启动 `Deskbridge`。

## 快速开始

直接启动 `Deskbridge`。

Windows 服务端、macOS 客户端：

1. Windows 端角色选择 `服务端`
2. 监听地址保持 `0.0.0.0:24920`
3. 如果 Mac 在 Windows 屏幕右侧，边缘选择 `右侧`
4. macOS 端角色选择 `客户端`，服务端地址填写 `WINDOWS_IP:24920`
5. 两端点击 `启动`

macOS 服务端、Windows 客户端：

1. macOS 端角色选择 `服务端`
2. 监听地址保持 `0.0.0.0:24920`
3. 如果 Windows 在 Mac 屏幕右侧，边缘选择 `右侧`
4. Windows 端角色选择 `客户端`，服务端地址填写 `MAC_IP:24920`
5. 两端点击 `启动`

配置文件位置：

```text
~/.deskbridge/config.ini
```

如果中文显示成方框，请确认系统存在 CJK 字体，例如 Hiragino Sans GB、
STHeiti、微软雅黑、黑体或 Noto Sans CJK。

## 修饰键映射

当 macOS 作为服务端控制 Windows 时，可以把 macOS 物理键盘上的 Command、
Control、Option 分别映射成 Windows 的 Ctrl、Win、Alt 或禁用。GUI 中在
启动/停止按钮旁边点击 `设置` 即可调整。保存后需要重启服务才会生效。

默认值保持旧版本兼容：

- Command -> Ctrl
- Control -> Ctrl
- Option -> Alt

如果你的使用习惯里 Command 更像 Windows 徽标键，可以把 Command 改成 Win。

## 卸载

Windows 端可以从系统设置或控制面板卸载 Deskbridge。

macOS 端删除：

```text
/Applications/Deskbridge.app
~/.deskbridge
```

macOS 的 app zip 里也包含 `Uninstall Deskbridge.command`。

## 开发者构建

普通用户请直接使用上面的 Release 安装包。开发者仍然可以从源码打包：

macOS：

```bash
./scripts/package_macos.sh
```

Windows：

```powershell
.\scripts\package_windows.ps1
```

Windows 打包需要 Rust、Visual Studio Build Tools、Windows SDK 和 Inno Setup 6。

## macOS 权限

macOS 端需要辅助功能权限来注入或捕获键盘和鼠标事件。macOS 服务端模式可能
还需要输入监控权限。

打开：

```text
系统设置 -> 隐私与安全性 -> 辅助功能
```

添加并启用 `Deskbridge.app`。如果从终端运行，则需要给对应终端应用权限。

## 命令行

GUI 内部实际启动的也是这些命令。

Windows 服务端：

```powershell
deskbridge.exe server --bind 0.0.0.0:24920 --edge right
```

macOS 客户端：

```bash
deskbridge client --server WINDOWS_IP:24920
```

macOS 服务端：

```bash
deskbridge server --bind 0.0.0.0:24920 --edge right
```

macOS 服务端，并把 Command 映射为 Win：

```bash
DESKBRIDGE_MAC_COMMAND_MAPPING=win \
deskbridge server --bind 0.0.0.0:24920 --edge right
```

Windows 客户端：

```powershell
deskbridge.exe client --server MAC_IP:24920
```

带滚轮参数的 macOS 客户端：

```bash
DESKBRIDGE_SCROLL_SCALE=1.35 \
DESKBRIDGE_SCROLL_RESPONSE=0.38 \
DESKBRIDGE_SCROLL_MAX_STEP=120 \
DESKBRIDGE_SCROLL_FRAME_MS=8 \
deskbridge client --server WINDOWS_IP:24920
```

## 低延迟建议

尽量使用直连局域网 IP。中继路径会明显影响键盘、鼠标和滚轮响应。

Deskbridge 会合并高频鼠标和滚轮 delta，并让键盘、鼠标按键事件立即发送。
文件传输分块刻意保持较小，避免大文件剪贴板/文件传输长时间占用输入流。

macOS 输入后端使用了一层很小的原生 C shim。它会用 `IOHIDManager` 做 HID
设备发现和诊断，然后通过公开的 CoreGraphics 事件 API 发送合成输入。这个
shim 还会把 `CGEventSource` 的默认本地事件抑制窗口设为 0，以减少合成事件后
出现的短暂停顿。

输入合并窗口默认是 2 ms，两端都可以通过环境变量调节：

```bash
DESKBRIDGE_INPUT_FLUSH_MS=2 deskbridge client --server WINDOWS_IP:24920
```

允许值会被限制在 1-16 ms。如果高回报率鼠标或中继网络下仍然偶发暂停，
可以尝试 3-4 ms。

## 许可证

Deskbridge 使用 GNU General Public License v3.0 开源。详情见
[`LICENSE`](LICENSE)。
