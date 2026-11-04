# Palettes

[English](README.md) · **简体中文**

Rust 编写的 Wayland 桌面 shell 与桌面小控件库：状态栏、程序坞、启动器、弹出层与桌面挂件，Vulkan 渲染。基于 [aegle](https://github.com/antinomie1/aegle)（保留式 UI、布局、文字、标记语言、Vulkan 渲染器），自带面向 shell 的 Wayland 客户端。支持任何实现 wlr-layer-shell 的混成器；配合 [Pastel](https://github.com/antinomie1/Pastel) 还能显示并操作工作区与窗口。

- **标记语言面板**：一个 `.aegle` 文件即一个面板，自动在每块屏幕上打开，保存即原地热重载
- **按名字取 shell 状态**：声明 `state time`、`state workspaces`、`state volume`……即自动保持最新
- **Luau 脚本**：`bar.aegle` 旁的 `bar.luau` 填充状态、定义动作、运行命令、定时器与文件监视，保存即重载
- **弹出层**：`Panel { edge: popup }` 在被点击的控件旁打开，尺寸随内容，点击别处即关闭；工具提示以弹出层显示在栏外
- **命令行**：`palettes msg toggle launcher.aegle` 或任意动作，供混成器快捷键调用
- **文本输入**：组合键、输入法、剪贴板、主选区（中键粘贴）、拖放
- **资产自动化**：按名字或 app id 查主题图标（含 SVG）、自动注册 `fonts/` 中的字体、路径相对文档解析
- **服务**：时钟、工作区与窗口（Pastel IPC；其他混成器上经 ext-workspace-v1 与 wlr-foreign-toplevel-management 获取）、PipeWire 设备、默认设备切换、按应用音量与峰值电平、处理器与内存占用、已安装应用
- **D-Bus 系统服务**：电池与电源模式（UPower）、媒体播放（MPRIS）、通知服务器、带菜单的系统托盘（StatusNotifierItem、DBusMenu）、Wi-Fi（NetworkManager）、蓝牙（BlueZ）、背光（经 logind）：`state battery`、`state tray`、`host.wifi_connect(...)`
- **脚本中的 D-Bus**：`dbus.system(name, path)` 调用方法、跟随属性与信号，不阻塞 shell
- **毛玻璃与圆角**：圆角、半透明面板，并让混成器模糊其后的桌面（`radius`、`background: #1e1e2ea0`、`opacity`、`blur: true`），需混成器支持 ext-background-effect-v1，如 KWin 与 Pastel
- **壁纸**：`Panel { edge: background }` 中放入最高 4K 的图片，可模糊；其主色成为调色板令牌 `palette.1`–`palette.5`；脚本也可用 `palette{...}` 设置，例如跟随混成器配色
- **锁屏**：`Panel { edge: lock }` 在每块屏幕上锁定会话（ext-session-lock-v1）；`host.unlock(password)` 经 PAM 校验
- **空闲即静止**：无变化时不运行任何代码；服务在首次使用时才启动
- **可组合**：可与任何 aegle 控件库结合，例如 [am3](https://github.com/antinomie1/am3)（`--features am3`）

## 标记语言

`bar.aegle`：

```text
use "palettes.aegle"

Panel {
    edge: top
    size: 32dp
    state output_workspaces: list<Workspace> = []
    state time: string = ""
    Row {
        for ws in output_workspaces key ws.id {
            Button { text: ws.name; on clicked { host.focus_workspace(ws.id) } }
        }
        Text { text: time; grow: 1 }
    }
}
```

```sh
palettes bar.aegle            # 或放到 ~/.config/palettes/shell.aegle 后直接运行 `palettes`
palettes --check bar.aegle    # 只检查，不打开
```

旁边的 `bar.luau` 提供服务之外的数据：

```lua
-- 填充所有声明了 `state disk: string` 的文档。
every(60, function()
    exec("df --output=pcent / | tail -1", function(out)
        state.disk = (out:gsub("%s", ""))
    end)
end)

function lock() host.spawn("swaylock") end   -- 标记中：host.lock()
```

全部状态、记录类型、宿主动作与面板位置属性见 [docs/markup.md](docs/markup.md)；脚本见 [docs/scripts.md](docs/scripts.md)。

## Rust

```rust
use palettes::prelude::*;

fn main() -> Result<()> {
    let shell = Shell::new()?;
    let bar = shell.panel(PanelOptions::top(32))?;
    let time = bar.text("");
    shell.clock(&time, "%H:%M", |time, now| time.set_text(now))?;
    shell.run()
}
```

Surface 即其根容器：`bar.set_radius(12.0)`、`bar.set_background(color)`、`bar.set_opacity(0.9)` 设定形状与色调，`bar.set_blur(&bar, true)` 让混成器模糊其后的桌面。输入区域跟随圆角形状；`bar.set_input(&pill, true)` 可将其限定在指定节点上。

标记可以画矢量形状（`Shape` 用 SVG 路径数据，`Arc` 画环形与仪表），给缩放、旋转与透明度加动画，用弹簧与贝塞尔曲线缓动过渡，把图片铺在面板背后（`Image { fit: cover; inset: 0dp }`），并用 WGSL 片元着色器填充元素，只有用到 `time` 时才逐帧绘制（`Shader { source: "aurora.wgsl" }`）；见 [`examples/rings.aegle`](examples/rings.aegle)、[`examples/aurora.aegle`](examples/aurora.aegle) 与 [`docs/markup.md`](docs/markup.md)。

在 Rust 中运行文档：`palettes::run("bar.aegle")`。更多示例见 [`crates/palettes/examples`](crates/palettes/examples) 与 [`examples`](examples)。

## 构建

需要 Rust ≥ 1.88、libxkbcommon、libpipewire-0.3、libpam、clang（bindgen）与 Vulkan 驱动。

```sh
cargo build --release
cargo run --release -p palettes -- examples/bar.aegle
cargo run --release -p palettes --example bar
```

## Crate

| Crate | 职责 |
|---|---|
| `palettes-wayland` | 面向 layer surface、弹出层、输出与输入（组合键、输入法、剪贴板、拖放）的 Wayland 客户端，实现 aegle 平台契约 |
| `palettes-shell` | `Shell` 与 `Surface`：shell surface 上的 aegle UI 树与 Vulkan 呈现 |
| `palettes-clock` | 本地时间格式化与按实时时钟对齐的计时 |
| `palettes-workspace` | 与混成器无关的工作区与窗口模型；Pastel 后端 |
| `palettes-audio` | PipeWire 默认输出与输入 |
| `palettes-assets` | 资产根目录、图标主题、desktop 条目、文件监视 |
| `palettes-system` | D-Bus 服务：UPower、MPRIS、通知、托盘、NetworkManager、BlueZ |
| `palettes` | 门面、服务、标记语言集成、Luau 脚本与 `palettes` 命令 |

设计说明见 [docs/architecture.md](docs/architecture.md)，开发约定见 [AGENTS.md](AGENTS.md)，路线图见 [TODO.md](TODO.md)。

## 许可证

BSD 2-Clause，见 [LICENSE](LICENSE)。
