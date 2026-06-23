# Pointer movement metrics

这个目录用于离线评估 Deskbridge 光标移动是否卡顿、跳变或输出节奏不均匀。

它不依赖 GUI，也不依赖平台 API。只要把服务端采集到的鼠标事件、客户端注入前后的光标轨迹，保存成 CSV，就可以用同一套指标比较 macOS / Windows / Linux 的效果。

## 文件

```text
analyze_pointer_trace.py  光标轨迹指标分析脚本
sample_trace.csv          示例轨迹文件
```

## 输入 CSV 格式

推荐格式：

```csv
t_ms,x,y
0,100,100
8,104,100
16,108,100
```

也支持相对位移格式：

```csv
t_ms,dx,dy
0,0,0
8,4,0
16,4,0
```

字段说明：

```text
t_ms：时间戳，单位 ms，必须递增
x,y：绝对坐标，单位 px
dx,dy：相对位移，单位 px
```

如果同时存在 `x,y` 和 `dx,dy`，脚本优先使用 `x,y`。

## 使用方法

在项目根目录执行：

```bash
python3 tests/pointer_metrics/analyze_pointer_trace.py tests/pointer_metrics/sample_trace.csv
```

输出 JSON：

```bash
python3 tests/pointer_metrics/analyze_pointer_trace.py tests/pointer_metrics/sample_trace.csv --json
```

调整卡顿阈值，例如认为大于 20ms 的帧间隔算一次卡顿：

```bash
python3 tests/pointer_metrics/analyze_pointer_trace.py tests/pointer_metrics/sample_trace.csv --stutter-threshold-ms 20
```

## 主要指标

```text
sample_count：采样点数量
duration_ms：总时长
mean_interval_ms：平均事件间隔
p95_interval_ms：95 分位事件间隔
max_interval_ms：最大事件间隔
interval_jitter_ms：事件间隔标准差，越大说明输出越不均匀
stutter_count：超过阈值的间隔次数
stutter_rate：卡顿间隔占比
max_step_px：相邻两点最大位移，异常大通常表示跳变
p95_step_px：相邻位移 95 分位
velocity_jitter_px_s：速度抖动
acceleration_spike_count：加速度突变次数
stutter_score：综合卡顿分，越低越好
```

## 建议采集位置

为了定位卡顿发生在哪一段，建议分别记录三类轨迹：

```text
server_capture：服务端刚采集到的原始鼠标事件
protocol_send：准备发送给客户端的事件流
client_inject：客户端真正注入前的事件流
```

如果 `server_capture` 已经不均匀，说明问题在采集端。  
如果 `server_capture` 平滑但 `client_inject` 不均匀，说明问题在网络、队列或协议处理。  
如果 `client_inject` 平滑但肉眼仍然卡，说明问题在平台注入方式，例如 Windows 上绝对 SetCursorPos 或 Linux uinput/Wayland 桥接。

## 判读建议

一般来说：

```text
mean_interval_ms 接近 1~8ms：高频鼠标事件正常
p95_interval_ms 明显大于 mean：输出节奏不稳
max_interval_ms 大于 30~50ms：肉眼容易感觉卡一下
max_step_px 很大：可能出现光标跳变
velocity_jitter_px_s 很大：移动速度不连续
stutter_score 越高：整体越容易感到卡顿
```

这些指标不能完全替代真实手感，但可以稳定比较不同版本的优化效果。
