# Palettes

[English](README.md) · **简体中文**

Rust 编写的 Wayland 桌面 shell 与桌面小控件库：状态栏、程序坞、启动器、弹出层与桌面挂件，Vulkan 渲染。基于 [aegle](https://github.com/antinomie1/aegle)（保留式 UI、布局、文字、标记语言、Vulkan 渲染器），自带面向 shell 的 Wayland 客户端。支持任何实现 wlr-layer-shell 的混成器；配合 [Pastel](https://github.com/antinomie1/Pastel) 还能显示并操作工作区与窗口。

- **标记语言面板**：一个 `.aegle` 文件即一个面板，自动在每块屏幕上打开，保存即原地热重载
- **按名字取 shell 状态**：声明 `state time`、`state workspaces`、`state volume`……即自动保持最新
- **资产自动化**：按名字或 app id 查主题图标（含 SVG）、自动注册 `fonts/` 中的字体、路径相对文档解析
- **服务**：时钟、工作区与窗口（Pastel IPC）、PipeWire 音量与静音、已安装应用
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

全部状态、记录类型、宿主动作与面板位置属性见 [docs/markup.md](docs/markup.md)。

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

在 Rust 中运行文档：`palettes::run("bar.aegle")`。更多示例见 [`crates/palettes/examples`](crates/palettes/examples) 与 [`examples`](examples)。

## 构建

需要 Rust ≥ 1.88、libxkbcommon、libpipewire-0.3、clang（bindgen）与 Vulkan 驱动。`pastel-ipc` 来自同级目录 `../Pastel` 中的 [Pastel](https://github.com/antinomie1/Pastel)。

```sh
cargo build --release
cargo run --release -p palettes -- examples/bar.aegle
cargo run --release -p palettes --example bar
```

## Crate

| Crate | 职责 |
|---|---|
| `palettes-wayland` | 面向 layer surface、弹出层、输出与输入的 Wayland 客户端，实现 aegle 平台契约 |
| `palettes-shell` | `Shell` 与 `Surface`：shell surface 上的 aegle UI 树与 Vulkan 呈现 |
| `palettes-clock` | 本地时间格式化与按实时时钟对齐的计时 |
| `palettes-workspace` | 与混成器无关的工作区与窗口模型；Pastel 后端 |
| `palettes-audio` | PipeWire 默认输出与输入 |
| `palettes-assets` | 资产根目录、图标主题、desktop 条目、文件监视 |
| `palettes` | 门面、服务、标记语言集成与 `palettes` 命令 |

设计说明见 [docs/architecture.md](docs/architecture.md)，开发约定见 [AGENTS.md](AGENTS.md)，路线图见 [TODO.md](TODO.md)。

## 许可证

BSD 2-Clause，见 [LICENSE](LICENSE)。
