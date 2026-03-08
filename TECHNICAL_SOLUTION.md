# Voxio 技术方案

## 1. 目标与范围

Voxio 的核心目标是做一个开源、低延迟、跨应用的语音输入工具。结合 PRD，v1.1 的技术方案需要优先解决四件事：

1. 全局快捷键触发录音。
2. 语音实时/准实时转写。
3. 在任意应用当前光标处安全插入文本。
4. 提供极简状态反馈与基础设置能力。

当前方案按桌面端优先设计，首发平台建议为 macOS，后续再扩展 Windows。原因很直接：PRD 的关键价值不是“网页里能语音输入”，而是“任何应用都可用”，这天然要求桌面级系统能力。

## 2. 产品形态与总体架构

### 2.1 形态选择

建议采用：

- 桌面客户端：`Tauri 2 + Rust`
- 前端设置界面：`React + TypeScript`
- 语音识别引擎：本地优先，云端可插拔

这样做的原因：

- `Rust` 适合处理全局快捷键、音频流、系统事件、权限与性能敏感模块。
- `Tauri` 包体积小，跨平台能力强，适合开源桌面工具。
- 前端设置页和悬浮状态 UI 用 Web 技术实现，迭代快。
- 语音引擎做成 Provider 抽象，避免后续被单一模型或单一服务锁死。

### 2.2 总体架构

```text
+-----------------------+
|   Settings UI         |
| React + Tauri WebView |
+-----------+-----------+
            |
            v
+-----------------------+
|   App Core (Rust)     |
| - State Machine       |
| - Command Router      |
| - Config Manager      |
| - Permission Manager  |
+-----+--------+--------+
      |        | 
      |        +--------------------+
      |                             |
      v                             v
+-------------+              +---------------+
| Audio Layer |              | Overlay UI    |
| CPAL/CoreAu |              | status window |
+------+------+              +-------+-------+
       |                             |
       v                             |
+-------------+                      |
| ASR Engine   |<--------------------+
| Provider API |
+------+------+ 
       |
       v
+-------------+
| Text Post    |
| punctuation  |
| segmentation |
+------+------+ 
       |
       v
+-------------+
| Text Inject  |
| Accessibility|
| Clipboard    |
+-------------+
```

## 3. 核心模块设计

### 3.1 Global Hotkey 模块

职责：

- 注册系统级快捷键。
- 处理开始录音、停止录音、取消输入。
- 避免与系统默认快捷键冲突。

建议实现：

- macOS：优先基于 Quartz Event Tap 或成熟 Rust crate 封装。
- Windows：后续使用 `RegisterHotKey`。

设计要点：

- 快捷键配置持久化到本地。
- 录音中再次按下同一快捷键，执行 `stop_and_commit`。
- `Esc` 可作为全局取消键，但需允许关闭。

### 3.2 Audio Capture 模块

职责：

- 获取麦克风输入流。
- 做静音检测、分帧、缓存。
- 为实时识别或停止后识别提供 PCM 数据。

建议实现：

- Rust 音频库：`cpal`
- 平台适配：
  - macOS 走 CoreAudio
  - Windows 走 WASAPI

设计要点：

- 统一内部音频格式：`16kHz / mono / PCM16`
- 每 20ms 到 40ms 一个音频帧，便于流式 ASR
- 增加简单 VAD（Voice Activity Detection）用于静音自动停止

### 3.3 Speech Recognition 模块

目标：

- 默认离线可用。
- 支持中英文及后续扩展语言。
- 兼容实时 partial transcript 和 final transcript。

建议采用 Provider 分层：

```text
AsrProvider
├── WhisperCppProvider      # 本地默认
├── FasterWhisperProvider   # 后续可选
└── CloudAsrProvider        # 未来扩展 OpenAI/Deepgram 等
```

#### v0.1 建议

默认本地引擎使用：

- `whisper.cpp`

原因：

- 开源成熟，离线可用。
- 中英文能力足够覆盖 MVP。
- 社区生态成熟，便于打包模型。

能力边界：

- 小模型延迟更低，但准确率一般。
- 大模型准确率更高，但对 CPU/内存要求高。

因此建议：

- 默认模型：`base` 或同等级多语言模型
- 设置项允许切换为 `small`

### 3.4 Text Post-Processing 模块

职责：

- 自动断句
- 自动标点
- 首字母大写
- 去除明显口语停顿词

分两层实现：

1. 规则层
2. AI 优化层

#### 规则层

v0.1-v0.2 先上规则层：

- 英文句尾停顿 + 语义词概率推断句号
- 中文按停顿与语气词插入 `，。！？`
- 英文句首大写

优点：

- 快
- 离线
- 可控

#### AI 优化层

v0.3 再引入可选 AI 文本润色：

- 基于本地或云端 LLM
- 仅在用户开启时使用
- 默认关闭，避免隐私争议和额外延迟

### 3.5 Text Injection 模块

这是整个产品最关键、最容易踩坑的模块。

目标：

- 不切换焦点
- 在当前光标处插入文本
- 尽量兼容浏览器、IDE、文档工具、聊天工具

建议按“分层回退”设计：

#### 一级方案：Accessibility API 插入

- macOS 使用 Accessibility API 获取焦点元素
- 优先对可编辑元素执行插入/替换

优点：

- 语义正确
- 对大多数原生输入框兼容更好

缺点：

- 权限要求高
- 某些应用兼容性不稳定

#### 二级方案：粘贴板注入

流程：

1. 备份当前剪贴板
2. 将转写文本写入剪贴板
3. 模拟 `Cmd+V`
4. 恢复原剪贴板

优点：

- 兼容性极强

缺点：

- 会短暂污染剪贴板
- 某些安全输入框不可用

#### 三级方案：按键逐字符输入

仅作为兜底：

- 速度慢
- 容易被输入法和快捷键干扰

结论：

- 默认先尝试 Accessibility 注入
- 失败后自动回退到 Clipboard Paste
- 设置中提供“强制使用粘贴模式”

### 3.6 Overlay / 状态提示模块

职责：

- 展示 `Idle / Listening / Processing`
- 提供录音动画
- 可点击取消

实现建议：

- Tauri 独立透明窗口
- 常驻最上层但不抢焦点
- 小尺寸 HUD，默认屏幕顶部居中

设计要点：

- 状态变化必须在 100ms 内反馈
- 录音开始要有明确视觉反馈
- 完成和取消自动消失

### 3.7 Settings 模块

建议首版配置项：

- 快捷键
- 语言
- 自动标点开关
- 静音自动停止时长
- 注入模式
- 模型大小
- 开机启动

配置存储：

- 本地 `TOML` 或 `JSON`
- 建议路径：
  - macOS: `~/Library/Application Support/Voxio/config.json`

## 4. 状态机设计

整个录音与转写流程需要严格状态机约束，避免重复触发和脏状态。

```text
Idle
  -> Listening        触发快捷键并成功拿到麦克风
Listening
  -> Processing       用户停止 / 检测静音
Listening
  -> Cancelled        用户取消 / 无语音 / 权限失败
Processing
  -> Injecting        ASR 成功
Processing
  -> Error            ASR 失败
Injecting
  -> Idle             插入成功
Cancelled
  -> Idle
Error
  -> Idle
```

关键约束：

- `Listening` 状态禁止重复开启录音流。
- `Processing` 状态禁止再次触发新的会话。
- 每次 dictation session 都有唯一 `session_id`，避免旧异步结果写回新状态。

## 5. 关键业务流程

### 5.1 主流程

1. 用户按下全局快捷键。
2. 系统校验麦克风和辅助功能权限。
3. 进入 `Listening`，显示录音提示。
4. 音频流进入 ASR 引擎。
5. 用户再次按快捷键或静音超时，停止录音。
6. 进入 `Processing`。
7. 执行文本后处理。
8. 通过注入模块写入当前焦点位置。
9. UI 自动消失，回到 `Idle`。

### 5.2 失败流程

- 麦克风未授权：弹权限提示，不进入录音。
- 无语音输入：自动取消并提示 `No speech detected`。
- ASR 失败：提示重试，不进行注入。
- 注入失败：保底复制到剪贴板，并提示用户手动粘贴。

## 6. 平台能力与权限设计

### 6.1 macOS

必须处理：

- 麦克风权限
- Accessibility 权限
- 输入监控相关能力（取决于全局快捷键实现）

技术重点：

- 首次启动时做权限预检查
- 设置页展示缺失权限状态和引导按钮
- 对权限失败做明确错误码和文案

### 6.2 Windows

后续支持项：

- 全局快捷键
- 麦克风
- UI Overlay
- 文本注入

注意：

- 不同应用对输入模拟兼容性差异较大
- 需要针对 Electron、JetBrains、Office、浏览器做专门验证

## 7. 数据与隐私设计

Voxio 作为语音输入工具，隐私必须前置。

原则：

- 默认本地识别
- 默认不上传音频
- 默认不保存录音
- 日志中禁止记录原始语音和完整文本

可采集数据：

- 错误码
- 识别耗时
- 注入成功率
- 权限状态

如果未来加入遥测：

- 必须用户显式同意
- 仅采集匿名统计

## 8. 非功能要求

### 8.1 性能目标

建议指标：

- 热键触发到开始录音提示：< 100ms
- 录音停止到文本可插入：短句 < 800ms
- 常驻内存：
  - 空闲态 < 200MB
  - 识别态根据模型浮动

### 8.2 稳定性目标

- 连续 100 次短句 dictation 不崩溃
- 权限缺失、注入失败、无语音输入都能回到 `Idle`
- 异常情况下不会留下悬挂音频流

### 8.3 兼容性目标

首批验收应用：

- Chrome / Safari
- VS Code
- Cursor / JetBrains IDE
- Notion
- Slack / Discord
- Word / Pages

## 9. 工程目录建议

建议仓库结构：

```text
voxio/
├── apps/
│   └── desktop/
│       ├── src-tauri/         # Rust core
│       └── src/               # React UI
├── crates/
│   ├── audio/
│   ├── asr/
│   ├── text_postprocess/
│   ├── injector/
│   ├── overlay/
│   └── shared/
├── models/
│   └── whisper/               # 下载或缓存模型
├── docs/
│   ├── architecture.md
│   ├── permissions.md
│   └── qa-matrix.md
└── scripts/
```

说明：

- `crates` 拆分后，核心能力能被测试与复用。
- `apps/desktop` 负责产品组装，不把所有逻辑都塞进 Tauri 主工程。

## 10. 关键接口设计

### 10.1 Rust 内部服务接口

```rust
pub trait AsrProvider {
    fn start_stream(&mut self, config: AsrConfig) -> Result<()>;
    fn push_audio(&mut self, frame: AudioFrame) -> Result<()>;
    fn stop(&mut self) -> Result<TranscriptionResult>;
}

pub trait TextInjector {
    fn inject(&self, text: &str) -> Result<InjectResult>;
}
```

### 10.2 前后端命令接口

建议 Tauri commands：

- `start_dictation`
- `stop_dictation`
- `cancel_dictation`
- `get_app_state`
- `get_settings`
- `update_settings`
- `request_permissions`

前端事件订阅：

- `state_changed`
- `partial_transcript`
- `final_transcript`
- `permission_changed`
- `error_occurred`

## 11. Roadmap 对应实现策略

### v0.1

范围：

- 全局快捷键
- 开始/停止录音
- 本地 ASR
- 文本注入
- 极简状态提示

建议取舍：

- 先只做 macOS
- 先只支持手动停止，不做复杂连续听写
- 自动标点只做基础规则版

### v0.2

范围：

- 自动标点增强
- 多语言切换
- 静音自动停止
- 注入兼容性优化

### v0.3

范围：

- AI 文本优化
- 翻译输入
- 云端 ASR Provider

### v0.4

范围：

- Voice commands
- Continuous dictation
- Windows 支持

## 12. 主要风险与应对

### 风险 1：跨应用文本注入兼容性不稳定

应对：

- 建立应用兼容矩阵
- 采用 Accessibility + Clipboard 双通道
- 将注入层做独立可测试模块

### 风险 2：本地 ASR 延迟过高

应对：

- 模型分级
- 优先短句场景优化
- 后续增加流式 partial transcript

### 风险 3：macOS 权限流程影响首次体验

应对：

- 首次启动即做权限向导
- 把失败原因做成明确可操作提示

### 风险 4：开源项目安装门槛高

应对：

- 提供一键下载模型
- 提供最小可运行版本
- 文档中明确 CPU 要求与权限配置

## 13. 结论

Voxio 的正确实现路径不是先做一个网页语音输入框，而是先做一个桌面级输入工具内核。首版建议聚焦：

- `macOS + Tauri + Rust`
- `whisper.cpp` 本地识别
- `Accessibility + Clipboard` 双通道文本注入
- 极简悬浮状态 UI

这样可以最快验证 PRD 的核心价值：用户在任何应用中按下快捷键，说话，然后文字稳定进入当前输入位置。
