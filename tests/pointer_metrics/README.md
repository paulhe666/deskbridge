# Pointer movement metrics

这个目录用于离线评估 Deskbridge 光标移动是否卡顿、跳变或输出节奏不均匀。

它不依赖平台 API。你可以手动提供 CSV，也可以在 GUI 的 Settings / Developer 里打开 Pointer movement trace，让实际连接过程自动记录 CSV，再用同一套指标比较 macOS / Windows / Linux 的效果。

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

## 实际连接时采集

GUI 方式：打开 Settings / Developer / Pointer movement trace。打开后填写 Trace 保存目录，例如 macOS 上的 `/Users/lpj/Desktop`，Windows 上的 `C:\\Users\\你的用户名\\Desktop`。留空则使用系统临时目录。

启动服务后，Deskbridge 会在目录下自动新建 CSV 文件，不会覆盖旧文件。文件名格式类似：

```text
deskbridge-pointer-server-1791234567890-12345.csv
deskbridge-pointer-client-1791234567890-12345.csv
```

命令行方式也可以直接设置环境变量。建议填目录：

```bash
DESKBRIDGE_POINTER_TRACE=/tmp deskbridge server --bind 0.0.0.0:24920
DESKBRIDGE_POINTER_TRACE=/tmp deskbridge client --server 192.168.1.10:24920
```

如果你填的是 `.csv` 文件路径，也不会覆盖原文件。程序会自动在文件名后追加角色、时间戳和进程号，例如 `deskbridge-pointer-server-1791234567890-12345.csv`。

断开连接后用下面的脚本分析。不要直接运行 CSV 文件；要运行 `analyze_pointer_trace.py`，并把 CSV 路径作为参数传进去。

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
