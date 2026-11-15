# MotionSizer Studio

电机选型与运动曲线计算桌面应用（Rust + eframe/egui）。

- 支持直线轴与旋转轴两套输入单位：直线 `m / m/s / m/s² / N`，旋转 `° / rpm / rpm/s / N·m`
- 点对点与自定义运动曲线求解，输出速度、加速度、峰值力/扭矩与 RMS 值
- 按需求力、峰值力与安全系数筛选电机，并匹配驱动器、生成选型报告
- 内置电机数据（SQLite；驱动器表为空，不再预置驱动器清单），支持从 PDF/图片本地 OCR 导入新电机参数

## 构建与运行

仓库自带 Windows GNU 工具链，路径在 `tools/`，不依赖系统已安装的 Rust。

```bat
:: 构建（debug）
build.cmd build

:: 运行测试
build.cmd test

:: 启动应用
run.cmd
```

> 注意：`build.cmd test` 只编译测试目标，不会重新链接 `target\debug\motor-selection.exe`；
> 需要看到界面改动时请再执行一次 `build.cmd build`。

## 目录结构

| 路径                                   | 内容                                                         |
| -------------------------------------- | ------------------------------------------------------------ |
| `src/main.rs`                          | 入口与命令行参数（截图、无界面报告、导入自检）               |
| `src/app/`                             | 应用状态、页面装配与报告导出                                 |
| `src/domain/`                          | 纯计算层：运动曲线、电机选型、驱动器匹配                     |
| `src/data/`                            | 电机/驱动器模型与 SQLite 仓储（含首次启动播种）              |
| `src/import/`                          | PDF/图片 OCR 导入：调用 PowerShell + WinRT，解析、预览、入库 |
| `src/ui/`                              | 主题、控件、面板、图表与导入对话框                           |
| `data/motors.json`、`data/drives.json` | 内置种子数据，随二进制编译进去                               |
| `data/motors.db`                       | 运行时数据库；首次启动自动从种子数据创建                     |
| `scripts/import_ocr.ps1`               | 本地 OCR 脚本，随二进制内嵌                                  |
| `docs/`                                | 计算说明、参数表与旧版软件界面参考                           |
| `shots/`                               | 关键界面截图，见 [shots/README.md](shots/README.md)          |

## 命令行参数

```bat
:: 界面截图自检（启动后渲染并保存，便于自动化核对）
target\debug\motor-selection.exe --screenshot shots\name.png [--tab rotary]

:: 无界面导出选型报告（打印生成的文件路径）
target\debug\motor-selection.exe --report data\reports [--tab rotary]

:: 导入自检：选中文件，可选立即识别与提交
target\debug\motor-selection.exe --import <pdf或图片> --import-run [--import-commit]
```

导入自检可配合环境变量 `MOTORSIZER_DB=<临时数据库路径>`，避免污染正式数据库。

## 数据与计算约定

- 内部单位固定为：旋转距离 `rev`、转速 `rpm`、角加速度 `rad/s²`；界面再按 `360`、`60/2π` 换算显示为 `°` 与 `rpm/s`。
- 每个筛选组“一个都不勾选”表示不限制；“勾选若干项”表示只显示这些类别。
- 风冷并入普通冷却（自然冷却），不再单独提供筛选项。
- 自定义模式下，可计算但未填写的输入框会以蓝底显示求解值，编辑后即成为手动约束。

## 相关文档

- [docs/MECHANICS.md](docs/MECHANICS.md)：运动学与选型公式说明
- [docs/mechanics.json](docs/mechanics.json)：公式与参数的机器可读版本
- [docs/reference-legacy-ui.png](docs/reference-legacy-ui.png)：旧版 MotorSizer 界面参考
