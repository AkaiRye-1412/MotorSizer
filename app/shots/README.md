# 界面截图

保留关键功能验证截图，作为界面行为的对照记录。新增截图请沿用这些文件名语义，避免堆积一次性实验文件。

| 文件 | 内容 |
| --- | --- |
| `final-linear.png` | 直线轴页面整体布局与选型结果 |
| `final-rotary.png` | 旋转轴页面整体布局 |
| `formula-check.png` | 默认工况（20 kg / 0.8 m / 0.5 s）与旧版软件公式对齐 |
| `computed-fields.png` | 点对点模式下最大速度/加速度以蓝底只读值显示 |
| `custom-auto-fields.png` | 自定义模式下自动补算值填入输入框（可编辑） |
| `filters-default.png` | 推荐型号筛选项默认全不勾选、列表仍完整 |
| `rotary-units.png` | 旋转轴单位：角度 °、速度 rpm、加速度 rpm/s |
| `import-ready.png` | 导入电机参数：已选择文件、等待识别 |
| `import-preview.png` | OCR 识别结果预览（覆盖率/冲突/缺失标记） |
| `import-committed.png` | 导入提交后写入数据库 |
| `import-error.png` | OCR 失败时的错误提示与阶段信息 |

重新生成：

```bat
run.cmd --screenshot shots\final-linear.png
run.cmd --screenshot shots\rotary-units.png --tab rotary
```
