# 电机选型软件 — 功能与公式规格

逆向依据：`AkribisSystemsMotorSizer.exe`（VB.NET / .NET Framework 4，2018-04-26 版本）。
所有公式来自反编译源码中的方法，行号为反编译文件 `MotorSizer_Dialog.cs` 的行号。

## 1. 两种选型对象（顶部页签）
- 直线电机（Linear）：输入直线运动参数，输出"力 Force"（N）。
- 旋转电机（Rotary）：输入转动运动参数，输出"力矩 Torque"（N·m）。
旋转与直线共用同一套运动学结构（梯形/T-曲线、点到点/自定义），把"质量"换成"转动惯量"、"力"换成"力矩"即可。

## 2. 两种运动曲线
- 梯形曲线（trapezoid=1）：加速段 → 匀速段 → 减速段。
- T-曲线 / 三角形（trapezoid=0）：只有加速段 → 减速段，无匀速段（峰值速度处瞬时转折）。
来源：`Trapezoid_RadioButton_CheckedChanged`（行 29355 附近）、`Triangle_RadioButton_CheckedChanged`（行 29342）。

## 3. 运动学求解（梯形）
给定 速度 V、运动时间 T、距离 S、加速度 A 中任意两项，求其余量。
三段时间：`ta`（加速段时长）、`tc`（匀速段时长）、`td`（减速段时长），`ta = td`。
来源：`calcParameters()`（行 23290–23748）。

- 已知 V、T（无 A）：`ta = T/3`，`tc = T/3`，`A = V/ta`，`S = 0.5·V·(tc + T)`
- 已知 T、S（无 V、无 A）：用二次方程；程序用 `Quadratic_Solver`（行 22517）
- 已知 V、S（无 T、无 A）：`T = 1.5·S/V`，`ta = T/3`，`A = V/ta`
- 已知 V、A：`ta = V/A`，`tc = T − 2·ta`，`S = 0.5·V·(tc + T)`
- 已知 S、A（无 V）：`V = √(S·A)`，`T = 2V/A`（T-曲线情形）
- T-曲线（trapezoid=0）：`tc = 0`，`S = 0.5·V·T`，`ta = td = T/2`，`A = V/ta`
- 一致性容差：`5e-5`（绝对值比较）
- 加速度上限：`A > 100000` 时报错"Required Acceleration Too High"（行 23739）

## 4. 力 / 力矩计算（核心）
来源：`calcSingle()`（行 25169–25927，梯形/T-曲线），公式在行 25835–25862。

重力分力（倾斜安装）：
```
Fg = m · g · sin( (倾角 mod 360)/360 · 2π )      g = 9.81
```

- 加速段力：`Fa = m·A + Ff + Fg + Fo`
- 减速段力：`Fd = −m·A + Ff + Fg + Fo`
- 峰值力：`Fpeak = max(|Fa|, |Fd|)`
- 周期总时长：`Ttot = T + Tdwell`（Tdwell = 停顿/驻留时间）
- 有效值力（RMS，梯形）：
```
Frms = √( (Fa²·ta + (Ff+Fg+Fo)²·tc + Fd²·td + (Fg+Fo)²·Tdwell) / Ttot )
```
- T-曲线（tc=0）：上式去掉匀速段项。

其中 m=质量(kg)，Ff=摩擦力(N)，Fo=对向/外力(N)。旋转电机时 m→转动惯量 J，力→力矩，去掉重力分力（或由外力矩项替代）。

## 5. S-曲线（jerk limited，复选框"S-曲线"）
来源：`calcSingle_S_Curve()`（行 25929–26705）。
- jerk 段参数：`jerk_p = 4V / ta²`（行 26599）
- 实际加减速段时长修正：`ta' = V/A + A/jerk_p`（行 26614）
- 匀速段时长：`tc = S − V²/(2A) − (V·A + V·Dec)/(2·jerk_p)`（行 26615）
- 峰值/RMS 力公式结构与梯形相同，用修正后的 `ta'`、`tc`。

## 6. 安全余量（Safety Factor）
来源：行 25789–25833。
选项 0%/5%/10%/20%/30%/40%/50%/60%/70%/80%/90%/100% → 倍率 1.00 … 2.00。
默认（未匹配）= 1.5（50%）。
选型时用 `需求力 × 安全倍率` 与电机规格比较。

## 7. 电机推荐与匹配
来源：`Recommend_Motor_L()`（行 30594–30842）。
- 比较值：`needRMS = Frms · 安全倍率`，`needPeak = Fpeak · 安全倍率`。
- 入选条件：电机 `PeakForce ≥ needPeak` 且 `ContinuousForce ≥ needRMS`。
- 安全余量百分比：
```
safety% = ( min(ContForce/Frms, PeakForce/Fpeak) − 1 ) · 100
```
- 若启用"窗口"限制（CheckBox + TrackBar，行 30642）：
```
min(ContForce/needRMS, PeakForce/needPeak) < 1 + TrackBar/100
```
即只推荐余量不超过窗口百分比的电机，避免过度选型。
- 音圈电机(VOICE)额外条件：`行程(mm)·0.001 ≥ 需求距离`（行 30761）。
- 冷却方式过滤：型号名含 W=水冷，第 6 位含 A=风冷（AUM/ACM 系列）。

电机库分类（Combolist/Forcelist）：ACM（无铁芯直线）、AUM（有铁芯直线）、VOICE（音圈 AVA/AVM/AVR）、DDR（直驱旋转 ADR/ACD/ACW/AXD）、ALL（全部）。

## 8. 多曲线汇总选型
来源：`calcAll(...)`（行 28486–29340）。
- 每条曲线（profile）独立计算 Fa/Fd/各段时间。
- 总峰值力：`max(所有曲线 Fpeak)`
- 总 RMS 力：`√( Σ 各段(F²·时长) / Σ 周期总时长 )`（行 29328）
- 其余：总运动时间=Σ周期、总停顿=Σ停顿、总距离=Σ距离、最大速度取最大。
- 再用第 7 节规则从全库推荐电机。

## 9. 选中电机后展示的规格
来源：`ShowSelectedMotorAll()`（行 35268 起），读取 `Motors/<型号>.txt`。
直线电机字段（顺序）：
Peak Force, Continuous Force, Peak Power, Continuous Power, Peak Current, Continuous Current,
Force Constant, Back EMF Constant, Terminal Resistance(Phase Resistance), Phase Inductance,
Motor Constant, Coil WeightB, Coil LengthB, Coil WeightA, Coil LengthA, Thermal Resistance。
旋转电机：Peak/Continuous Torque、Torque Constant、Rotor Inertia 等。
DDR 的 Back EMF 显示换算：`值 · 30/π`（行 35288）。
热传导系数 = `1 / Thermal Resistance`（行 35303）。

## 10. 旋转负载转动惯量计算
来源：`LoadTypeRotary.cs`（行 1078）。
- 实心圆柱（Type 0）：`J = m·R²/2`
- 空心圆柱（Type 1）：`J = m·(R² + r²)/2`
- 矩形/长方体（Type 2）：`J = m·(3R² + L²)/12`
- 另一形式（Type 3）：`J = m·(R² + r² + L²/3)/4`

## 11. 图表
- 图1：速度-时间图（Velocity vs Time），梯形/T/S 曲线轮廓。
- 图2：力-时间图（Force vs Time），各段 Fa/匀速力/Fd/驻留力的阶梯轮廓。
原版用 ZedGraph 控件绘制。本实现用 egui 自绘折线。

## 单位
质量 kg；力 N；速度 m/s；加速度 m/s²；距离 m；时间 s；角度 °；
旋转力矩 N·m；转动惯量 kg·m²；温度 °C；电流 A；电压 V；功率 W。
