# esp8266-hal

[English](README.md) | [简体中文](README_cn.md)

> ⚠️ **注意**：官方 `esp-rs` 组织已弃用该 HAL 库，后续不再提供官方维护。但本项目集成了现代 Xtensa Rust 工具链的兼容性补丁，确保其可以在现代 Rust 工具链（`esp` channel，已在 rustc 1.93+ nightly 验证）上正常编译并运行。

基于 Rust 语言编写的 [ESP8266](https://en.wikipedia.org/wiki/ESP8266) 微控制器（Xtensa LX106 架构）实验性嵌入式硬件抽象层（`no_std` HAL），基于 [`embedded-hal`](https://github.com/rust-embedded/embedded-hal) trait 体系设计。

社区讨论链接：https://matrix.to/#/#esp-rs:matrix.org!

使用该 crate 的示例工程模板可参考 [这里](https://github.com/icewind1991/xtensa-rust-quickstart/tree/esp8266)。

---

## 支持的外设与特性

- **GPIO**：支持数字输入、推挽输出、开漏输出以及复用引脚功能（UART、SPI 等）。
- **UART / 串口通信**：支持 UART0（发送与接收）和 UART1（仅发送）、基于 `core::fmt::Write` 的格式化输出，以及中断驱动收发。
- **定时器与延时**：基于 FRC1 硬件定时器，支持 `embedded-hal` 阻塞延时 trait 与倒计时定时器。
- **SPI 主机 (SPI Master)**：全双工 SPI 通信。
- **SPI Flash**：支持直接读写 Flash 存储区、扇区擦除及 SPI Flash Cache 缓存管理。
- **硬件随机数 (RNG)**：提供硬件真实随机数生成器驱动。
- **RTC / RTCCNTL**：支持晶振频率选择（26 MHz / 40 MHz）及 RTC RAM 运行时清零初始化。
- **看门狗 (Watchdog)**：提供硬件和软件看门狗的喂狗与状态管理。
- **EFuse**：读取出厂烧录的 MAC 地址与芯片信息。
- **运行时 (`rt`) 与中断支持**：系统复位入口处理（`ESP8266Reset`）、基于 `xtensa-lx-rt` 的向量表配置，以及 `#[interrupt]` 中断处理宏。

---

## 编译器与环境配置

编译 Xtensa 架构的 Rust 代码需使用乐鑫维护的 [esp-rs/rust](https://github.com/esp-rs/rust) 分支编译器。

### 1. 安装 `esp` Rust 工具链

推荐直接通过 `rustup` 安装：

```shell
$ rustup toolchain install esp
```

或者使用 [esp-rs/rust-build](https://github.com/esp-rs/rust-build) 仓库提供的预编译包与安装脚本，也可以从源码手动编译。详细说明请参阅 *The Rust on ESP Book* 的 [安装 Rust](https://esp-rs.github.io/book/getting-started/installing-rust.html) 章节。

### 2. 配置 Xtensa GCC 链接器 PATH 环境变量

较新的 `esp` 工具链将 Xtensa GCC 二进制工具放置在工具链内部目录中。由于 [`.cargo/config`](.cargo/config) 将链接器配置为 `linker = "xtensa-esp32-elf-gcc"`，必须将该目录加入系统的 `PATH` 路径中：

```shell
$ export PATH="$(echo ~/.rustup/toolchains/esp/xtensa-esp-elf/*/xtensa-esp-elf/bin):$PATH"
```

> 💡 **提示**：可将上述命令添加到您的 `~/.bashrc` 或 `~/.zshrc` 中，方便在所有终端会话中自动生效。

### 3. 安装 `espflash` 烧录工具

安装串口烧录工具：

```shell
$ cargo install espflash --version 2.1
```

*(如工作流支持，亦可使用最新版本的 `espflash`)*。

---

## 编译与烧录运行

### 编译示例程序

编译单个示例（以 `blinky` 为例）：

```shell
$ cargo build --release --example blinky
```

或者一次性编译所有示例：

```shell
$ cargo build --release --examples
```

### 烧录固件到开发板

使用 Micro-USB 数据线连接 ESP8266 开发板（如 NodeMCU、ESP-12 或 Wemos D1 Mini），执行以下烧录命令：

```shell
$ espflash flash -p /dev/ttyUSB0 target/xtensa-esp8266-none-elf/release/examples/blinky
```

`espflash` 会在建立串口连接后自动识别芯片类型，无需手动传递 `--chip` 参数。

### 串口监视与波特率说明

查看设备串口日志：

```shell
$ espflash monitor -p /dev/ttyUSB0 -b 115200
```

*(也可以使用 `picocom`、`miniterm` 或 `screen` 等串口调试工具)*。

> ℹ️ **波特率说明**：
> - **74880 波特率**：ESP8266 上电时，ROM Bootloader 会以 74880 波特率打印第一行启动标语（例如 `ets Jan 8 2013,rst cause:2, boot mode:(1,6)`）。如果在串口中不断循环打印该标语，说明程序触发了异常重启或死循环。
> - **115200 波特率**：本仓库中的应用程序示例均将 UART0 初始化为标准的 115200 波特率进行通信。

---

## 示例程序索引

| 示例 | 说明 |
| :--- | :--- |
| [`blinky`](examples/blinky.rs) | 使用 FRC1 定时器延时控制 GPIO2 引脚（大多数开发板的板载蓝灯）每 500 ms 翻转一次。 |
| [`serial`](examples/serial.rs) | 将 GPIO1 (TX) 和 GPIO3 (RX) 初始化为 UART0，打印欢迎标语并将接收到的字节原样回显。 |
| [`uart_interrupt`](examples/uart_interrupt.rs) | 演示中断驱动的串口收发，结合回调函数与 `#[interrupt(uart)]` 宏，在接收到数据时翻转 LED。 |
| [`flash`](examples/flash.rs) | 读取、递增、擦除扇区并写入 8 字节的持久化启动计数器到 SPI Flash 的 `0x7E000` 偏移处。 |
| [`diag`](examples/diag.rs) | 分步诊断程序，在每一步（UART 初始化、GPIO2 置高、定时器配置、FRC1 寄存器校验、延时测试、LED 翻转）通过 UART0 打印诊断信息。 |

---

## 针对现代工具链的适配说明

本项目集成了针对近期 Xtensa Rust nightly 编译器的适配改动（已在 `rustc` 1.93 nightly 环境验证测试）：

1. **`xtensa-lx-rt` naked asm 修复**：
   - 依赖版本锁定为 `0.16`，并通过 `Cargo.toml` 中的 `[patch.crates-io]` 重定向到本地修补版本 [`vendor/xtensa-lx-rt`](vendor/xtensa-lx-rt)。
   - 较新版本的 `rustc` 禁止在 `#[naked]` 函数内部使用普通 `asm!` 宏，直接报出硬编译错误（[E0787](https://doc.rust-lang.org/error_codes/E0787.html)）。
   - 修补版本将 ESP8266 的异常/中断入口改用单个 `global_asm!` 代码块统一编写（保证汇编中 `.macro` 与 `.set` 的可见性），采用稳定的 `#[unsafe(naked)]` 属性，并为 `call0` 目标添加 4 字节字对齐。
2. **链接器驱动兼容性配置**：
   - 官方 target spec 默认寻找 `xtensa-lx106-elf-gcc`，但乐鑫近期发布的工具链包已不再单独打包 LX106 封装。
   - [`.cargo/config`](.cargo/config) 将链接器设置为 `linker = "xtensa-esp32-elf-gcc"`。因为机器指令汇编均已在 rustc/LLVM 阶段完成，小端的 esp32 gcc 驱动可以完全胜任 LX106 目标文件的链接工作。
3. **PAC 外设库依赖版本调整**：
   - `Cargo.toml` 改为依赖 crates.io 发布的 `esp8266 = "0.6"`，停用本地 `./esp8266`（v0.7）。本地版本使用较新版本的 svd2rust 生成，将 `RegisterBlock` 的结构体字段设为私有，这与此 HAL 中直接读写寄存器字段的实现冲突。

---

## 备选安装配置方式

### 使用官方安装脚本

#### Linux / macOS

```shell
$ curl -LO https://raw.githubusercontent.com/esp-rs/rust-build/main/install-rust-toolchain.sh
$ chmod +x install-rust-toolchain.sh
$ ./install-rust-toolchain.sh
```

#### Windows (PowerShell)

```powershell
PS> Invoke-WebRequest https://raw.githubusercontent.com/esp-rs/rust-build/main/Install-RustToolchain.ps1 -OutFile Install-RustToolchain.ps1
PS> .\Install-RustToolchain.ps1
```

### Docker / Podman 容器构建

亦可在官方预装了 Rust 和 ESP 工具链的容器中构建：

```shell
$ podman run --device /dev/ttyUSB0 -it docker.io/espressif/idf-rust
```

进入容器后，安装 ESP8266 所需工具链：

```shell
# apt update && apt install gcc-xtensa-lx106 binutils-xtensa-lx106
```

---

## 开源协议

本项目采用双重开源协议授权：

- Apache 许可证 2.0 版本 ([LICENSE-APACHE](LICENSE-APACHE) 或 http://www.apache.org/licenses/LICENSE-2.0)
- MIT 许可证 ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

任选其一。

### 参与贡献

除非另有明确声明，否则由您故意提交并包含在本项目中的任何贡献（根据 Apache-2.0 许可证定义），均按上述双重许可授权，不附加任何其他条款或条件。
